//! High-level orchestration for unattended campus portal login.
//!
//! This crate is the reusable core behind the CLI. It resolves TOML
//! configuration into login targets, checks captive-portal state, performs v3
//! encrypted login, lists sessions, and optionally logs out one existing device
//! before retrying. The default behavior remains unattended login; interactive
//! shell-style flows should not be reintroduced here.

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
    time::{SystemTime, UNIX_EPOCH},
};

use config::{AccountConfig, AppConfig, NetworkBinding, ResolvedTarget, write_network_nas_ip};
use error::{PortalError, Result};
use interface::interface_mac_for_ip;
use portal::{CampusClient, LoginStatus, NetworkStatus};
use session::{Session, choose_logout_mac, normalize_mac};
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, error, info, warn};
use url::Url;

/// Overall result of running one or more login targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// Every requested target completed successfully.
    Success,
    /// At least one target failed while other target groups may have succeeded.
    PartialFailure,
}

/// Runs the automatic login flow for every resolved target in the configuration.
///
/// Targets are grouped by username and each account group runs concurrently.
/// Within a group, targets run sequentially so repeated logins for the same
/// account do not race against portal device-limit behavior.
///
/// # Errors
///
/// Returns configuration resolution errors before any target is spawned.
pub async fn run(config: AppConfig, config_path: Option<PathBuf>) -> Result<RunStatus> {
    // 实现说明：按 username 分组是为了避免同一账号的多接口登录互相抢设备名额；
    // 不同账号之间可以并行缩短 unattended 任务耗时。
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

/// Runs only the simple-mode default login target.
///
/// This is used by `--one` and by legacy single-account operation.
///
/// # Errors
///
/// Returns configuration, network, login, session, or automatic-logout errors
/// from the single target flow.
pub async fn run_default_login(config: AppConfig, config_path: Option<PathBuf>) -> Result<()> {
    // 实现说明：这里不走 JoinSet，保持单目标错误直接返回给 CLI。
    let target = config.default_target()?;
    let config_updater = ConfigUpdateWriter::new(&config, config_path.clone());
    let result = run_target(&config, config_path.as_deref(), &config_updater, &target).await;
    config_updater.wait().await;
    result
}

/// Runs the login flow for one configured target ID.
///
/// # Errors
///
/// Returns [`PortalError::InvalidConfig`] if `target_id` is unknown, or target
/// runtime errors from the login flow.
pub async fn run_target_login(
    config: AppConfig,
    config_path: Option<PathBuf>,
    target_id: &str,
) -> Result<()> {
    // 实现说明：先解析全部 target 再查找，确保引用校验和普通 run 保持一致。
    let target = config
        .resolved_targets()?
        .into_iter()
        .find(|target| target.id == target_id)
        .ok_or_else(|| PortalError::InvalidConfig(format!("找不到 target {target_id}")))?;
    let config_updater = ConfigUpdateWriter::new(&config, config_path.clone());
    let result = run_target(&config, config_path.as_deref(), &config_updater, &target).await;
    config_updater.wait().await;
    result
}

/// Runs login for every target belonging to one account ID.
///
/// # Errors
///
/// Returns [`PortalError::InvalidConfig`] if no target references `account_id`,
/// or runtime errors from target execution.
pub async fn run_account_login(
    config: AppConfig,
    config_path: Option<PathBuf>,
    account_id: &str,
) -> Result<RunStatus> {
    // 实现说明：账号内 targets 仍复用 run_target_group 的顺序执行策略。
    let targets = config
        .resolved_targets()?
        .into_iter()
        .filter(|target| target.account.id.as_deref() == Some(account_id))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err(PortalError::InvalidConfig(format!(
            "账号 {account_id} 没有配置可登录的 target"
        )));
    }

    let config_updater = Arc::new(ConfigUpdateWriter::new(&config, config_path.clone()));
    let config = Arc::new(config);
    let config_path = Arc::new(config_path);
    let failed = run_target_group(config, config_path, config_updater.clone(), targets).await;
    config_updater.wait().await;

    if failed {
        Ok(RunStatus::PartialFailure)
    } else {
        Ok(RunStatus::Success)
    }
}

/// Session information enriched with configured device names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSession {
    /// Configured known-MAC name, or `未知` when not configured.
    pub name: String,
    /// Normalized MAC used for display and selection.
    pub mac: String,
    /// API-provided MAC preserved for logout payloads.
    pub api_mac: String,
    /// Portal-reported device type.
    pub device_type: String,
    /// Portal-reported user IP.
    pub user_ip: String,
    /// Portal-reported session start time.
    pub start_time: String,
    /// Accounting unique ID required by the logout API.
    pub unique_id: String,
}

