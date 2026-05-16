pub mod config;
pub mod crypto;
pub mod error;
pub mod interface;
pub mod portal;
pub mod session;

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use config::{AppConfig, NetworkBinding, ResolvedTarget, write_network_nas_ip};
use error::{PortalError, Result};
use interface::interface_mac_for_ip;
use portal::{CampusClient, LoginStatus, NetworkStatus};
use session::{Session, choose_logout_mac, normalize_mac};
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, error, info, warn};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    PartialFailure,
}

pub async fn run(config: AppConfig, config_path: Option<PathBuf>) -> Result<RunStatus> {
    let targets = config.resolved_targets()?;
    let target_groups = group_targets_by_username(targets);
    let config_updater = Arc::new(ConfigUpdateWriter::new(&config, config_path.clone()));
    let config = Arc::new(config);
    let config_path = Arc::new(config_path);
    let mut tasks = JoinSet::new();

    for targets in target_groups {
        let config = config.clone();
        let config_path = config_path.clone();
        let config_updater = config_updater.clone();
        tasks.spawn(
            async move { run_target_group(config, config_path, config_updater, targets).await },
        );
    }

    let mut failed = false;

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(group_failed) => failed |= group_failed,
            Err(err) => {
                failed = true;
                error!(error = %err, "目标任务组执行失败");
            }
        }
    }
    config_updater.wait().await;

    if failed {
        Ok(RunStatus::PartialFailure)
    } else {
        Ok(RunStatus::Success)
    }
}

pub async fn run_default_login(config: AppConfig, config_path: Option<PathBuf>) -> Result<()> {
    let target = config.default_target()?;
    let config_updater = ConfigUpdateWriter::new(&config, config_path.clone());
    let result = run_target(&config, config_path.as_deref(), &config_updater, &target).await;
    config_updater.wait().await;
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSession {
    pub name: String,
    pub mac: String,
    pub api_mac: String,
    pub device_type: String,
    pub user_ip: String,
    pub start_time: String,
    pub unique_id: String,
}

pub async fn list_default_sessions(
    config: AppConfig,
    config_path: Option<PathBuf>,
) -> Result<Vec<NamedSession>> {
    let config_updater = ConfigUpdateWriter::new(&config, config_path.clone());
    let result = async {
        let (client, token) =
            default_session_client(&config, config_path.as_deref(), &config_updater).await?;
        let sessions = client.list_sessions(&token).await?;
        Ok(sessions
            .iter()
            .map(|session| named_session(&config, session))
            .collect())
    }
    .await;
    config_updater.wait().await;
    result
}

pub async fn logout_default_session(
    config: AppConfig,
    selector: Option<&str>,
    config_path: Option<PathBuf>,
) -> Result<NamedSession> {
    let config_updater = ConfigUpdateWriter::new(&config, config_path.clone());
    let result = async {
        let (client, token) =
            default_session_client(&config, config_path.as_deref(), &config_updater).await?;
        let sessions = client.list_sessions(&token).await?;
        let session = select_logout_session(&config, &sessions, selector)?;
        let logged_out = named_session(&config, session);
        client
            .logout_session(&token, &session.unique_id, &session.api_mac)
            .await?;
        Ok(logged_out)
    }
    .await;
    config_updater.wait().await;
    result
}

async fn default_session_client(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
) -> Result<(CampusClient, String)> {
    let target = config.default_target()?;
    let client = CampusClient::new(config.network.clone(), NetworkBinding::default())?;
    let token =
        login_for_session_token(config, config_path, config_updater, &target, &client).await?;
    Ok((client, token))
}

async fn login_for_session_token(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    target: &ResolvedTarget,
    client: &CampusClient,
) -> Result<String> {
    let redirect_url = match client.check_network().await? {
        NetworkStatus::Online => session_login_redirect_url(config)?,
        NetworkStatus::Redirected(redirect_url) => {
            config_updater.update_nas_ip_from_redirect(config_path, &redirect_url);
            redirect_url
        }
    };

    match client.login(&target.account, &redirect_url).await? {
        LoginStatus::Success {
            token: Some(token), ..
        }
        | LoginStatus::Overloaded {
            token: Some(token), ..
        } => Ok(token),
        LoginStatus::Success { token: None, .. } | LoginStatus::Overloaded { token: None, .. } => {
            Err(PortalError::MissingToken)
        }
        LoginStatus::Failed {
            code,
            error,
            description,
        } => Err(PortalError::LoginRejected {
            code,
            error,
            description,
        }),
    }
}

fn select_logout_session<'a>(
    config: &AppConfig,
    sessions: &'a [Session],
    selector: Option<&str>,
) -> Result<&'a Session> {
    if let Some(selector) = selector {
        return select_session_by_selector(config, sessions, selector);
    }

    if let Some(current_mac) = &config.logout.current_mac {
        let mac = normalize_mac(current_mac)?;
        return sessions
            .iter()
            .find(|session| session.mac == mac)
            .ok_or_else(|| PortalError::SessionNotFound(current_mac.clone()));
    }

    if sessions.len() == 1 {
        return Ok(&sessions[0]);
    }

    if let Some(local_ip) = local_ip_for_gateway(config)? {
        return sessions
            .iter()
            .find(|session| session.user_ip == local_ip.to_string())
            .ok_or_else(|| PortalError::CurrentSessionNotFound(local_ip.to_string()));
    }

    Err(PortalError::NoLogoutCandidate)
}

