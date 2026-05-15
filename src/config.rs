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
            let account = self.default_account.clone().ok_or_else(|| {
                PortalError::InvalidConfig(
                    "[default_account] is required when [[targets]] is not configured".to_string(),
                )
            })?;
            validate_account(&account)?;
            return Ok(vec![ResolvedTarget {
                id: "default".to_string(),
                account,
                interface: None,
            }]);
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
                        "target {} references unknown account {}",
                        target.id, target.account
                    ))
                })?;
                validate_account(account)?;
                let interface = match &target.interface {
                    Some(interface_id) => {
                        let interface = interfaces.get(interface_id.as_str()).ok_or_else(|| {
                            PortalError::InvalidConfig(format!(
                                "target {} references unknown interface {}",
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
}

fn validate_account(account: &AccountConfig) -> Result<()> {
    if account.username.trim().is_empty() {
        return Err(PortalError::InvalidConfig(
            "account username cannot be empty".to_string(),
        ));
    }
    if account.password.is_empty() {
        return Err(PortalError::InvalidConfig(
            "account password cannot be empty".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_gateway")]
    pub gateway: String,
    #[serde(default = "default_test_url")]
    pub test_url: String,
    #[serde(default = "default_login_url")]
    pub login_url: String,
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
            test_url: default_test_url(),
            login_url: default_login_url(),
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
    pub known_macs: Vec<String>,
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
    pub fn local_ip(&self) -> Result<Option<IpAddr>> {
        self.interface
            .as_ref()
            .map(InterfaceConfig::local_ip)
            .unwrap_or(Ok(None))
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
            targets[0].local_ip().unwrap(),
            Some("10.180.0.10".parse().unwrap())
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
}