/// Sessions belonging to one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSessions {
    /// Account username.
    pub account: String,
    /// Sessions currently active for the account.
    pub sessions: Vec<NamedSession>,
}

/// Result of logging out one session for an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLogout {
    /// Account username.
    pub account: String,
    /// Session that was logged out.
    pub session: NamedSession,
}

/// Lists sessions for the simple-mode default account.
///
/// # Errors
///
/// Returns configuration, login-token, request, or decryption errors.
pub async fn list_default_sessions(
    config: AppConfig,
    config_path: Option<PathBuf>,
) -> Result<Vec<NamedSession>> {
    // 实现说明：session API token 来自一次登录调用；如果当前在线且已有 nas_ip，则
    // 构造 redirectUrl 获取 token。
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

/// Lists sessions for all configured accounts.
///
/// Accounts are processed concurrently. If at least one account succeeds, failed
/// accounts are logged and omitted from the result; if all fail, the first error
/// is returned.
///
/// # Errors
///
/// Returns configuration resolution errors before spawning tasks, or the first
/// account error when no account could be listed.
pub async fn list_account_sessions(
    config: AppConfig,
    config_path: Option<PathBuf>,
) -> Result<Vec<AccountSessions>> {
    // 实现说明：结果按原 account 顺序重排，避免 JoinSet 完成顺序影响 CLI 输出。
    let account_targets = config.account_targets()?;
    let config_updater = Arc::new(ConfigUpdateWriter::new(&config, config_path.clone()));
    let config = Arc::new(config);
    let config_path = Arc::new(config_path);
    let mut tasks = JoinSet::new();

    for (index, (account, targets)) in account_targets.into_iter().enumerate() {
        let account_name = account.username.clone();
        let config = config.clone();
        let config_path = config_path.clone();
        let config_updater = config_updater.clone();
        tasks.spawn(async move {
            (
                index,
                account_name,
                account_sessions_for_targets(config, config_path, config_updater, account, targets)
                    .await,
            )
        });
    }

    let mut groups = Vec::<Option<AccountSessions>>::new();
    let mut first_error = None;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((index, _, Ok(group))) => {
                if groups.len() <= index {
                    groups.resize_with(index + 1, || None);
                }
                groups[index] = Some(group);
            }
            Ok((_, account, Err(err))) => {
                warn!(account = %account, error = %err, "获取账号设备列表失败");
                first_error.get_or_insert(err);
            }
            Err(err) => {
                first_error.get_or_insert_with(|| PortalError::TaskJoin(err.to_string()));
            }
        }
    }
    config_updater.wait().await;
    let groups = groups.into_iter().flatten().collect::<Vec<_>>();
    if groups.is_empty()
        && let Some(err) = first_error
    {
        return Err(err);
    }

    Ok(groups)
}

/// Lists sessions for one configured account ID.
///
/// # Errors
///
/// Returns [`PortalError::InvalidConfig`] if the account is unknown, or session
/// access errors from the account flow.
pub async fn list_account_sessions_for_account(
    config: AppConfig,
    config_path: Option<PathBuf>,
    account_id: &str,
) -> Result<AccountSessions> {
    // 实现说明：复用 account_sessions_for_targets，让无显式 targets 的账号也能走
    // synthetic default target。
    let (account, targets) = config
        .account_targets()?
        .into_iter()
        .find(|(account, _)| account.id.as_deref() == Some(account_id))
        .ok_or_else(|| PortalError::InvalidConfig(format!("找不到账号 {account_id}")))?;
    let config_updater = Arc::new(ConfigUpdateWriter::new(&config, config_path.clone()));
    let config = Arc::new(config);
    let result = account_sessions_for_targets(
        config.clone(),
        Arc::new(config_path),
        config_updater.clone(),
        account,
        targets,
    )
    .await;
    config_updater.wait().await;
    result
}

