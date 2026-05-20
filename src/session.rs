//! Session response normalization and automatic logout selection.
//!
//! The portal session-list API returns partially optional fields and MAC
//! addresses in the API's original formatting. This module converts those
//! records into valid [`Session`] values while preserving the original MAC text
//! needed by the logout endpoint.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use crate::error::{PortalError, Result};

/// A validated portal session ready for display or logout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Normalized MAC address used for local comparison.
    pub mac: String,
    /// API-provided MAC string preserved for logout payloads.
    pub api_mac: String,
    /// Portal-reported device type.
    pub device_type: String,
    /// Portal-reported user IP.
    pub user_ip: String,
    /// Portal-reported accounting start time.
    pub start_time: String,
    /// Portal accounting unique ID required by the logout API.
    pub unique_id: String,
}

/// Raw response returned by the encrypted v3 session-list endpoint.
#[derive(Debug, Deserialize)]
pub struct SessionListResponse {
    /// Portal concurrency metadata. It is currently not needed by the CLI.
    #[serde(default)]
    pub concurrency: serde_json::Value,
    /// Raw session records.
    #[serde(default)]
    pub sessions: Vec<PortalSession>,
}

impl SessionListResponse {
    /// Converts the raw response into valid, deduplicated sessions.
    ///
    /// Records without MAC, unique ID, or user IP are discarded because they
    /// cannot be shown or safely logged out.
    pub fn into_sessions(self) -> Vec<Session> {
        // 实现说明：用 normalized MAC 去重，但把 api_mac 原样保留给 logout；网关对
        // payload 里的 MAC 格式可能敏感。
        let mut seen = HashSet::new();
        let mut sessions = Vec::new();

        for session in self.sessions {
            let Some(api_mac) = session.calling_station_id.filter(|value| !value.is_empty()) else {
                continue;
            };
            let Ok(mac) = normalize_mac(&api_mac) else {
                continue;
            };
            if !seen.insert(mac.clone()) {
                continue;
            }
            let Some(unique_id) = session.acct_unique_id.filter(|value| !value.is_empty()) else {
                continue;
            };
            let user_ip = session.framed_ip_address.unwrap_or_default();
            if user_ip.is_empty() {
                continue;
            }
            sessions.push(Session {
                mac,
                api_mac,
                device_type: session.device_type.unwrap_or_default(),
                user_ip,
                start_time: session.acct_start_time.unwrap_or_default(),
                unique_id,
            });
        }

        sessions
    }
}

/// Raw session record returned by the portal.
///
/// Field names intentionally follow the portal's mixed naming style so serde can
/// deserialize the encrypted JSON response without an intermediate map.
#[derive(Debug, Deserialize)]
pub struct PortalSession {
    /// Raw `deviceType` field.
    #[serde(default, rename = "deviceType")]
    pub device_type: Option<String>,
    /// Raw `framed_ip_address` field.
    #[serde(default)]
    pub framed_ip_address: Option<String>,
    /// Raw `calling_station_id` MAC field.
    #[serde(default)]
    pub calling_station_id: Option<String>,
    /// Raw session start time field.
    #[serde(default)]
    pub acct_start_time: Option<String>,
    /// Raw accounting unique ID field.
    #[serde(default)]
    pub acct_unique_id: Option<String>,
}

/// Chooses which MAC address to log out when an account is over the device limit.
///
/// The strategy is:
///
/// 1. Prefer the first active session not listed in `known_macs`.
/// 2. If every active session is known, choose by `known_macs` order.
/// 3. If no configured known MAC matches, fall back to the first valid session.
///
/// Invalid MAC strings are ignored.
pub fn choose_logout_mac<K>(session_macs: &[&str], known_macs: &[K]) -> Option<String>
where
    K: AsRef<str>,
{
    // 实现说明：先把会话 MAC 规范化并按出现顺序去重，随后把 known_macs 同样规范化。
    // 这样不同分隔符和大小写不会影响选择策略。
    if session_macs.is_empty() {
        return None;
    }

    let mut session_order = Vec::new();
    let mut session_set = HashSet::new();
    for mac in session_macs {
        let Ok(mac) = normalize_mac(mac) else {
            continue;
        };
        if session_set.insert(mac.clone()) {
            session_order.push(mac);
        }
    }
    if session_order.is_empty() {
        return None;
    }

    let known_order = known_macs
        .iter()
        .filter_map(|mac| normalize_mac(mac.as_ref()).ok())
        .collect::<Vec<_>>();
    let known_set = known_order.iter().cloned().collect::<HashSet<_>>();

    for mac in &session_order {
        if !known_set.contains(mac) {
            return Some(mac.clone());
        }
    }

    let session_lookup = session_order
        .iter()
        .map(|mac| (mac.as_str(), ()))
        .collect::<HashMap<_, _>>();
    for mac in known_order {
        if session_lookup.contains_key(mac.as_str()) {
            return Some(mac);
        }
    }

    session_order.into_iter().next()
}

