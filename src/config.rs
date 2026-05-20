//! TOML configuration model and target resolution.
//!
//! The CLI supports a single default account as well as advanced multi-account
//! and multi-interface targets. This module keeps TOML deserialization separate
//! from runtime resolution so the rest of the crate can work with validated
//! [`ResolvedTarget`] values. Configuration is TOML-only; do not reintroduce the
//! old YAML shape here.

use std::{
    collections::HashMap,
    fs,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    time::Duration,
};

use serde::Deserialize;

use crate::{
    error::{PortalError, Result},
    interface::interface_ipv4,
};

/// Network binding requested by a resolved target.
///
/// `interface_name` is the primary selector and is passed to
/// `reqwest::ClientBuilder::interface` on supported platforms. `local_ip` is an
/// optional source-address hint and should not be used as a substitute for
/// interface binding on OpenWrt/mwan3 setups.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkBinding {
    /// Interface name to bind the HTTP client to, such as `eth0` or `wan`.
    pub interface_name: Option<String>,
    /// Optional local source address to use in addition to interface binding.
    pub local_ip: Option<IpAddr>,
}

/// Root configuration deserialized from `config.toml`.
///
/// The public configuration shape is documented by `config.example.toml` and
/// `config.advanced.example.toml`. `default_account` drives the simple mode;
/// `accounts`, `interfaces`, and `targets` drive advanced unattended login.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// Network URLs, timeout, and discovered NAS address.
    #[serde(default)]
    pub network: NetworkConfig,
    /// Single-account mode configuration used when no `targets` are present.
    pub default_account: Option<AccountConfig>,
    /// Automatic logout settings and known device names.
    #[serde(default)]
    pub logout: LogoutConfig,
    /// Named accounts used by advanced targets.
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    /// Named local interfaces used by advanced targets.
    #[serde(default)]
    pub interfaces: Vec<InterfaceConfig>,
    /// Login targets that pair an account with an optional interface.
    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