/// Logs out one session for the simple-mode default account.
///
/// When `selector` is provided, it may be a MAC address or a name from
/// `logout.known_macs`. Without a selector, the function tries
/// `logout.current_mac`, a single active session, or the local route IP.
///
/// # Errors
///
/// Returns session selection errors when no unique candidate can be found, plus
/// any configuration, login-token, or request errors.
pub async fn logout_default_session(
    config: AppConfig,
    selector: Option<&str>,
    config_path: Option<PathBuf>,
) -> Result<NamedSession> {
    // 实现说明：先把将要下线的 session 转成 NamedSession，再调用 logout；这样上层
    // 可以在成功后展示完整信息。
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

/// Logs out sessions matching a selector across account targets.
///
/// Multi-account logout requires a selector. If the selector maps to a target
/// interface MAC, only that target is queried. Otherwise all accounts are
/// searched and every matching account session is logged out.
///
/// # Errors
///
/// Returns [`PortalError::InvalidConfig`] when selector is missing, session
/// lookup errors when nothing matches, or runtime errors from matching accounts.
pub async fn logout_account_sessions(
    config: AppConfig,
    selector: Option<&str>,
    config_path: Option<PathBuf>,
) -> Result<Vec<AccountLogout>> {
    // 实现说明：先尝试通过 selector 精准定位 target，避免多接口场景中为了下线一个
    // 明确设备而查询所有账号。
    let selector = selector.ok_or_else(|| {
        PortalError::InvalidConfig(
            "多账号/多网卡模式下执行 logout 需要指定 MAC，或 logout.known_macs 中配置的名称"
                .to_string(),
        )
    })?;
    let account_targets = config.account_targets()?;
    let targets = account_targets
        .iter()
        .flat_map(|(_, targets)| targets.iter().cloned())
        .collect::<Vec<_>>();
    if let Some(target) = logout_target_for_selector(&config, &targets, selector)? {
        let config_updater = ConfigUpdateWriter::new(&config, config_path.clone());
        let result = logout_for_single_target(
            &config,
            config_path.as_deref(),
            &config_updater,
            target,
            selector,
        )
        .await;
        config_updater.wait().await;
        return result.map(|session| vec![session]);
    }

    let config_updater = Arc::new(ConfigUpdateWriter::new(&config, config_path.clone()));
    let config = Arc::new(config);
    let config_path = Arc::new(config_path);
    let selector = Arc::new(selector.to_string());
    let mut tasks = JoinSet::new();

    for (account, targets) in account_targets {
        let config = config.clone();
        let config_path = config_path.clone();
        let config_updater = config_updater.clone();
        let selector = selector.clone();
        tasks.spawn(async move {
            logout_for_target_account(
                config,
                config_path,
                config_updater,
                account,
                targets,
                &selector,
            )
            .await
        });
    }

    let mut logged_out = Vec::new();
    let mut first_error = None;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(Some(result))) => logged_out.push(result),
            Ok(Ok(None)) => {}
            Ok(Err(err)) => {
                first_error.get_or_insert(err);
            }
            Err(err) => {
                first_error.get_or_insert_with(|| PortalError::TaskJoin(err.to_string()));
            }
        }
    }
    config_updater.wait().await;
    if let Some(err) = first_error {
        return Err(err);
    }

    if logged_out.is_empty() {
        return Err(PortalError::SessionNotFound(selector.as_ref().clone()));
    }

    Ok(logged_out)
}

/// Finds a target whose configured interface MAC matches a logout selector.
///
/// # Errors
///
/// Returns [`PortalError::AmbiguousSessionName`] if the selector name maps to
/// multiple known MAC entries.
fn logout_target_for_selector(
    config: &AppConfig,
    targets: &[ResolvedTarget],
    selector: &str,
) -> Result<Option<ResolvedTarget>> {
    // 实现说明：只匹配 interface.mac，因为这是唯一能在查询 session API 前定位
    // 具体 target 的本地信息。
    let Some(selector_mac) = selector_mac(config, selector)? else {
        return Ok(None);
    };

    Ok(targets
        .iter()
        .find(|target| {
            target
                .interface
                .as_ref()
                .and_then(|interface| interface.mac.as_deref())
                .and_then(|mac| normalize_mac(mac).ok())
                .as_deref()
                == Some(selector_mac.as_str())
        })
        .cloned())
}

/// Resolves a user selector to a normalized MAC if possible.
///
/// The selector may be a MAC address or a name from `logout.known_macs`.
///
/// # Errors
///
/// Returns [`PortalError::AmbiguousSessionName`] if the name is configured more
/// than once.
fn selector_mac(config: &AppConfig, selector: &str) -> Result<Option<String>> {
    // 实现说明：直接 MAC 优先；名称查找忽略格式无效的 known_macs，让配置里的其它
    // 名称不影响当前 selector。
    if let Ok(mac) = normalize_mac(selector) {
        return Ok(Some(mac));
    }

    let macs = config
        .logout
        .known_macs
        .iter()
        .filter(|known| known.name.as_deref() == Some(selector))
        .filter_map(|known| normalize_mac(&known.mac).ok())
        .collect::<Vec<_>>();

    match macs.as_slice() {
        [] => Ok(None),
        [mac] => Ok(Some(mac.clone())),
        _ => Err(PortalError::AmbiguousSessionName(selector.to_string())),
    }
}

/// Builds a session-capable client for the default account.
///
/// # Errors
///
/// Returns default target resolution or token acquisition errors.
async fn default_session_client(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
) -> Result<(CampusClient, String)> {
    // 实现说明：默认模式没有 target ID 参数，统一转成 synthetic default target。
    let target = config.default_target()?;
    session_client_for_target(config, config_path, config_updater, &target).await
}