fn select_session_by_selector<'a>(
    config: &AppConfig,
    sessions: &'a [Session],
    selector: &str,
) -> Result<&'a Session> {
    if let Ok(mac) = normalize_mac(selector) {
        return sessions
            .iter()
            .find(|session| session.mac == mac)
            .ok_or_else(|| PortalError::SessionNotFound(selector.to_string()));
    }

    let known_macs = config
        .logout
        .known_macs
        .iter()
        .filter(|known| known.name.as_deref() == Some(selector))
        .filter_map(|known| normalize_mac(&known.mac).ok())
        .collect::<Vec<_>>();

    let [mac] = known_macs.as_slice() else {
        return if known_macs.is_empty() {
            Err(PortalError::SessionNotFound(selector.to_string()))
        } else {
            Err(PortalError::AmbiguousSessionName(selector.to_string()))
        };
    };

    sessions
        .iter()
        .find(|session| &session.mac == mac)
        .ok_or_else(|| PortalError::SessionNotFound(selector.to_string()))
}

fn named_session(config: &AppConfig, session: &Session) -> NamedSession {
    NamedSession {
        name: known_mac_name(config, &session.mac)
            .unwrap_or("未知")
            .to_string(),
        mac: session.mac.clone(),
        api_mac: session.api_mac.clone(),
        device_type: session.device_type.clone(),
        user_ip: session.user_ip.clone(),
        start_time: session.start_time.clone(),
        unique_id: session.unique_id.clone(),
    }
}

fn known_mac_name<'a>(config: &'a AppConfig, mac: &str) -> Option<&'a str> {
    let normalized = normalize_mac(mac).ok()?;
    config.logout.known_macs.iter().find_map(|known| {
        let known_mac = normalize_mac(&known.mac).ok()?;
        (known_mac == normalized)
            .then_some(known.name.as_deref())
            .flatten()
    })
}

fn session_login_redirect_url(config: &AppConfig) -> Result<String> {
    let template = &config.network.session_login_redirect_url;
    let needs_local_ip = template.contains("{local_ip}");
    let needs_local_mac = template.contains("{local_mac}");
    let needs_nas_ip = template.contains("{nas_ip}");
    if needs_local_ip || needs_local_mac {
        let local_ip = local_ip_for_gateway(config)?.ok_or_else(|| {
            PortalError::InvalidConfig(
                "network.session_login_redirect_url 使用了 {local_ip}，但无法检测到通往网关的本机 IP"
                    .to_string(),
            )
        })?;
        let mut value = template.replace("{local_ip}", &local_ip.to_string());
        if needs_local_mac {
            let local_mac = local_mac_for_gateway(config, local_ip)?.ok_or_else(|| {
                PortalError::InvalidConfig(
                    "network.session_login_redirect_url 使用了 {local_mac}，但无法检测到通往网关的本机 MAC"
                        .to_string(),
                )
            })?;
            value = value.replace("{local_mac}", &portal_mac(&local_mac)?);
        }
        if needs_nas_ip {
            value = value.replace("{nas_ip}", &nas_ip(config)?);
        }
        return Ok(value);
    }

    if needs_nas_ip {
        return Ok(template.replace("{nas_ip}", &nas_ip(config)?));
    }

    if template.contains("userip=") && template.contains("usermac=") && template.contains("nasip=")
    {
        return Ok(template.clone());
    }

    let Some(local_ip) = local_ip_for_gateway(config)? else {
        return Ok(template.clone());
    };
    let separator = if template.contains('?') { '&' } else { '?' };
    let mut value = format!("{template}{separator}userip={local_ip}&");
    if !template.contains("usermac=")
        && let Some(local_mac) = local_mac_for_gateway(config, local_ip)?
    {
        value.push_str(&format!("usermac={}&", portal_mac(&local_mac)?));
    }
    if !template.contains("nasip=") {
        value.push_str(&format!("nasip={}&", nas_ip(config)?));
    }
    Ok(value)
}