impl AppConfig {
    /// Reads and parses an application configuration file.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::ConfigRead`] if the file cannot be read and
    /// [`PortalError::ConfigParse`] if TOML deserialization fails.
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        // 实现说明：保留路径字符串在错误里，方便 CLI 直接输出具体失败文件。
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| PortalError::ConfigRead {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| PortalError::ConfigParse {
            path: path.display().to_string(),
            source,
        })
    }

    /// Resolves the configured login targets into validated runtime targets.
    ///
    /// When `targets` is empty, this returns a single target built from
    /// `[default_account]` for backward-compatible simple mode.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::InvalidConfig`] if referenced account or interface
    /// IDs do not exist, or if an account is missing required credentials.
    pub fn resolved_targets(&self) -> Result<Vec<ResolvedTarget>> {
        // 实现说明：先建立账号和接口索引，再按 targets 原顺序解析，确保多目标
        // 登录的执行顺序和配置文件一致。
        if self.targets.is_empty() {
            return Ok(vec![self.default_target()?]);
        }

        let accounts = self
            .accounts
            .iter()
            .filter_map(|account| account.id.as_ref().map(|id| (id.as_str(), account)))
            .collect::<HashMap<_, _>>();
        let interfaces = self
            .interfaces
            .iter()
            .map(|interface| (interface.id.as_str(), interface))
            .collect::<HashMap<_, _>>();

        self.targets
            .iter()
            .map(|target| {
                let account = accounts.get(target.account.as_str()).ok_or_else(|| {
                    PortalError::InvalidConfig(format!(
                        "target {} 引用了不存在的账号 {}",
                        target.id, target.account
                    ))
                })?;
                validate_account(account)?;
                let interface = match &target.interface {
                    Some(interface_id) => {
                        let interface = interfaces.get(interface_id.as_str()).ok_or_else(|| {
                            PortalError::InvalidConfig(format!(
                                "target {} 引用了不存在的网络接口 {}",
                                target.id, interface_id
                            ))
                        })?;
                        Some((*interface).clone())
                    }
                    None => None,
                };

                Ok(ResolvedTarget {
                    id: target.id.clone(),
                    account: (*account).clone(),
                    interface,
                })
            })
            .collect()
    }

    /// Groups resolved targets by account while preserving account order.
    ///
    /// Accounts without explicit targets are included with an empty target list
    /// so session/list/logout commands can fall back to a synthetic default
    /// target for that account.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::InvalidConfig`] when a target references an
    /// unknown account or interface, or when an account is missing credentials.
    pub fn account_targets(&self) -> Result<Vec<(AccountConfig, Vec<ResolvedTarget>)>> {
        // 实现说明：targets_by_account 用目标里的 account ID 收集，最后再按
        // accounts 的配置顺序输出，避免 HashMap 遍历顺序影响 CLI 展示。
        let mut accounts_by_id = HashMap::new();
        for account in &self.accounts {
            validate_account(account)?;
            if let Some(id) = &account.id {
                accounts_by_id.insert(id.as_str(), account);
            }
        }

        let interfaces = self
            .interfaces
            .iter()
            .map(|interface| (interface.id.as_str(), interface))
            .collect::<HashMap<_, _>>();
        let mut targets_by_account = HashMap::<String, Vec<ResolvedTarget>>::new();

        for target in &self.targets {
            let account = accounts_by_id.get(target.account.as_str()).ok_or_else(|| {
                PortalError::InvalidConfig(format!(
                    "target {} 引用了不存在的账号 {}",
                    target.id, target.account
                ))
            })?;
            let interface = match &target.interface {
                Some(interface_id) => {
                    let interface = interfaces.get(interface_id.as_str()).ok_or_else(|| {
                        PortalError::InvalidConfig(format!(
                            "target {} 引用了不存在的网络接口 {}",
                            target.id, interface_id
                        ))
                    })?;
                    Some((*interface).clone())
                }
                None => None,
            };

            targets_by_account
                .entry(target.account.clone())
                .or_default()
                .push(ResolvedTarget {
                    id: target.id.clone(),
                    account: (*account).clone(),
                    interface,
                });
        }

        Ok(self
            .accounts
            .iter()
            .map(|account| {
                let targets = account
                    .id
                    .as_ref()
                    .and_then(|id| targets_by_account.remove(id))
                    .unwrap_or_default();
                (account.clone(), targets)
            })
            .collect())
    }

    /// Builds the simple-mode default target from `[default_account]`.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::InvalidConfig`] if `[default_account]` is absent or
    /// contains empty credentials.
    pub fn default_target(&self) -> Result<ResolvedTarget> {
        // 实现说明：默认目标没有接口绑定，运行时使用系统默认路由。
        let account = self.default_account.clone().ok_or_else(|| {
            PortalError::InvalidConfig("缺少必需的 [default_account] 配置".to_string())
        })?;
        validate_account(&account)?;
        Ok(ResolvedTarget {
            id: "default".to_string(),
            account,
            interface: None,
        })
    }
}