/// Builds a bound client and obtains a session API token for a target.
///
/// # Errors
///
/// Returns network binding, client construction, probe, login, or token errors.
async fn session_client_for_target(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    target: &ResolvedTarget,
) -> Result<(CampusClient, String)> {
    // 实现说明：session API 需要 token；token 不是独立接口获取，而是通过一次 v3
    // login 响应获得。
    let binding = target.network_binding()?;
    debug!(
        target = %target.id,
        account = %target.account.username,
        interface = target.interface_label().as_deref().unwrap_or("default"),
        bind_device = binding.interface_name.as_deref().unwrap_or("default"),
        local_ip = binding.local_ip.map(|ip| ip.to_string()).as_deref().unwrap_or("default"),
        "正在获取会话 token"
    );
    let client = CampusClient::new(config.network.clone(), binding)?;
    let token =
        login_for_session_token(config, config_path, config_updater, target, &client).await?;
    Ok((client, token))
}

/// Performs the login exchange needed to obtain a session API token.
///
/// # Errors
///
/// Returns [`PortalError::MissingToken`] if a success/overloaded response omits
/// the token, or login/probe errors for other failures.
async fn login_for_session_token(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    target: &ResolvedTarget,
    client: &CampusClient,
) -> Result<String> {
    // 实现说明：如果配置了接口且已有 nas_ip，可直接构造真实 redirectUrl；否则先
    // probe 网络，以便从 captive redirect 捕获 nasip 并写回配置。
    let redirect_url = match (&target.interface, config.network.nas_ip.as_ref()) {
        (Some(_), Some(_)) => session_login_redirect_url(config, target)?,
        _ => match client.check_network().await? {
            NetworkStatus::Online => session_login_redirect_url(config, target)?,
            NetworkStatus::Redirected(redirect_url) => {
                config_updater.update_nas_ip_from_redirect(config_path, &redirect_url);
                redirect_url
            }
        },
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

/// Lists sessions for an account using any viable target for that account.
///
/// # Errors
///
/// Returns an error if no target can produce a session token or the session API
/// request fails.
async fn account_sessions_for_targets(
    config: Arc<AppConfig>,
    config_path: Arc<Option<PathBuf>>,
    config_updater: Arc<ConfigUpdateWriter>,
    account: AccountConfig,
    targets: Vec<ResolvedTarget>,
) -> Result<AccountSessions> {
    // 实现说明：一个账号的任意 target 都能访问该账号的 session 列表；逐个 target
    // 尝试提高多网卡配置下的容错性。
    let account_name = account.username.clone();
    let targets = account_session_targets(account, targets);
    let (client, token) =
        session_client_for_any_target(&config, config_path.as_deref(), &config_updater, &targets)
            .await?;
    let sessions = client.list_sessions(&token).await?;

    Ok(AccountSessions {
        account: account_name,
        sessions: sessions
            .iter()
            .map(|session| named_session(&config, session))
            .collect(),
    })
}

/// Returns explicit account targets or creates a synthetic default target.
fn account_session_targets(
    account: AccountConfig,
    targets: Vec<ResolvedTarget>,
) -> Vec<ResolvedTarget> {
    // 实现说明：允许只有 [[accounts]] 而没有 [[targets]] 的账号执行 list/logout，
    // 此时使用默认路由访问 session API。
    if !targets.is_empty() {
        return targets;
    }

    let target_id = account
        .id
        .as_deref()
        .map(|id| format!("{id}-default"))
        .unwrap_or_else(|| "default".to_string());

    vec![ResolvedTarget {
        id: target_id,
        account,
        interface: None,
    }]
}

/// Searches one account's sessions and logs out a matching selector when found.
///
/// Returns `Ok(None)` when the selector does not match this account, allowing
/// callers to continue searching other accounts.
///
/// # Errors
///
/// Returns token, list, ambiguous selector, or logout errors.
async fn logout_for_target_account(
    config: Arc<AppConfig>,
    config_path: Arc<Option<PathBuf>>,
    config_updater: Arc<ConfigUpdateWriter>,
    account: AccountConfig,
    targets: Vec<ResolvedTarget>,
    selector: &str,
) -> Result<Option<AccountLogout>> {
    // 实现说明：账号内 selector 找不到时返回 Ok(None)，让跨账号 logout 可以继续
    // 搜索其它账号；真正的协议/配置错误仍向上传播。
    let account_name = account.username.clone();
    let targets = account_session_targets(account, targets);
    let (client, token) =
        session_client_for_any_target(&config, config_path.as_deref(), &config_updater, &targets)
            .await?;
    let sessions = client.list_sessions(&token).await?;
    let session = match select_session_by_selector(&config, &sessions, selector) {
        Ok(session) => session,
        Err(PortalError::SessionNotFound(_)) => return Ok(None),
        Err(err) => return Err(err),
    };
    let logged_out = named_session(&config, session);
    client
        .logout_session(&token, &session.unique_id, &session.api_mac)
        .await?;

    Ok(Some(AccountLogout {
        account: account_name,
        session: logged_out,
    }))
}

/// Logs out a matching session through one already selected target.
///
/// # Errors
///
/// Returns target token, session selection, or logout request errors.
async fn logout_for_single_target(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    target: ResolvedTarget,
    selector: &str,
) -> Result<AccountLogout> {
    // 实现说明：用于 selector 已经匹配到 interface.mac 的快速路径，避免跨账号扫描。
    let account = target.account.username.clone();
    let (client, token) =
        session_client_for_target(config, config_path, config_updater, &target).await?;
    let sessions = client.list_sessions(&token).await?;
    let session = select_session_by_selector(config, &sessions, selector)?;
    let logged_out = named_session(config, session);
    client
        .logout_session(&token, &session.unique_id, &session.api_mac)
        .await?;

    Ok(AccountLogout {
        account,
        session: logged_out,
    })
}

/// Obtains a session client from the first target that works.
///
/// # Errors
///
/// Returns the last target error, or invalid configuration if no target exists.
async fn session_client_for_any_target(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    targets: &[ResolvedTarget],
) -> Result<(CampusClient, String)> {
    // 实现说明：同账号多 target 可能只有部分网卡当前可达；逐个尝试比直接失败更适合
    // unattended 运行。
    let mut last_error = None;

    for target in targets {
        match session_client_for_target(config, config_path, config_updater, target).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                warn!(
                    target = %target.id,
                    account = %target.account.username,
                    error = %err,
                    "获取账号会话 token 失败，尝试下一个 target"
                );
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        PortalError::InvalidConfig("账号没有配置可用于访问 session API 的 target".to_string())
    }))
}

