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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkBinding {
    pub interface_name: Option<String>,
    pub local_ip: Option<IpAddr>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub network: NetworkConfig,
    pub default_account: Option<AccountConfig>,
    #[serde(default)]
    pub logout: LogoutConfig,
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    #[serde(default)]
    pub interfaces: Vec<InterfaceConfig>,
    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

impl AppConfig {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
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

    pub fn resolved_targets(&self) -> Result<Vec<ResolvedTarget>> {
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

    pub fn default_target(&self) -> Result<ResolvedTarget> {
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

pub async fn write_network_nas_ip(path: impl AsRef<Path>, nas_ip: &str) -> Result<bool> {
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

fn validate_account(account: &AccountConfig) -> Result<()> {
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

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_gateway")]
    pub gateway: String,
    #[serde(default)]
    pub nas_ip: Option<String>,
    #[serde(default = "default_test_url")]
    pub test_url: String,
    #[serde(default = "default_login_url")]
    pub login_url: String,
    #[serde(default = "default_session_login_redirect_url")]
    pub session_login_redirect_url: String,
    #[serde(default = "default_session_list_url")]
    pub session_list_url: String,
    #[serde(default = "default_session_logout_url")]
    pub session_logout_url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl NetworkConfig {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    pub fn gateway_base(&self) -> String {
        if self.gateway.starts_with("http://") || self.gateway.starts_with("https://") {
            self.gateway.trim_end_matches('/').to_string()
        } else {
            format!("http://{}", self.gateway.trim_end_matches('/'))
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
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

fn default_gateway() -> String {
    "10.184.6.32".to_string()
}

fn default_test_url() -> String {
    "http://1.1.1.1".to_string()
}

fn default_login_url() -> String {
    "http://10.184.6.32/portal-conversion/api/v3/portal/connect".to_string()
}

fn default_session_login_redirect_url() -> String {
    "http://10.184.6.32:80/wenet/auth?userip={local_ip}&usermac={local_mac}&nasip={nas_ip}&"
        .to_string()
}

fn default_session_list_url() -> String {
    "http://10.184.6.32/portal-conversion/api/v3/session/list".to_string()
}

fn default_session_logout_url() -> String {
    "http://10.184.6.32/portal-conversion/api/v3/session/acctUniqueId".to_string()
}

fn default_timeout_secs() -> u64 {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountConfig {
    pub id: Option<String>,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogoutConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub current_mac: Option<String>,
    #[serde(default)]
    pub known_macs: Vec<KnownMacConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownMacConfig {
    pub mac: String,
    pub name: Option<String>,
}

impl KnownMacConfig {
    pub fn new(mac: impl Into<String>, name: Option<String>) -> Self {
        Self {
            mac: mac.into(),
            name,
        }
    }
}

impl AsRef<str> for KnownMacConfig {
    fn as_ref(&self) -> &str {
        &self.mac
    }
}

impl<'de> Deserialize<'de> for KnownMacConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
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

#[derive(Debug, Clone, Deserialize)]
pub struct InterfaceConfig {
    pub id: String,
    pub name: Option<String>,
    pub local_ip: Option<IpAddr>,
    pub mac: Option<String>,
}

impl InterfaceConfig {
    pub fn local_ip(&self) -> Result<Option<IpAddr>> {
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

    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }
}

fn ensure_ipv4(ip: IpAddr) -> Result<()> {
    match ip {
        IpAddr::V4(value) if value != Ipv4Addr::UNSPECIFIED => Ok(()),
        _ => Err(PortalError::InvalidLocalIp(ip)),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TargetConfig {
    pub id: String,
    pub account: String,
    pub interface: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub id: String,
    pub account: AccountConfig,
    pub interface: Option<InterfaceConfig>,
}

impl ResolvedTarget {
    pub fn network_binding(&self) -> Result<NetworkBinding> {
        let Some(interface) = &self.interface else {
            return Ok(NetworkBinding::default());
        };

        Ok(NetworkBinding {
            interface_name: interface.name.clone(),
            local_ip: interface.local_ip()?,
        })
    }

    pub fn interface_label(&self) -> Option<String> {
        self.interface.as_ref().map(InterfaceConfig::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_target_when_targets_are_absent() {
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
    fn resolves_configured_targets_in_order() {
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
    fn rejects_missing_default_account() {
        let config: AppConfig = toml::from_str("").unwrap();

        assert!(config.resolved_targets().is_err());
    }

    #[test]
    fn rejects_unknown_target_account() {
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
    fn parses_named_known_macs() {
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
    fn example_configs_parse() {
        toml::from_str::<AppConfig>(include_str!("../config.example.toml")).unwrap();
        toml::from_str::<AppConfig>(include_str!("../config.advanced.example.toml")).unwrap();
    }

    #[tokio::test]
    async fn writes_network_nas_ip() {
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
    async fn updates_network_nas_ip_without_removing_comments() {
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