/// Writes `network.nas_ip` into a TOML configuration file.
///
/// Existing comments and value decoration are preserved as much as
/// `toml_edit` allows. The function returns `true` only when the file content
/// was changed.
///
/// # Errors
///
/// Returns configuration read/edit/write errors if the file cannot be loaded,
/// parsed as editable TOML, or saved.
pub async fn write_network_nas_ip(path: impl AsRef<Path>, nas_ip: &str) -> Result<bool> {
    // 实现说明：用 toml_edit 而不是反序列化再序列化，避免自动捕获 NAS IP 时抹掉
    // 用户在配置文件里的注释和排版。
    let path = path.as_ref();
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|source| PortalError::ConfigRead {
            path: path.display().to_string(),
            source,
        })?;
    let mut document =
        text.parse::<toml_edit::DocumentMut>()
            .map_err(|source| PortalError::ConfigEdit {
                path: path.display().to_string(),
                source,
            })?;

    if !document.as_table().contains_key("network") {
        document["network"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    let network = document["network"]
        .as_table_mut()
        .ok_or_else(|| PortalError::InvalidConfig("[network] 必须是 TOML 表".to_string()))?;
    if network.get("nas_ip").and_then(toml_edit::Item::as_str) == Some(nas_ip) {
        return Ok(false);
    }

    if let Some(item) = network.get_mut("nas_ip") {
        if let Some(value) = item.as_value_mut() {
            let decor = value.decor().clone();
            let mut new_value = toml_edit::Value::from(nas_ip);
            *new_value.decor_mut() = decor;
            *value = new_value;
        } else {
            *item = toml_edit::value(nas_ip);
        }
    } else {
        network.insert("nas_ip", toml_edit::value(nas_ip));
    }
    tokio::fs::write(path, document.to_string())
        .await
        .map_err(|source| PortalError::ConfigWrite {
            path: path.display().to_string(),
            source,
        })?;
    Ok(true)
}

/// Validates that an account has usable credentials.
///
/// # Errors
///
/// Returns [`PortalError::InvalidConfig`] if the username is blank or the
/// password is empty.
fn validate_account(account: &AccountConfig) -> Result<()> {
    // 实现说明：用户名允许保留内部和尾部非空白字符，但全空白无意义；密码不 trim，
    // 因为门户密码理论上可能包含空格。
    if account.username.trim().is_empty() {
        return Err(PortalError::InvalidConfig(
            "账号 username 不能为空".to_string(),
        ));
    }
    if account.password.is_empty() {
        return Err(PortalError::InvalidConfig(
            "账号 password 不能为空".to_string(),
        ));
    }
    Ok(())
}

/// Network-related configuration.
///
/// Runtime portal URLs live here so deployments can point the CLI at the
/// verified v3 endpoints without changing code. The redirect probe default must
/// remain plain `http://1.1.1.1` with redirects disabled by the HTTP client.
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    /// Gateway host or base URL used for headers and route probing.
    #[serde(default = "default_gateway")]
    pub gateway: String,
    /// NAS IP discovered from the portal redirect URL.
    #[serde(default)]
    pub nas_ip: Option<String>,
    /// Plain HTTP URL used to detect whether the network is already online.
    #[serde(default = "default_test_url")]
    pub test_url: String,
    /// v3 encrypted login endpoint.
    #[serde(default = "default_login_url")]
    pub login_url: String,
    /// Redirect URL template used to obtain a token for session APIs.
    #[serde(default = "default_session_login_redirect_url")]
    pub session_login_redirect_url: String,
    /// v3 encrypted session-list endpoint.
    #[serde(default = "default_session_list_url")]
    pub session_list_url: String,
    /// v3 encrypted session-logout endpoint.
    #[serde(default = "default_session_logout_url")]
    pub session_logout_url: String,
    /// HTTP request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl NetworkConfig {
    /// Returns the configured request timeout as a [`Duration`].
    pub fn timeout(&self) -> Duration {
        // 实现说明：reqwest 接受 Duration，配置文件保留秒数便于手写。
        Duration::from_secs(self.timeout_secs)
    }

    /// Returns the gateway as a normalized HTTP(S) base URL without a trailing slash.
    ///
    /// A bare host such as `10.184.6.32` is treated as `http://10.184.6.32`.
    pub fn gateway_base(&self) -> String {
        // 实现说明：请求头里的 Origin/Referer 需要 URL 形式；配置里允许用户只写
        // 网关地址以保持示例简洁。
        if self.gateway.starts_with("http://") || self.gateway.starts_with("https://") {
            self.gateway.trim_end_matches('/').to_string()
        } else {
            format!("http://{}", self.gateway.trim_end_matches('/'))
        }
    }
}

