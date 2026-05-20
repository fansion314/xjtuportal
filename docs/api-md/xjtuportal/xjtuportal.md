**xjtuportal**

# Module: xjtuportal

## Contents

**Modules**

- [`config`](#config) - TOML configuration model and target resolution.
- [`crypto`](#crypto) - Portal request and response encryption helpers.
- [`error`](#error) - Error types shared by the CLI and library.
- [`interface`](#interface) - Local network-interface discovery helpers.
- [`portal`](#portal) - HTTP client for the campus portal v3 protocol.
- [`session`](#session) - Session response normalization and automatic logout selection.

**Structs**

- [`AccountLogout`](#accountlogout) - Result of logging out one session for an account.
- [`AccountSessions`](#accountsessions) - Sessions belonging to one account.
- [`NamedSession`](#namedsession) - Session information enriched with configured device names.

**Enums**

- [`RunStatus`](#runstatus) - Overall result of running one or more login targets.

**Functions**

- [`list_account_sessions`](#list_account_sessions) - Lists sessions for all configured accounts.
- [`list_account_sessions_for_account`](#list_account_sessions_for_account) - Lists sessions for one configured account ID.
- [`list_default_sessions`](#list_default_sessions) - Lists sessions for the simple-mode default account.
- [`logout_account_sessions`](#logout_account_sessions) - Logs out sessions matching a selector across account targets.
- [`logout_default_session`](#logout_default_session) - Logs out one session for the simple-mode default account.
- [`nas_ip_from_redirect_url`](#nas_ip_from_redirect_url) - Extracts `nasip` from a portal redirect URL.
- [`run`](#run) - Runs the automatic login flow for every resolved target in the configuration.
- [`run_account_login`](#run_account_login) - Runs login for every target belonging to one account ID.
- [`run_default_login`](#run_default_login) - Runs only the simple-mode default login target.
- [`run_target_login`](#run_target_login) - Runs the login flow for one configured target ID.

---

## xjtuportal::AccountLogout

*Struct*

Result of logging out one session for an account.

**Fields:**
- `account: String` - Account username.
- `session: NamedSession` - Session that was logged out.

**Traits:** Eq

**Trait Implementations:**

- **Clone**
  - `fn clone(self: &Self) -> AccountLogout`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **PartialEq**
  - `fn eq(self: &Self, other: &AccountLogout) -> bool`



## xjtuportal::AccountSessions

*Struct*

Sessions belonging to one account.

**Fields:**
- `account: String` - Account username.
- `sessions: Vec<NamedSession>` - Sessions currently active for the account.

**Traits:** Eq

**Trait Implementations:**

- **PartialEq**
  - `fn eq(self: &Self, other: &AccountSessions) -> bool`
- **Clone**
  - `fn clone(self: &Self) -> AccountSessions`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::NamedSession

*Struct*

Session information enriched with configured device names.

**Fields:**
- `name: String` - Configured known-MAC name, or `未知` when not configured.
- `mac: String` - Normalized MAC used for display and selection.
- `api_mac: String` - API-provided MAC preserved for logout payloads.
- `device_type: String` - Portal-reported device type.
- `user_ip: String` - Portal-reported user IP.
- `start_time: String` - Portal-reported session start time.
- `unique_id: String` - Accounting unique ID required by the logout API.

**Traits:** Eq

**Trait Implementations:**

- **PartialEq**
  - `fn eq(self: &Self, other: &NamedSession) -> bool`
- **Clone**
  - `fn clone(self: &Self) -> NamedSession`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::RunStatus

*Enum*

Overall result of running one or more login targets.

**Variants:**
- `Success` - Every requested target completed successfully.
- `PartialFailure` - At least one target failed while other target groups may have succeeded.

**Traits:** Eq, Copy

**Trait Implementations:**

- **Clone**
  - `fn clone(self: &Self) -> RunStatus`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **PartialEq**
  - `fn eq(self: &Self, other: &RunStatus) -> bool`



## Module: config

TOML configuration model and target resolution.

The CLI supports a single default account as well as advanced multi-account
and multi-interface targets. This module keeps TOML deserialization separate
from runtime resolution so the rest of the crate can work with validated
[`ResolvedTarget`] values. Configuration is TOML-only; do not reintroduce the
old YAML shape here.



## Module: crypto

Portal request and response encryption helpers.

The campus portal v3 APIs exchange AES-128-CBC encrypted, hex-encoded
payloads. The key, IV, padding, and compact JSON behavior match the Python
reverse-engineering scripts in `exp/`; changing any of those constants will
break compatibility with the verified endpoint.



## Module: error

Error types shared by the CLI and library.

This module intentionally keeps all user-facing failures in one enum so the
CLI can map configuration problems to exit code `2` while treating network,
crypto, and portal failures as runtime errors. Error messages are written in
Chinese because they are surfaced directly to command-line users.



## Module: interface

Local network-interface discovery helpers.

This module is deliberately small and platform-aware. It resolves interface
IPv4 addresses for configuration, discovers MAC addresses for redirect URL
construction, and leaves actual socket binding to `reqwest` in the protocol
client.



## xjtuportal::list_account_sessions

*Function*

Lists sessions for all configured accounts.

Accounts are processed concurrently. If at least one account succeeds, failed
accounts are logged and omitted from the result; if all fail, the first error
is returned.

# Errors

Returns configuration resolution errors before spawning tasks, or the first
account error when no account could be listed.

```rust
fn list_account_sessions(config: config::AppConfig, config_path: Option<std::path::PathBuf>) -> error::Result<Vec<AccountSessions>>
```



## xjtuportal::list_account_sessions_for_account

*Function*

Lists sessions for one configured account ID.

# Errors

Returns [`PortalError::InvalidConfig`] if the account is unknown, or session
access errors from the account flow.

```rust
fn list_account_sessions_for_account(config: config::AppConfig, config_path: Option<std::path::PathBuf>, account_id: &str) -> error::Result<AccountSessions>
```



## xjtuportal::list_default_sessions

*Function*

Lists sessions for the simple-mode default account.

# Errors

Returns configuration, login-token, request, or decryption errors.

```rust
fn list_default_sessions(config: config::AppConfig, config_path: Option<std::path::PathBuf>) -> error::Result<Vec<NamedSession>>
```



## xjtuportal::logout_account_sessions

*Function*

Logs out sessions matching a selector across account targets.

Multi-account logout requires a selector. If the selector maps to a target
interface MAC, only that target is queried. Otherwise all accounts are
searched and every matching account session is logged out.

# Errors

Returns [`PortalError::InvalidConfig`] when selector is missing, session
lookup errors when nothing matches, or runtime errors from matching accounts.

```rust
fn logout_account_sessions(config: config::AppConfig, selector: Option<&str>, config_path: Option<std::path::PathBuf>) -> error::Result<Vec<AccountLogout>>
```



## xjtuportal::logout_default_session

*Function*

Logs out one session for the simple-mode default account.

When `selector` is provided, it may be a MAC address or a name from
`logout.known_macs`. Without a selector, the function tries
`logout.current_mac`, a single active session, or the local route IP.

# Errors

Returns session selection errors when no unique candidate can be found, plus
any configuration, login-token, or request errors.

```rust
fn logout_default_session(config: config::AppConfig, selector: Option<&str>, config_path: Option<std::path::PathBuf>) -> error::Result<NamedSession>
```



## xjtuportal::nas_ip_from_redirect_url

*Function*

Extracts `nasip` from a portal redirect URL.

Returns `None` if the URL is invalid, lacks `nasip`, or has an empty value.

```rust
fn nas_ip_from_redirect_url(redirect_url: &str) -> Option<String>
```



## Module: portal

HTTP client for the campus portal v3 protocol.

This module owns the encrypted endpoint calls and the redirect probe. All
clients disable automatic redirects so `http://1.1.1.1` can reveal the portal
`Location` header when the network is captive. Login, session list, and
logout all use the verified v3 encrypted APIs.



## xjtuportal::run

*Function*

Runs the automatic login flow for every resolved target in the configuration.

Targets are grouped by username and each account group runs concurrently.
Within a group, targets run sequentially so repeated logins for the same
account do not race against portal device-limit behavior.

# Errors

Returns configuration resolution errors before any target is spawned.

```rust
fn run(config: config::AppConfig, config_path: Option<std::path::PathBuf>) -> error::Result<RunStatus>
```



## xjtuportal::run_account_login

*Function*

Runs login for every target belonging to one account ID.

# Errors

Returns [`PortalError::InvalidConfig`] if no target references `account_id`,
or runtime errors from target execution.

```rust
fn run_account_login(config: config::AppConfig, config_path: Option<std::path::PathBuf>, account_id: &str) -> error::Result<RunStatus>
```



## xjtuportal::run_default_login

*Function*

Runs only the simple-mode default login target.

This is used by `--one` and by legacy single-account operation.

# Errors

Returns configuration, network, login, session, or automatic-logout errors
from the single target flow.

```rust
fn run_default_login(config: config::AppConfig, config_path: Option<std::path::PathBuf>) -> error::Result<()>
```



## xjtuportal::run_target_login

*Function*

Runs the login flow for one configured target ID.

# Errors

Returns [`PortalError::InvalidConfig`] if `target_id` is unknown, or target
runtime errors from the login flow.

```rust
fn run_target_login(config: config::AppConfig, config_path: Option<std::path::PathBuf>, target_id: &str) -> error::Result<()>
```



## Module: session

Session response normalization and automatic logout selection.

The portal session-list API returns partially optional fields and MAC
addresses in the API's original formatting. This module converts those
records into valid [`Session`] values while preserving the original MAC text
needed by the logout endpoint.



