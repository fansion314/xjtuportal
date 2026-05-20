**xjtuportal > config**

# Module: config

## Contents

**Structs**

- [`AccountConfig`](#accountconfig) - Account credentials used for login.
- [`AppConfig`](#appconfig) - Root configuration deserialized from `config.toml`.
- [`InterfaceConfig`](#interfaceconfig) - Local interface configuration referenced by a target.
- [`KnownMacConfig`](#knownmacconfig) - A known device MAC address with an optional human-friendly name.
- [`LogoutConfig`](#logoutconfig) - Automatic logout configuration.
- [`NetworkBinding`](#networkbinding) - Network binding requested by a resolved target.
- [`NetworkConfig`](#networkconfig) - Network-related configuration.
- [`ResolvedTarget`](#resolvedtarget) - Runtime target with account and interface references already resolved.
- [`TargetConfig`](#targetconfig) - A configured login target.

**Functions**

- [`write_network_nas_ip`](#write_network_nas_ip) - Writes `network.nas_ip` into a TOML configuration file.

---

## xjtuportal::config::AccountConfig

*Struct*

Account credentials used for login.

**Fields:**
- `id: Option<String>` - Optional stable identifier referenced by advanced `targets`.
- `username: String` - Portal username, usually including the campus domain suffix.
- `password: String` - Portal password.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Clone**
  - `fn clone(self: &Self) -> AccountConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::config::AppConfig

*Struct*

Root configuration deserialized from `config.toml`.

The public configuration shape is documented by `config.example.toml` and
`config.advanced.example.toml`. `default_account` drives the simple mode;
`accounts`, `interfaces`, and `targets` drive advanced unattended login.

**Fields:**
- `network: NetworkConfig` - Network URLs, timeout, and discovered NAS address.
- `default_account: Option<AccountConfig>` - Single-account mode configuration used when no `targets` are present.
- `logout: LogoutConfig` - Automatic logout settings and known device names.
- `accounts: Vec<AccountConfig>` - Named accounts used by advanced targets.
- `interfaces: Vec<InterfaceConfig>` - Named local interfaces used by advanced targets.
- `targets: Vec<TargetConfig>` - Login targets that pair an account with an optional interface.

**Methods:**

- `fn read<impl AsRef<Path>>(path: impl Trait) -> Result<Self>` - Reads and parses an application configuration file.
- `fn resolved_targets(self: &Self) -> Result<Vec<ResolvedTarget>>` - Resolves the configured login targets into validated runtime targets.
- `fn account_targets(self: &Self) -> Result<Vec<(AccountConfig, Vec<ResolvedTarget>)>>` - Groups resolved targets by account while preserving account order.
- `fn default_target(self: &Self) -> Result<ResolvedTarget>` - Builds the simple-mode default target from `[default_account]`.

**Trait Implementations:**

- **Clone**
  - `fn clone(self: &Self) -> AppConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`



## xjtuportal::config::InterfaceConfig

*Struct*

Local interface configuration referenced by a target.

**Fields:**
- `id: String` - Stable identifier used by `targets[*].interface`.
- `name: Option<String>` - OS interface name used for binding, such as `eth0`.
- `local_ip: Option<std::net::IpAddr>` - Optional explicit source IP address.
- `mac: Option<String>` - Optional MAC address used for session redirect identity.

**Methods:**

- `fn local_ip(self: &Self) -> Result<Option<IpAddr>>` - Resolves the configured or discovered local IPv4 address.
- `fn label(self: &Self) -> String` - Returns a human-readable label for logs.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Clone**
  - `fn clone(self: &Self) -> InterfaceConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::config::KnownMacConfig

*Struct*

A known device MAC address with an optional human-friendly name.

TOML accepts either a bare string MAC or an inline table
`{ mac = "...", name = "..." }`.

**Fields:**
- `mac: String` - Device MAC address in any format accepted by [`crate::session::normalize_mac`].
- `name: Option<String>` - Optional selector/display name.

**Methods:**

- `fn new<impl Into<String>>(mac: impl Trait, name: Option<String>) -> Self` - Creates a known-MAC entry.

**Traits:** Eq

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<D>(deserializer: D) -> std::result::Result<Self, <D as >::Error>` - Deserializes a known MAC from either a string or an inline table.
- **AsRef**
  - `fn as_ref(self: &Self) -> &str` - Returns the raw MAC string.
- **Clone**
  - `fn clone(self: &Self) -> KnownMacConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **PartialEq**
  - `fn eq(self: &Self, other: &KnownMacConfig) -> bool`



## xjtuportal::config::LogoutConfig

*Struct*

Automatic logout configuration.

**Fields:**
- `enabled: bool` - Enables automatic logout when login reports the device limit.
- `current_mac: Option<String>` - Optional MAC used to identify the current device for `logout`.
- `known_macs: Vec<KnownMacConfig>` - Known devices used for names and automatic logout priority.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Clone**
  - `fn clone(self: &Self) -> LogoutConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **Default**
  - `fn default() -> LogoutConfig`



## xjtuportal::config::NetworkBinding

*Struct*

Network binding requested by a resolved target.

`interface_name` is the primary selector and is passed to
`reqwest::ClientBuilder::interface` on supported platforms. `local_ip` is an
optional source-address hint and should not be used as a substitute for
interface binding on OpenWrt/mwan3 setups.

**Fields:**
- `interface_name: Option<String>` - Interface name to bind the HTTP client to, such as `eth0` or `wan`.
- `local_ip: Option<std::net::IpAddr>` - Optional local source address to use in addition to interface binding.

**Traits:** Eq

**Trait Implementations:**

- **PartialEq**
  - `fn eq(self: &Self, other: &NetworkBinding) -> bool`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **Default**
  - `fn default() -> NetworkBinding`
- **Clone**
  - `fn clone(self: &Self) -> NetworkBinding`



## xjtuportal::config::NetworkConfig

*Struct*

Network-related configuration.

Runtime portal URLs live here so deployments can point the CLI at the
verified v3 endpoints without changing code. The redirect probe default must
remain plain `http://1.1.1.1` with redirects disabled by the HTTP client.

**Fields:**
- `gateway: String` - Gateway host or base URL used for headers and route probing.
- `nas_ip: Option<String>` - NAS IP discovered from the portal redirect URL.
- `test_url: String` - Plain HTTP URL used to detect whether the network is already online.
- `login_url: String` - v3 encrypted login endpoint.
- `session_login_redirect_url: String` - Redirect URL template used to obtain a token for session APIs.
- `session_list_url: String` - v3 encrypted session-list endpoint.
- `session_logout_url: String` - v3 encrypted session-logout endpoint.
- `timeout_secs: u64` - HTTP request timeout in seconds.

**Methods:**

- `fn timeout(self: &Self) -> Duration` - Returns the configured request timeout as a [`Duration`].
- `fn gateway_base(self: &Self) -> String` - Returns the gateway as a normalized HTTP(S) base URL without a trailing slash.

**Trait Implementations:**

- **Default**
  - `fn default() -> Self` - Builds the default network configuration for the XJTU campus portal.
- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Clone**
  - `fn clone(self: &Self) -> NetworkConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::config::ResolvedTarget

*Struct*

Runtime target with account and interface references already resolved.

**Fields:**
- `id: String` - Target identifier.
- `account: AccountConfig` - Account credentials to use for this target.
- `interface: Option<InterfaceConfig>` - Optional interface binding configuration.

**Methods:**

- `fn network_binding(self: &Self) -> Result<NetworkBinding>` - Builds the HTTP binding requested by this target.
- `fn interface_label(self: &Self) -> Option<String>` - Returns the target interface label for logs.

**Trait Implementations:**

- **Clone**
  - `fn clone(self: &Self) -> ResolvedTarget`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::config::TargetConfig

*Struct*

A configured login target.

**Fields:**
- `id: String` - Stable target identifier used by CLI `login TARGET_ID`.
- `account: String` - Account ID referenced from `[[accounts]]`.
- `interface: Option<String>` - Optional interface ID referenced from `[[interfaces]]`.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Clone**
  - `fn clone(self: &Self) -> TargetConfig`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::config::write_network_nas_ip

*Function*

Writes `network.nas_ip` into a TOML configuration file.

Existing comments and value decoration are preserved as much as
`toml_edit` allows. The function returns `true` only when the file content
was changed.

# Errors

Returns configuration read/edit/write errors if the file cannot be loaded,
parsed as editable TOML, or saved.

```rust
fn write_network_nas_ip<impl AsRef<Path>>(path: impl Trait, nas_ip: &str) -> crate::error::Result<bool>
```