impl Default for NetworkConfig {
    /// Builds the default network configuration for the XJTU campus portal.
    fn default() -> Self {
        // 实现说明：默认值集中调用小函数，方便 serde default 复用同一组常量。
        Self {
            gateway: default_gateway(),
            nas_ip: None,
            test_url: default_test_url(),
            login_url: default_login_url(),
            session_login_redirect_url: default_session_login_redirect_url(),
            session_list_url: default_session_list_url(),
            session_logout_url: default_session_logout_url(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

/// Returns the default campus gateway host.
fn default_gateway() -> String {
    // 实现说明：保持 host 形态，gateway_base 会在需要 URL 时补 http://。
    "10.184.6.32".to_string()
}

/// Returns the redirect-probe URL.
fn default_test_url() -> String {
    // 实现说明：必须是 plain HTTP 的 1.1.1.1，配合禁用重定向才能拿到 portal
    // Location。
    "http://1.1.1.1".to_string()
}

/// Returns the verified v3 encrypted login endpoint.
fn default_login_url() -> String {
    // 实现说明：保留完整 URL，便于用户在 [network] 中整体覆盖。
    "http://10.184.6.32/portal-conversion/api/v3/portal/connect".to_string()
}

/// Returns the session-token redirect URL template.
fn default_session_login_redirect_url() -> String {
    // 实现说明：模板变量由 lib.rs 的 session_login_redirect_url 替换，并补齐缺失
    // query 参数。
    "http://10.184.6.32:80/wenet/auth?userip={local_ip}&usermac={local_mac}&nasip={nas_ip}"
        .to_string()
}

/// Returns the verified v3 encrypted session-list endpoint.
fn default_session_list_url() -> String {
    // 实现说明：不要退回旧 v2 token/session API，它们已验证不可用。
    "http://10.184.6.32/portal-conversion/api/v3/session/list".to_string()
}

/// Returns the verified v3 encrypted session-logout endpoint.
fn default_session_logout_url() -> String {
    // 实现说明：接口名里的 acctUniqueId 是网关现状，保持原样。
    "http://10.184.6.32/portal-conversion/api/v3/session/acctUniqueId".to_string()
}

/// Returns the default HTTP timeout in seconds.
fn default_timeout_secs() -> u64 {
    // 实现说明：5 秒让 OpenWrt 定时任务能快速失败重试，同时避免弱网下过早超时。
    5
}

/// Account credentials used for login.
#[derive(Debug, Clone, Deserialize)]
pub struct AccountConfig {
    /// Optional stable identifier referenced by advanced `targets`.
    pub id: Option<String>,
    /// Portal username, usually including the campus domain suffix.
    pub username: String,
    /// Portal password.
    pub password: String,
}

/// Automatic logout configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogoutConfig {
    /// Enables automatic logout when login reports the device limit.
    #[serde(default)]
    pub enabled: bool,
    /// Optional MAC used to identify the current device for `logout`.
    #[serde(default)]
    pub current_mac: Option<String>,
    /// Known devices used for names and automatic logout priority.
    #[serde(default)]
    pub known_macs: Vec<KnownMacConfig>,
}

/// A known device MAC address with an optional human-friendly name.
///
/// TOML accepts either a bare string MAC or an inline table
/// `{ mac = "...", name = "..." }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownMacConfig {
    /// Device MAC address in any format accepted by [`crate::session::normalize_mac`].
    pub mac: String,
    /// Optional selector/display name.
    pub name: Option<String>,
}

impl KnownMacConfig {
    /// Creates a known-MAC entry.
    pub fn new(mac: impl Into<String>, name: Option<String>) -> Self {
        // 实现说明：测试和 custom Deserialize 共用该构造函数，避免两处字段初始化
        // 形态漂移。
        Self {
            mac: mac.into(),
            name,
        }
    }
}

impl AsRef<str> for KnownMacConfig {
    /// Returns the raw MAC string.
    fn as_ref(&self) -> &str {
        // 实现说明：choose_logout_mac 只关心 MAC 文本，AsRef 让它可同时接受
        // KnownMacConfig 和 String。
        &self.mac
    }
}

impl<'de> Deserialize<'de> for KnownMacConfig {
    /// Deserializes a known MAC from either a string or an inline table.
    ///
    /// # Errors
    ///
    /// Returns the serde deserializer error if the TOML value is neither shape.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // 实现说明：untagged enum 精确表达两种公开 TOML 形态，避免手写 Value
        // 分支解析。
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum KnownMacValue {
            Mac(String),
            Named { mac: String, name: Option<String> },
        }

        match KnownMacValue::deserialize(deserializer)? {
            KnownMacValue::Mac(mac) => Ok(Self::new(mac, None)),
            KnownMacValue::Named { mac, name } => Ok(Self::new(mac, name)),
        }
    }
}

/// Local interface configuration referenced by a target.
#[derive(Debug, Clone, Deserialize)]
pub struct InterfaceConfig {
    /// Stable identifier used by `targets[*].interface`.
    pub id: String,
    /// OS interface name used for binding, such as `eth0`.
    pub name: Option<String>,
    /// Optional explicit source IP address.
    pub local_ip: Option<IpAddr>,
    /// Optional MAC address used for session redirect identity.
    pub mac: Option<String>,
}

impl InterfaceConfig {
    /// Resolves the configured or discovered local IPv4 address.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::InvalidLocalIp`] for non-IPv4 or unspecified
    /// addresses, and interface-inspection errors when resolving by name fails.
    pub fn local_ip(&self) -> Result<Option<IpAddr>> {
        // 实现说明：显式 local_ip 优先；否则如果有接口名，就从系统接口读取 IPv4。
        // 没有任何线索时返回 None，让调用方使用默认路由。
        if let Some(ip) = self.local_ip {
            ensure_ipv4(ip)?;
            return Ok(Some(ip));
        }
        if let Some(name) = &self.name {
            let ip = interface_ipv4(name)?;
            ensure_ipv4(IpAddr::V4(ip))?;
            return Ok(Some(IpAddr::V4(ip)));
        }
        Ok(None)
    }