/// Selects the session to log out in simple-mode logout.
///
/// A selector wins first. Without selector, the function tries
/// `logout.current_mac`, a single-session shortcut, then local route IP.
///
/// # Errors
///
/// Returns session lookup errors if no unambiguous candidate exists.
fn select_logout_session<'a>(
    config: &AppConfig,
    sessions: &'a [Session],
    selector: Option<&str>,
) -> Result<&'a Session> {
    // 实现说明：无 selector 时尽量避免误下线：current_mac 精确匹配优先，只有一个
    // session 可安全选择，否则尝试用本机到 gateway 的源 IP 对应当前设备。
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

/// Selects a session by MAC address or configured known-MAC name.
///
/// # Errors
///
/// Returns [`PortalError::SessionNotFound`] if no session matches, or
/// [`PortalError::AmbiguousSessionName`] if a name maps to multiple known MACs.
fn select_session_by_selector<'a>(
    config: &AppConfig,
    sessions: &'a [Session],
    selector: &str,
) -> Result<&'a Session> {
    // 实现说明：直接 MAC 不经过 known_macs；名称 selector 必须唯一映射到一个 MAC。
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

/// Adds configured display metadata to a normalized session.
fn named_session(config: &AppConfig, session: &Session) -> NamedSession {
    // 实现说明：展示名称来自 known_macs；未命名设备统一显示“未知”，但仍保留所有
    // 原始会话字段。
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

/// Finds a known-MAC display name for a normalized MAC.
fn known_mac_name<'a>(config: &'a AppConfig, mac: &str) -> Option<&'a str> {
    // 实现说明：配置 MAC 和会话 MAC 都 normalize 后比较，容忍用户混用横线/冒号和
    // 大小写。
    let normalized = normalize_mac(mac).ok()?;
    config.logout.known_macs.iter().find_map(|known| {
        let known_mac = normalize_mac(&known.mac).ok()?;
        (known_mac == normalized)
            .then_some(known.name.as_deref())
            .flatten()
    })
}

/// Builds the redirect URL used to obtain a session API token.
///
/// The template may include `{local_ip}`, `{local_mac}`, and `{nas_ip}`. Missing
/// required query parameters are appended after substitution.
///
/// # Errors
///
/// Returns configuration, MAC, NAS IP, or URL parse errors.
fn session_login_redirect_url(config: &AppConfig, target: &ResolvedTarget) -> Result<String> {
    // 实现说明：有接口绑定时使用真实接口身份；默认模式使用 fallback MAC 和 0.0.0.0
    // 以兼容门户 token 获取。
    let template = &config.network.session_login_redirect_url;
    let (user_ip, user_mac) = redirect_user_identity(config, target)?;
    let nas_ip = nas_ip(config)?;
    let value = template
        .replace("{local_ip}", &user_ip)
        .replace("{local_mac}", &user_mac)
        .replace("{nas_ip}", &nas_ip);

    ensure_redirect_query(value, &user_ip, &user_mac, &nas_ip)
}

