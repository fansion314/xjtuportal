use std::{
    net::{IpAddr, Ipv4Addr},
    process::Command,
};

use get_if_addrs::{IfAddr, get_if_addrs};

use crate::error::{PortalError, Result};

pub fn interface_ipv4(name: &str) -> Result<Ipv4Addr> {
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

pub fn interface_mac_for_ip(ip: IpAddr) -> Result<Option<String>> {
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

fn interface_mac(name: &str) -> Result<Option<String>> {
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

fn usable_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

pub fn local_address_display(local_ip: Option<IpAddr>) -> String {
    local_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "default".to_string())
}