fn nas_ip(config: &AppConfig) -> Result<String> {
    config.network.nas_ip.clone().ok_or_else(|| {
        PortalError::InvalidConfig(
            "已在线时执行 list/logout 需要 network.nas_ip；请先在未登录时运行一次 login 自动捕获，或从重定向 Location 头中手动填写，例如 nasip=10.6.33.10"
                .to_string(),
        )
    })
}

pub fn nas_ip_from_redirect_url(redirect_url: &str) -> Option<String> {
    let url = Url::parse(redirect_url).ok()?;
    url.query_pairs().find_map(|(key, value)| {
        (key.eq_ignore_ascii_case("nasip") && !value.is_empty()).then(|| value.into_owned())
    })
}

struct ConfigUpdateWriter {
    initial_nas_ip: Option<String>,
    config_path: Option<PathBuf>,
    nas_ip_write_started: AtomicBool,
    nas_ip_write: Mutex<Option<JoinHandle<()>>>,
}

impl ConfigUpdateWriter {
    fn new(config: &AppConfig, config_path: Option<PathBuf>) -> Self {
        Self {
            initial_nas_ip: config.network.nas_ip.clone(),
            config_path,
            nas_ip_write_started: AtomicBool::new(false),
            nas_ip_write: Mutex::new(None),
        }
    }

    fn update_nas_ip_from_redirect(&self, config_path: Option<&Path>, redirect_url: &str) {
        let Some(config_path) = config_path else {
            return;
        };
        let Some(stored_config_path) = &self.config_path else {
            return;
        };
        if config_path != stored_config_path {
            return;
        }
        let Some(nas_ip) = nas_ip_from_redirect_url(redirect_url) else {
            return;
        };
        if self.initial_nas_ip.as_deref() == Some(nas_ip.as_str()) {
            return;
        }
        if self
            .nas_ip_write_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let config_path = stored_config_path.clone();
        let handle = tokio::spawn(async move {
            match write_network_nas_ip(&config_path, &nas_ip).await {
                Ok(true) => info!(
                    path = %config_path.display(),
                    nas_ip = %nas_ip,
                    "已从校园网重定向更新 network.nas_ip"
                ),
                Ok(false) => {}
                Err(err) => warn!(
                    path = %config_path.display(),
                    nas_ip = %nas_ip,
                    error = %err,
                    "从校园网重定向更新 network.nas_ip 失败"
                ),
            }
        });
        *self.nas_ip_write.lock().expect("config write handle lock") = Some(handle);
    }

    async fn wait(&self) {
        let handle = self
            .nas_ip_write
            .lock()
            .expect("config write handle lock")
            .take();
        if let Some(handle) = handle
            && let Err(err) = handle.await
        {
            warn!(error = %err, "等待配置写回任务失败");
        }
    }
}

fn local_mac_for_gateway(config: &AppConfig, local_ip: IpAddr) -> Result<Option<String>> {
    if let Some(mac) = &config.logout.current_mac {
        return Ok(Some(mac.clone()));
    }

    for interface in &config.interfaces {
        if let Some(mac) = &interface.mac
            && interface.local_ip()? == Some(local_ip)
        {
            return Ok(Some(mac.clone()));
        }
    }

    interface_mac_for_ip(local_ip)
}

fn portal_mac(mac: &str) -> Result<String> {
    Ok(normalize_mac(mac)?.replace(':', "-"))
}

fn local_ip_for_gateway(config: &AppConfig) -> Result<Option<IpAddr>> {
    let parsed =
        Url::parse(&config.network.gateway_base()).map_err(|source| PortalError::UrlParse {
            url: config.network.gateway.clone(),
            source,
        })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| PortalError::GatewayResolve(config.network.gateway.clone()))?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| PortalError::GatewayResolve(format!("{host}:{port}")))?;

    for addr in addrs {
        let bind_addr = match addr {
            SocketAddr::V4(_) => "0.0.0.0:0",
            SocketAddr::V6(_) => "[::]:0",
        };
        let socket = UdpSocket::bind(bind_addr)?;
        socket.connect(addr)?;
        let local_ip = socket.local_addr()?.ip();
        if !local_ip.is_unspecified() {
            return Ok(Some(local_ip));
        }
    }

    Ok(None)
}

