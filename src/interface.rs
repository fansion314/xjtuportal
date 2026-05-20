//! Local network-interface discovery helpers.
//!
//! This module is deliberately small and platform-aware. It resolves interface
//! IPv4 addresses for configuration, discovers MAC addresses for redirect URL
//! construction, and leaves actual socket binding to `reqwest` in the protocol
//! client.

use std::{
    net::{IpAddr, Ipv4Addr},
    process::Command,
};

use get_if_addrs::{IfAddr, get_if_addrs};

use crate::error::{PortalError, Result};

/// Returns the first usable IPv4 address assigned to an interface name.
///
/// Loopback, link-local, and unspecified addresses are ignored because the
/// portal needs a routable LAN address.
///
/// # Errors
///
/// Returns [`PortalError::InterfaceInspect`] if interface enumeration fails, or
/// [`PortalError::InterfaceAddressMissing`] if the named interface has no usable
/// IPv4 address.
pub fn interface_ipv4(name: &str) -> Result<Ipv4Addr> {
    // 实现说明：get_if_addrs 已经提供跨平台枚举；这里保留第一个可用 IPv4，避免
    // 对多地址网卡做额外策略判断。
    for interface in get_if_addrs()? {
        if interface.name != name {
            continue;
        }
        let IfAddr::V4(v4) = interface.addr else {
            continue;
        };
        if usable_ipv4(v4.ip) {
            return Ok(v4.ip);
        }
    }

    Err(PortalError::InterfaceAddressMissing {
        name: name.to_string(),
    })
}

/// Finds the MAC address of the interface that owns a specific IP address.
///
/// The returned MAC keeps the operating system's text format. Callers that send
/// it to the portal should normalize or convert separators as needed.
///
/// # Errors
///
/// Returns [`PortalError::InterfaceInspect`] if local interface enumeration or a
/// platform MAC lookup fails.
pub fn interface_mac_for_ip(ip: IpAddr) -> Result<Option<String>> {
    // 实现说明：先用 get_if_addrs 把 IP 映射到接口名，再委托 interface_mac 做
    // 平台相关的 MAC 查询。
    for interface in get_if_addrs()? {
        let interface_ip = match interface.addr {
            IfAddr::V4(v4) => IpAddr::V4(v4.ip),
            IfAddr::V6(v6) => IpAddr::V6(v6.ip),
        };
        if interface_ip == ip {
            return interface_mac(&interface.name);
        }
    }

    Ok(None)
}

/// Reads the MAC address for a named interface.
///
/// Linux prefers `/sys/class/net/<name>/address`; other Unix-like systems fall
/// back to parsing `ifconfig <name>`.
///
/// # Errors
///
/// Returns [`PortalError::InterfaceInspect`] when the OS command or sysfs read
/// fails for reasons other than a missing sysfs address file.
fn interface_mac(name: &str) -> Result<Option<String>> {
    // 实现说明：OpenWrt/Linux 上 sysfs 最可靠也最轻量；macOS 等平台通常没有该
    // 路径，因此再退到 ifconfig 的 ether 行。
    #[cfg(target_os = "linux")]
    {
        let path = format!("/sys/class/net/{name}/address");
        match std::fs::read_to_string(path) {
            Ok(value) => {
                let value = value.trim();
                if !value.is_empty() {
                    return Ok(Some(value.to_string()));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }

    let output = Command::new("ifconfig").arg(name).output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        let Some(mac) = line.strip_prefix("ether ") else {
            continue;
        };
        let mac = mac.split_whitespace().next().unwrap_or_default();
        if !mac.is_empty() {
            return Ok(Some(mac.to_string()));
        }
    }

    Ok(None)
}

/// Returns whether an IPv4 address is suitable for portal identity and binding.
fn usable_ipv4(ip: Ipv4Addr) -> bool {
    // 实现说明：门户和 SO_BINDTODEVICE 场景都需要实际接口地址，排除不会出现在
    // 校园网会话中的特殊地址。
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

/// Formats an optional local address for logs.
///
/// `None` is displayed as `default`, matching the wording used by binding logs.
pub fn local_address_display(local_ip: Option<IpAddr>) -> String {
    // 实现说明：日志层只需要稳定的人类可读字符串，不需要区分 IPv4/IPv6。
    local_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "default".to_string())
}
