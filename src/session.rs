use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use crate::error::{PortalError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub mac: String,
    pub user_ip: String,
    pub start_time: String,
    pub unique_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionListResponse {
    #[serde(default)]
    pub concurrency: serde_json::Value,
    #[serde(default)]
    pub sessions: Vec<PortalSession>,
}

impl SessionListResponse {
    pub fn into_sessions(self) -> Vec<Session> {
        let mut seen = HashSet::new();
        let mut sessions = Vec::new();

        for session in self.sessions {
            let Some(mac) = session
                .calling_station_id
                .as_deref()
                .and_then(|mac| normalize_mac(mac).ok())
            else {
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
                user_ip,
                start_time: session.acct_start_time.unwrap_or_default(),
                unique_id,
            });
        }

        sessions
    }
}

#[derive(Debug, Deserialize)]
pub struct PortalSession {
    #[serde(default)]
    pub framed_ip_address: Option<String>,
    #[serde(default)]
    pub calling_station_id: Option<String>,
    #[serde(default)]
    pub acct_start_time: Option<String>,
    #[serde(default)]
    pub acct_unique_id: Option<String>,
}

pub fn choose_logout_mac(session_macs: &[&str], known_macs: &[String]) -> Option<String> {
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
        .filter_map(|mac| normalize_mac(mac).ok())
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

pub fn normalize_mac(mac: &str) -> Result<String> {
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

    fn macs(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn chooses_unknown_mac_first() {
        let session_macs = ["00:00:5e:00:53:01", "11:22:33:44:55:66"];
        let known_macs = macs(&["11:22:33:44:55:66", "aa:bb:cc:dd:ee:ff"]);

        assert_eq!(
            choose_logout_mac(&session_macs, &known_macs),
            Some("00:00:5e:00:53:01".to_string())
        );
    }

    #[test]
    fn chooses_known_mac_by_config_order_when_all_are_known() {
        let session_macs = ["aa:bb:cc:dd:ee:ff", "11:22:33:44:55:66"];
        let known_macs = macs(&["11:22:33:44:55:66", "aa:bb:cc:dd:ee:ff"]);

        assert_eq!(
            choose_logout_mac(&session_macs, &known_macs),
            Some("11:22:33:44:55:66".to_string())
        );
    }

    #[test]
    fn deduplicates_sessions_and_normalizes_macs() {
        let session_macs = ["AA-BB-CC-DD-EE-FF", "aa:bb:cc:dd:ee:ff"];
        let known_macs = macs(&[]);

        assert_eq!(
            choose_logout_mac(&session_macs, &known_macs),
            Some("aa:bb:cc:dd:ee:ff".to_string())
        );
    }

    #[test]
    fn returns_none_for_empty_sessions() {
        assert_eq!(choose_logout_mac(&[], &[]), None);
    }
}