    /// Returns a human-readable label for logs.
    pub fn label(&self) -> String {
        // 实现说明：name 更接近系统实际绑定目标；没有 name 时退回配置 ID。
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }
}

/// Ensures an IP address is a usable IPv4 address.
///
/// # Errors
///
/// Returns [`PortalError::InvalidLocalIp`] for IPv6, unspecified, or otherwise
/// unsupported addresses.
fn ensure_ipv4(ip: IpAddr) -> Result<()> {
    // 实现说明：reqwest local_address 可以接受 IPv6，但门户身份和会话匹配目前只按
    // IPv4 工作。
    match ip {
        IpAddr::V4(value) if value != Ipv4Addr::UNSPECIFIED => Ok(()),
        _ => Err(PortalError::InvalidLocalIp(ip)),
    }
}

/// A configured login target.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetConfig {
    /// Stable target identifier used by CLI `login TARGET_ID`.
    pub id: String,
    /// Account ID referenced from `[[accounts]]`.
    pub account: String,
    /// Optional interface ID referenced from `[[interfaces]]`.
    pub interface: Option<String>,
}

/// Runtime target with account and interface references already resolved.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    /// Target identifier.
    pub id: String,
    /// Account credentials to use for this target.
    pub account: AccountConfig,
    /// Optional interface binding configuration.
    pub interface: Option<InterfaceConfig>,
}

impl ResolvedTarget {
    /// Builds the HTTP binding requested by this target.
    ///
    /// # Errors
    ///
    /// Returns local-IP validation or interface inspection errors from the
    /// referenced interface.
    pub fn network_binding(&self) -> Result<NetworkBinding> {
        // 实现说明：interface.name 进入 reqwest interface 绑定；interface.local_ip()
        // 只作为附加 source-address hint。
        let Some(interface) = &self.interface else {
            return Ok(NetworkBinding::default());
        };

        Ok(NetworkBinding {
            interface_name: interface.name.clone(),
            local_ip: interface.local_ip()?,
        })
    }

    /// Returns the target interface label for logs.
    pub fn interface_label(&self) -> Option<String> {
        // 实现说明：没有接口绑定时返回 None，让日志统一显示 default。
        self.interface.as_ref().map(InterfaceConfig::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Verifies simple configuration resolves to one default target.
    fn resolves_default_target_when_targets_are_absent() {
        // 实现说明：覆盖 targets 缺省时 default_account -> ResolvedTarget 的兼容路径。
        let config: AppConfig = toml::from_str(
            r#"
            [default_account]
            username = "u@xjtu"
            password = "p"
            "#,
        )
        .unwrap();

        let targets = config.resolved_targets().unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "default");
        assert_eq!(targets[0].account.username, "u@xjtu");
        assert!(targets[0].interface.is_none());
    }

    #[test]
    /// Verifies advanced targets are resolved in configuration order.
    fn resolves_configured_targets_in_order() {
        // 实现说明：同时覆盖 account/interface 引用解析和 local_ip binding 输出。
        let config: AppConfig = toml::from_str(
            r#"
            [[accounts]]
            id = "main"
            username = "u@xjtu"
            password = "p"

            [[interfaces]]
            id = "wan1"
            local_ip = "10.180.0.10"

            [[targets]]
            id = "wan1-main"
            account = "main"
            interface = "wan1"
            "#,
        )
        .unwrap();

        let targets = config.resolved_targets().unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "wan1-main");
        assert_eq!(
            targets[0].network_binding().unwrap(),
            NetworkBinding {
                interface_name: None,
                local_ip: Some("10.180.0.10".parse().unwrap())
            }
        );
    }

