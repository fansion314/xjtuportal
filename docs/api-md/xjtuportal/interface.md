**xjtuportal > interface**

# Module: interface

## Contents

**Functions**

- [`interface_ipv4`](#interface_ipv4) - Returns the first usable IPv4 address assigned to an interface name.
- [`interface_mac_for_ip`](#interface_mac_for_ip) - Finds the MAC address of the interface that owns a specific IP address.
- [`local_address_display`](#local_address_display) - Formats an optional local address for logs.

---

## xjtuportal::interface::interface_ipv4

*Function*

Returns the first usable IPv4 address assigned to an interface name.

Loopback, link-local, and unspecified addresses are ignored because the
portal needs a routable LAN address.

# Errors

Returns [`PortalError::InterfaceInspect`] if interface enumeration fails, or
[`PortalError::InterfaceAddressMissing`] if the named interface has no usable
IPv4 address.

```rust
fn interface_ipv4(name: &str) -> crate::error::Result<std::net::Ipv4Addr>
```



## xjtuportal::interface::interface_mac_for_ip

*Function*

Finds the MAC address of the interface that owns a specific IP address.

The returned MAC keeps the operating system's text format. Callers that send
it to the portal should normalize or convert separators as needed.

# Errors

Returns [`PortalError::InterfaceInspect`] if local interface enumeration or a
platform MAC lookup fails.

```rust
fn interface_mac_for_ip(ip: std::net::IpAddr) -> crate::error::Result<Option<String>>
```



## xjtuportal::interface::local_address_display

*Function*

Formats an optional local address for logs.

`None` is displayed as `default`, matching the wording used by binding logs.

```rust
fn local_address_display(local_ip: Option<std::net::IpAddr>) -> String
```