/// Resolves the user IP and MAC to embed in the session redirect URL.
///
/// # Errors
///
/// Returns invalid configuration when a bound target lacks enough interface
/// identity to construct a real redirect URL.
fn redirect_user_identity(config: &AppConfig, target: &ResolvedTarget) -> Result<(String, String)> {
    // 实现说明：接口 target 必须提供/可发现真实 local_ip 和 MAC；无接口 target 则走
    // fallback 身份，避免简单模式强依赖本机接口探测。
    let Some(interface) = &target.interface else {
        return Ok(("0.0.0.0".to_string(), redirect_user_mac(config)?));
    };

    let local_ip = interface.local_ip()?.ok_or_else(|| {
        PortalError::InvalidConfig(format!(
            "target {} 的 interface {} 缺少 local_ip 或 name，无法构造真实 redirectUrl",
            target.id, interface.id
        ))
    })?;
    let mac = if let Some(mac) = &interface.mac {
        portal_mac(mac)?
    } else if let Some(mac) = interface_mac_for_ip(local_ip)? {
        portal_mac(&mac)?
    } else {
        return Err(PortalError::InvalidConfig(format!(
            "target {} 的 interface {} 缺少 mac，且无法从本机网卡查询到 MAC",
            target.id, interface.id
        )));
    };

    Ok((local_ip.to_string(), mac))
}

/// Chooses a MAC address for default-mode redirect URL construction.
///
/// # Errors
///
/// Returns [`PortalError::InvalidMac`] if the chosen configured MAC is invalid.
fn redirect_user_mac(config: &AppConfig) -> Result<String> {
    // 实现说明：优先 current_mac，其次 known_macs，再其次 interface.mac；都没有时
    // 生成一个本地管理地址用于 token 获取。
    if let Some(mac) = &config.logout.current_mac {
        return portal_mac(mac);
    }

    if let Some(mac) = config
        .logout
        .known_macs
        .iter()
        .find_map(|known| portal_mac(&known.mac).ok())
    {
        return Ok(mac);
    }

    if let Some(mac) = config
        .interfaces
        .iter()
        .filter_map(|interface| interface.mac.as_ref())
        .find_map(|mac| portal_mac(mac).ok())
    {
        return Ok(mac);
    }

    Ok(generated_portal_mac())
}

/// Ensures a redirect URL contains the query parameters required by the portal.
///
/// # Errors
///
/// Returns [`PortalError::UrlParse`] if the substituted URL is invalid.
fn ensure_redirect_query(
    value: String,
    user_ip: &str,
    user_mac: &str,
    nas_ip: &str,
) -> Result<String> {
    // 实现说明：模板可由用户覆盖；这里只补缺失参数，不覆盖用户已经写好的 query。
    let mut url = Url::parse(&value).map_err(|source| PortalError::UrlParse {
        url: value.clone(),
        source,
    })?;
    let has_user_ip = url.query_pairs().any(|(key, _)| key == "userip");
    let has_user_mac = url.query_pairs().any(|(key, _)| key == "usermac");
    let has_nas_ip = url.query_pairs().any(|(key, _)| key == "nasip");

    if has_user_ip && has_user_mac && has_nas_ip {
        return Ok(value);
    }

    {
        let mut query = url.query_pairs_mut();
        if !has_user_ip {
            query.append_pair("userip", user_ip);
        }
        if !has_user_mac {
            query.append_pair("usermac", user_mac);
        }
        if !has_nas_ip {
            query.append_pair("nasip", nas_ip);
        }
    }

    Ok(url.to_string())
}

/// Generates a locally administered MAC-like value for redirect token requests.
fn generated_portal_mac() -> String {
    // 实现说明：门户 token 获取只需要一个格式正确的 usermac；没有配置 MAC 时用时间种子
    // 生成 02 开头的本地管理地址，避免固定值导致多实例混淆。
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9000_0000);
    let bytes = [
        0x02,
        (seed >> 32) as u8,
        (seed >> 24) as u8,
        (seed >> 16) as u8,
        (seed >> 8) as u8,
        seed as u8,
    ];

    bytes
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("-")
}

/// Returns the configured NAS IP required for online session operations.
///
/// # Errors
///
/// Returns [`PortalError::InvalidConfig`] when `network.nas_ip` is missing.
fn nas_ip(config: &AppConfig) -> Result<String> {
    // 实现说明：已在线时无法再通过 captive redirect 发现 nasip，因此 list/logout 需要
    // 用户先运行一次未登录 login 捕获，或手动配置。
    config.network.nas_ip.clone().ok_or_else(|| {
        PortalError::InvalidConfig(
            "已在线时执行 list/logout 需要 network.nas_ip；请先在未登录时运行一次 login 自动捕获，或从重定向 Location 头中手动填写，例如 nasip=10.6.33.10"
                .to_string(),
        )
    })
}

