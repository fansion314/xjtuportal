use std::net::{IpAddr, Ipv4Addr};

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

fn usable_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

pub fn local_address_display(local_ip: Option<IpAddr>) -> String {
    local_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "default".to_string())
}