fn group_targets_by_username(targets: Vec<ResolvedTarget>) -> Vec<Vec<ResolvedTarget>> {
    let mut group_indexes = HashMap::<String, usize>::new();
    let mut groups: Vec<Vec<ResolvedTarget>> = Vec::new();

    for target in targets {
        let username = target.account.username.clone();
        let group_index = *group_indexes.entry(username).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[group_index].push(target);
    }

    groups
}

async fn run_target_group(
    config: Arc<AppConfig>,
    config_path: Arc<Option<PathBuf>>,
    config_updater: Arc<ConfigUpdateWriter>,
    targets: Vec<ResolvedTarget>,
) -> bool {
    let mut failed = false;

    for target in targets {
        if let Err(err) =
            run_target(&config, config_path.as_deref(), &config_updater, &target).await
        {
            failed = true;
            error!(target = %target.id, error = %err, "目标执行失败");
        }
    }

    failed
}

async fn run_target(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    target: &ResolvedTarget,
) -> Result<()> {
    let binding = target.network_binding()?;
    let client = CampusClient::new(config.network.clone(), binding.clone())?;

    info!(
        target = %target.id,
        account = %target.account.username,
        interface = target.interface_label().as_deref().unwrap_or("default"),
        bind_device = binding.interface_name.as_deref().unwrap_or("default"),
        local_ip = binding.local_ip.map(|ip| ip.to_string()).as_deref().unwrap_or("default"),
        "正在检查网络"
    );

    match client.check_network().await? {
        NetworkStatus::Online => {
            info!(target = %target.id, "已经在线");
            Ok(())
        }
        NetworkStatus::Redirected(redirect_url) => {
            config_updater.update_nas_ip_from_redirect(config_path, &redirect_url);
            login_with_optional_logout(config, target, &client, &redirect_url).await
        }
    }
}

async fn login_with_optional_logout(
    config: &AppConfig,
    target: &ResolvedTarget,
    client: &CampusClient,
    redirect_url: &str,
) -> Result<()> {
    match client.login(&target.account, redirect_url).await? {
        LoginStatus::Success { token } => {
            let token_preview = token_preview(token.as_deref()).unwrap_or_default();
            info!(target = %target.id, token = %token_preview, "登录成功");
            Ok(())
        }
        LoginStatus::Overloaded { description, token } => {
            warn!(target = %target.id, description = %description, "设备数量达到上限");
            if !config.logout.enabled {
                return Err(PortalError::DeviceLimitReached);
            }
            let token = token.ok_or(PortalError::MissingToken)?;
            logout_one_and_retry(config, target, client, redirect_url, &token).await
        }
        LoginStatus::Failed {
            code,
            error,
            description,
        } => Err(PortalError::LoginRejected {
            code,
            error,
            description,
        }),
    }
}

async fn logout_one_and_retry(
    config: &AppConfig,
    target: &ResolvedTarget,
    client: &CampusClient,
    redirect_url: &str,
    token: &str,
) -> Result<()> {
    let sessions = client.list_sessions(token).await?;
    let session_macs = sessions
        .iter()
        .map(|session| session.mac.as_str())
        .collect::<Vec<_>>();
    let logout_mac = choose_logout_mac(&session_macs, &config.logout.known_macs)
        .ok_or(PortalError::NoLogoutCandidate)?;
    let session = sessions
        .iter()
        .find(|session| session.mac == logout_mac)
        .ok_or(PortalError::NoLogoutCandidate)?;

    info!(target = %target.id, mac = %session.mac, "正在下线已有设备");
    client
        .logout_session(token, &session.unique_id, &session.api_mac)
        .await?;

    match client.login(&target.account, redirect_url).await? {
        LoginStatus::Success { token } => {
            let token_preview = token_preview(token.as_deref()).unwrap_or_default();
            info!(
                target = %target.id,
                token = %token_preview,
                "自动下线后登录成功"
            );
            Ok(())
        }
        LoginStatus::Overloaded { description, .. } => {
            Err(PortalError::StillOverloaded(description))
        }
        LoginStatus::Failed {
            code,
            error,
            description,
        } => Err(PortalError::LoginRejected {
            code,
            error,
            description,
        }),
    }
}

fn token_preview(token: Option<&str>) -> Option<String> {
    token.map(|value| value.chars().take(10).collect())
}

pub(crate) fn log_network_binding(target_id: &str, binding: &NetworkBinding) {
    debug!(
        target = target_id,
        bind_device = binding.interface_name.as_deref().unwrap_or("default"),
        local_ip = binding
            .local_ip
            .map(|ip| ip.to_string())
            .as_deref()
            .unwrap_or("default"),
        "已创建 HTTP 客户端"
    );
}