    #[test]
    /// Verifies a missing default account is rejected in simple mode.
    fn rejects_missing_default_account() {
        // 实现说明：空配置没有 targets，因此会尝试 default_target 并报错。
        let config: AppConfig = toml::from_str("").unwrap();

        assert!(config.resolved_targets().is_err());
    }

    #[test]
    /// Verifies targets cannot reference unknown accounts.
    fn rejects_unknown_target_account() {
        // 实现说明：target.account 必须命中 [[accounts]].id。
        let config: AppConfig = toml::from_str(
            r#"
            [[targets]]
            id = "bad"
            account = "missing"
            "#,
        )
        .unwrap();

        assert!(config.resolved_targets().is_err());
    }

    #[test]
    /// Verifies known MAC entries accept both string and named-table TOML forms.
    fn parses_named_known_macs() {
        // 实现说明：覆盖 KnownMacConfig 的 untagged Deserialize 两个分支。
        let config: AppConfig = toml::from_str(
            r#"
            [logout]
            known_macs = [
              "11:22:33:44:55:66",
              { mac = "aa:bb:cc:dd:ee:ff", name = "phone" },
            ]
            "#,
        )
        .unwrap();

        assert_eq!(
            config.logout.known_macs,
            vec![
                KnownMacConfig::new("11:22:33:44:55:66", None),
                KnownMacConfig::new("aa:bb:cc:dd:ee:ff", Some("phone".to_string())),
            ]
        );
    }

    #[test]
    /// Verifies public example configuration files stay parseable.
    fn example_configs_parse() {
        // 实现说明：把示例文件纳入测试，避免文档配置形态落后于 Deserialize 模型。
        toml::from_str::<AppConfig>(include_str!("../config.example.toml")).unwrap();
        toml::from_str::<AppConfig>(include_str!("../config.advanced.example.toml")).unwrap();
    }

    #[tokio::test]
    /// Verifies `network.nas_ip` can be inserted and skipped on no-op updates.
    async fn writes_network_nas_ip() {
        // 实现说明：第一次写入应返回 true；相同值第二次写入应返回 false。
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            file.path(),
            r#"
            [network]
            gateway = "10.184.6.32"

            [default_account]
            username = "u@xjtu"
            password = "p"
            "#,
        )
        .unwrap();

        assert!(
            write_network_nas_ip(file.path(), "10.6.33.10")
                .await
                .unwrap()
        );
        assert!(
            !write_network_nas_ip(file.path(), "10.6.33.10")
                .await
                .unwrap()
        );

        let config = AppConfig::read(file.path()).unwrap();
        assert_eq!(config.network.nas_ip.as_deref(), Some("10.6.33.10"));
    }

    #[tokio::test]
    /// Verifies NAS IP write-back preserves surrounding TOML comments.
    async fn updates_network_nas_ip_without_removing_comments() {
        // 实现说明：测试 toml_edit 路径，而不是 AppConfig 反序列化重写路径。
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            file.path(),
            r#"# root comment
[network] # network comment
gateway = "10.184.6.32" # gateway comment
nas_ip = "10.0.0.1" # nas comment

# account comment
[default_account]
username = "u@xjtu"
password = "p"
"#,
        )
        .unwrap();

        assert!(
            write_network_nas_ip(file.path(), "10.6.33.10")
                .await
                .unwrap()
        );

        let updated = fs::read_to_string(file.path()).unwrap();
        assert!(updated.contains("# root comment"));
        assert!(updated.contains("[network] # network comment"));
        assert!(updated.contains("gateway = \"10.184.6.32\" # gateway comment"));
        assert!(updated.contains("nas_ip = \"10.6.33.10\" # nas comment"));
        assert!(updated.contains("# account comment"));
    }
}