/// Extracts `nasip` from a portal redirect URL.
///
/// Returns `None` if the URL is invalid, lacks `nasip`, or has an empty value.
pub fn nas_ip_from_redirect_url(redirect_url: &str) -> Option<String> {
    // 实现说明：query key 按大小写不敏感匹配，兼容网关参数大小写变化。
    let url = Url::parse(redirect_url).ok()?;
    url.query_pairs().find_map(|(key, value)| {
        (key.eq_ignore_ascii_case("nasip") && !value.is_empty()).then(|| value.into_owned())
    })
}

/// Coordinates one asynchronous write-back of a discovered NAS IP.
struct ConfigUpdateWriter {
    /// NAS IP loaded at process start, used to skip no-op writes.
    initial_nas_ip: Option<String>,
    /// Configuration file path eligible for write-back.
    config_path: Option<PathBuf>,
    /// Guard that ensures only one write-back task is spawned.
    nas_ip_write_started: AtomicBool,
    /// Join handle for the optional write-back task.
    nas_ip_write: Mutex<Option<JoinHandle<()>>>,
}

impl ConfigUpdateWriter {
    /// Creates a writer for a configuration snapshot and optional path.
    fn new(config: &AppConfig, config_path: Option<PathBuf>) -> Self {
        // 实现说明：保存 initial_nas_ip 后，即使运行中 config 被 Arc 共享也能判断
        // redirect 捕获值是否真的需要落盘。
        Self {
            initial_nas_ip: config.network.nas_ip.clone(),
            config_path,
            nas_ip_write_started: AtomicBool::new(false),
            nas_ip_write: Mutex::new(None),
        }
    }