/// Normalizes a MAC address to lowercase colon-separated form.
///
/// Accepts one- or two-digit hex octets separated by `:` or `-` and returns a
/// six-octet `aa:bb:cc:dd:ee:ff` string.
///
/// # Errors
///
/// Returns [`PortalError::InvalidMac`] if the input is not six hexadecimal
/// octets.
pub fn normalize_mac(mac: &str) -> Result<String> {
    // 实现说明：逐段 parse 成 u8 再用 {:02x} 输出，既校验范围又自动补齐单字符
    // octet。
    let replaced = mac.trim().replace('-', ":").to_ascii_lowercase();
    let parts = replaced.split(':').collect::<Vec<_>>();
    if parts.len() != 6 {
        return Err(PortalError::InvalidMac(mac.to_string()));
    }

    let mut normalized = Vec::with_capacity(6);
    for part in parts {
        if part.is_empty() || part.len() > 2 || !part.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(PortalError::InvalidMac(mac.to_string()));
        }
        let value =
            u8::from_str_radix(part, 16).map_err(|_| PortalError::InvalidMac(mac.to_string()))?;
        normalized.push(format!("{value:02x}"));
    }

    Ok(normalized.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts test MAC literals into owned strings.
    fn macs(values: &[&str]) -> Vec<String> {
        // 实现说明：让 choose_logout_mac 的泛型分支覆盖 String/AsRef<str>。
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    /// Verifies unknown sessions are selected before configured known devices.
    fn chooses_unknown_mac_first() {
        // 实现说明：第一项不在 known_macs 中，应立即成为下线候选。
        let session_macs = ["00:00:5e:00:53:01", "11:22:33:44:55:66"];
        let known_macs = macs(&["11:22:33:44:55:66", "aa:bb:cc:dd:ee:ff"]);

        assert_eq!(
            choose_logout_mac(&session_macs, &known_macs),
            Some("00:00:5e:00:53:01".to_string())
        );
    }

    #[test]
    /// Verifies known-device order is used when all sessions are known.
    fn chooses_known_mac_by_config_order_when_all_are_known() {
        // 实现说明：会话顺序和配置顺序不同，断言配置顺序优先。
        let session_macs = ["aa:bb:cc:dd:ee:ff", "11:22:33:44:55:66"];
        let known_macs = macs(&["11:22:33:44:55:66", "aa:bb:cc:dd:ee:ff"]);

        assert_eq!(
            choose_logout_mac(&session_macs, &known_macs),
            Some("11:22:33:44:55:66".to_string())
        );
    }

    #[test]
    /// Verifies MAC normalization and deduplication in logout selection.
    fn deduplicates_sessions_and_normalizes_macs() {
        // 实现说明：同一 MAC 的横线和冒号格式只应产生一个候选。
        let session_macs = ["AA-BB-CC-DD-EE-FF", "aa:bb:cc:dd:ee:ff"];
        let known_macs = macs(&[]);

        assert_eq!(
            choose_logout_mac(&session_macs, &known_macs),
            Some("aa:bb:cc:dd:ee:ff".to_string())
        );
    }

    #[test]
    /// Verifies an empty session list produces no logout candidate.
    fn returns_none_for_empty_sessions() {
        // 实现说明：没有 active sessions 时自动下线必须交给上层报错。
        assert_eq!(choose_logout_mac(&[], &Vec::<String>::new()), None);
    }
}
