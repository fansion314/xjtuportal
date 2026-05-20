**xjtuportal > portal**

# Module: portal

## Contents

**Structs**

- [`CampusClient`](#campusclient) - Portal HTTP client bound to a network configuration and optional interface.
- [`LoginResponse`](#loginresponse) - Decrypted v3 login response.

**Enums**

- [`LoginStatus`](#loginstatus) - Classified portal login response.
- [`NetworkStatus`](#networkstatus) - Result of the captive-portal network probe.

**Functions**

- [`classify_login_response`](#classify_login_response) - Classifies a decrypted login response into success, device-limit, or failure.
- [`fix_redirect_url`](#fix_redirect_url) - Ensures a redirect URL has a path component acceptable to the portal.

---

## xjtuportal::portal::CampusClient

*Struct*

Portal HTTP client bound to a network configuration and optional interface.

**Methods:**

- `fn new(network: NetworkConfig, binding: NetworkBinding) -> Result<Self>` - Builds a portal client for a network binding.
- `fn check_network(self: &Self) -> Result<NetworkStatus>` - Checks whether the current binding is already online or captive-redirected.
- `fn login(self: &Self, account: &AccountConfig, redirect_url: &str) -> Result<LoginStatus>` - Attempts portal login for an account using a redirect URL.
- `fn list_sessions(self: &Self, token: &str) -> Result<Vec<Session>>` - Lists active sessions using a token returned by a login response.
- `fn logout_session(self: &Self, token: &str, unique_id: &str, mac: &str) -> Result<()>` - Logs out a single active session by accounting unique ID and API MAC.

**Trait Implementations:**

- **Clone**
  - `fn clone(self: &Self) -> CampusClient`



## xjtuportal::portal::LoginResponse

*Struct*

Decrypted v3 login response.

**Fields:**
- `code: Option<i64>` - Optional status code field observed in some responses.
- `error: Option<i64>` - Optional error field observed in other responses.
- `error_description: Option<String>` - Optional human-readable error description.
- `token: Option<String>` - Optional token used by session APIs.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::portal::LoginStatus

*Enum*

Classified portal login response.

**Variants:**
- `Success{ token: Option<String> }` - Login succeeded. A token may be present for subsequent session APIs.
- `Overloaded{ description: String, token: Option<String> }` - Login reached the device limit. Some responses include a session API token.
- `Failed{ code: Option<i64>, error: Option<i64>, description: String }` - Login failed for reasons other than device limit.

**Traits:** Eq

**Trait Implementations:**

- **Clone**
  - `fn clone(self: &Self) -> LoginStatus`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **PartialEq**
  - `fn eq(self: &Self, other: &LoginStatus) -> bool`



## xjtuportal::portal::NetworkStatus

*Enum*

Result of the captive-portal network probe.

**Variants:**
- `Online` - The test URL succeeded, so no login is needed.
- `Redirected(String)` - The test URL redirected to a portal login URL.

**Traits:** Eq

**Trait Implementations:**

- **PartialEq**
  - `fn eq(self: &Self, other: &NetworkStatus) -> bool`
- **Clone**
  - `fn clone(self: &Self) -> NetworkStatus`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::portal::classify_login_response

*Function*

Classifies a decrypted login response into success, device-limit, or failure.

The portal has been observed using either `code` or `error`; both are checked.

```rust
fn classify_login_response(response: LoginResponse) -> LoginStatus
```



## xjtuportal::portal::fix_redirect_url

*Function*

Ensures a redirect URL has a path component acceptable to the portal.

# Errors

Returns [`PortalError::UrlParse`] if `raw_url` is not a valid URL.

```rust
fn fix_redirect_url(raw_url: &str) -> crate::error::Result<String>
```