    /// Schedules a write-back if a redirect URL contains a new NAS IP.
    fn update_nas_ip_from_redirect(&self, config_path: Option<&Path>, redirect_url: &str) {
        // 实现说明：多 target 可能同时看到 redirect；AtomicBool 保证只启动一个写任务，
        // 避免并发重写同一个 TOML 文件。
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

    /// Waits for the pending write-back task, if one was started.
    async fn wait(&self) {
        // 实现说明：主流程结束前等待配置写回，避免 CLI 退出时 tokio 任务被直接丢弃。
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

/// Converts a MAC address into the dash-separated format used in redirect URLs.
///
/// # Errors
///
/// Returns [`PortalError::InvalidMac`] if `mac` cannot be normalized.
fn portal_mac(mac: &str) -> Result<String> {
    // 实现说明：normalize_mac 产出冒号格式；门户 redirectUrl 观察到的是横线格式。
    Ok(normalize_mac(mac)?.replace(':', "-"))
}

/// Finds the local IP address that would be used to reach the configured gateway.
///
/// # Errors
///
/// Returns URL, DNS resolution, or UDP socket errors.
fn local_ip_for_gateway(config: &AppConfig) -> Result<Option<IpAddr>> {
    // 实现说明：UDP connect 不发包，但会让操作系统选择路由和源地址，适合识别当前
    // 默认出口对应的会话 IP。
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

/// Groups targets by account username while preserving first-seen group order.
fn group_targets_by_username(targets: Vec<ResolvedTarget>) -> Vec<Vec<ResolvedTarget>> {
    // 实现说明：HashMap 只保存 username 到 group index 的映射，实际 group 顺序由 Vec
    // push 保持。
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

/// Runs all targets for one username sequentially.
///
/// Returns `true` if any target failed.
async fn run_target_group(
    config: Arc<AppConfig>,
    config_path: Arc<Option<PathBuf>>,
    config_updater: Arc<ConfigUpdateWriter>,
    targets: Vec<ResolvedTarget>,
) -> bool {
    // 实现说明：组内不短路，尽可能完成同账号的后续 target，并把失败汇总给上层
    // PartialFailure。
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

/// Runs the login flow for one resolved target.
///
/// # Errors
///
/// Returns binding, client, probe, login, or automatic-logout errors.
async fn run_target(
    config: &AppConfig,
    config_path: Option<&Path>,
    config_updater: &ConfigUpdateWriter,
    target: &ResolvedTarget,
) -> Result<()> {
    // 实现说明：先 probe，在线则不重复登录；被 portal redirect 时才执行登录流程，并把
    // redirect 中的新 nasip 交给异步写回器。
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

/// Logs in and optionally logs out one old session before retrying.
///
/// # Errors
///
/// Returns device-limit, missing-token, rejected-login, or logout/retry errors.
async fn login_with_optional_logout(
    config: &AppConfig,
    target: &ResolvedTarget,
    client: &CampusClient,
    redirect_url: &str,
) -> Result<()> {
    // 实现说明：只有 Overloaded 且 logout.enabled 时才进入 session API；成功分支只
    // 打印 token 前缀，避免完整 token 泄漏到日志。
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

/// Logs out one selected session and retries the original login.
///
/// # Errors
///
/// Returns session-list, candidate-selection, logout, or retry-login errors.
async fn logout_one_and_retry(
    config: &AppConfig,
    target: &ResolvedTarget,
    client: &CampusClient,
    redirect_url: &str,
    token: &str,
) -> Result<()> {
    // 实现说明：下线候选策略在 session::choose_logout_mac 中集中维护；这里用返回的
    // normalized MAC 找回完整 Session，以便 logout payload 使用 api_mac。
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

/// Returns a short token prefix for logs.
fn token_preview(token: Option<&str>) -> Option<String> {
    // 实现说明：只取前 10 个 char，既能关联日志事件，又不暴露完整授权 token。
    token.map(|value| value.chars().take(10).collect())
}

/// Logs the network binding used by an HTTP client.
pub(crate) fn log_network_binding(target_id: &str, binding: &NetworkBinding) {
    // 实现说明：portal::CampusClient 创建完成后调用，方便排查多 WAN 绑定是否生效。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AccountConfig, InterfaceConfig, LogoutConfig, NetworkConfig};

    #[test]
    /// Verifies session redirect URL construction for a bound interface target.
    fn session_redirect_url_uses_target_interface_identity() {
        // 实现说明：绑定接口时 userip/usermac 必须来自 target.interface，而不是默认
        // fallback 身份。
        let config = test_config(
            Some("10.6.33.10"),
            LogoutConfig {
                current_mac: Some("11:22:33:44:55:66".to_string()),
                ..LogoutConfig::default()
            },
        );
        let target = ResolvedTarget {
            id: "wan1-main".to_string(),
            account: test_account(),
            interface: Some(InterfaceConfig {
                id: "wan1".to_string(),
                name: None,
                local_ip: Some("10.180.0.10".parse().unwrap()),
                mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
            }),
        };

        let redirect_url = session_login_redirect_url(&config, &target).unwrap();

        assert_eq!(
            redirect_url,
            "http://10.184.6.32:80/wenet/auth?userip=10.180.0.10&usermac=aa-bb-cc-dd-ee-ff&nasip=10.6.33.10"
        );
    }

    #[test]
    /// Verifies default targets use fallback redirect identity.
    fn session_redirect_url_without_interface_uses_fallback_identity() {
        // 实现说明：无接口 target 使用 0.0.0.0 加 current_mac，覆盖简单模式 token
        // 获取路径。
        let config = test_config(
            Some("10.6.33.10"),
            LogoutConfig {
                current_mac: Some("11:22:33:44:55:66".to_string()),
                ..LogoutConfig::default()
            },
        );
        let target = ResolvedTarget {
            id: "default".to_string(),
            account: test_account(),
            interface: None,
        };

        let redirect_url = session_login_redirect_url(&config, &target).unwrap();

        assert_eq!(
            redirect_url,
            "http://10.184.6.32:80/wenet/auth?userip=0.0.0.0&usermac=11-22-33-44-55-66&nasip=10.6.33.10"
        );
    }

    /// Builds a minimal test configuration.
    fn test_config(nas_ip: Option<&str>, logout: LogoutConfig) -> AppConfig {
        // 实现说明：显式填满 NetworkConfig，避免测试被 Default 后续改动意外影响。
        AppConfig {
            network: NetworkConfig {
                gateway: "http://10.184.6.32".to_string(),
                nas_ip: nas_ip.map(str::to_string),
                test_url: "http://1.1.1.1".to_string(),
                login_url: "http://10.184.6.32/portal-conversion/api/v3/portal/connect"
                    .to_string(),
                session_login_redirect_url:
                    "http://10.184.6.32:80/wenet/auth?userip={local_ip}&usermac={local_mac}&nasip={nas_ip}"
                        .to_string(),
                session_list_url: "http://10.184.6.32/portal-conversion/api/v3/session/list"
                    .to_string(),
                session_logout_url:
                    "http://10.184.6.32/portal-conversion/api/v3/session/acctUniqueId"
                        .to_string(),
                timeout_secs: 5,
            },
            default_account: None,
            logout,
            accounts: vec![],
            interfaces: vec![],
            targets: vec![],
        }
    }

    /// Builds a minimal account used by redirect URL tests.
    fn test_account() -> AccountConfig {
        // 实现说明：账户内容不会发起真实登录，只需满足 ResolvedTarget 字段。
        AccountConfig {
            id: Some("main".to_string()),
            username: "u@xjtu".to_string(),
            password: "p".to_string(),
        }
    }
}
