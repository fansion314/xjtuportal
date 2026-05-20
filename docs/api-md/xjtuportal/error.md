**xjtuportal > error**

# Module: error

## Contents

**Enums**

- [`PortalError`](#portalerror) - Top-level error type for configuration, local network, HTTP, crypto, and

**Type Aliases**

- [`Result`](#result) - Convenient result alias used throughout the crate.

---

## xjtuportal::error::PortalError

*Enum*

Top-level error type for configuration, local network, HTTP, crypto, and
portal protocol failures.

Variants are intentionally descriptive rather than deeply nested: most
errors are printed directly by the CLI, and callers usually care about the
broad failure category rather than an implementation-specific source chain.

**Variants:**
- `ConfigRead{ path: String, source: std::io::Error }` - The configuration file could not be read from disk.
- `ConfigParse{ path: String, source: toml::de::Error }` - The configuration file was not valid TOML for [`crate::config::AppConfig`].
- `ConfigEdit{ path: String, source: toml_edit::TomlError }` - The configuration file could not be parsed as editable TOML.
- `ConfigWrite{ path: String, source: std::io::Error }` - The configuration file could not be written after an automatic update.
- `InvalidConfig(String)` - Configuration is syntactically valid but semantically unusable.
- `HttpClient(reqwest::Error)` - Building a `reqwest` HTTP client failed.
- `Request(reqwest::Error)` - Sending an HTTP request or reading its body failed.
- `PortalHttpStatus{ status: reqwest::StatusCode, body: String }` - The portal returned a non-success status that could not be interpreted.
- `MissingRedirect` - A redirect response did not include a `Location` header.
- `UnsupportedNetworkStatus(reqwest::StatusCode)` - The redirect probe returned a status outside the supported cases.
- `UrlParse{ url: String, source: url::ParseError }` - A configured or portal-provided URL could not be parsed.
- `Json(serde_json::Error)` - JSON serialization or deserialization failed.
- `Hex(hex::FromHexError)` - Hex decoding failed for an encrypted portal body.
- `Decrypt` - AES-CBC decryption or UTF-8 conversion failed.
- `Encrypt` - AES-CBC encryption failed.
- `InvalidHeaderValue(String)` - An HTTP header value contained invalid bytes.
- `InterfaceAddressMissing{ name: String }` - A named local interface did not have a usable IPv4 address.
- `InterfaceInspect(std::io::Error)` - Inspecting local network interfaces failed.
- `LoginRejected{ code: Option<i64>, error: Option<i64>, description: String }` - The portal rejected a login attempt.
- `DeviceLimitReached` - Login reached the device limit and automatic logout was disabled.
- `NoLogoutCandidate` - No valid session was available for automatic logout.
- `StillOverloaded(String)` - Login still reported device-limit after one automatic logout and retry.
- `MissingToken` - A login response did not include the token needed by session APIs.
- `InvalidMac(String)` - A MAC address could not be normalized.
- `InvalidLocalIp(std::net::IpAddr)` - A configured local IP is not a usable IPv4 address.
- `GatewayResolve(String)` - The configured gateway could not be resolved for route probing.
- `CurrentSessionNotFound(String)` - The CLI could not identify the current device in the session list.
- `SessionNotFound(String)` - No session matched a user-provided selector.
- `AmbiguousSessionName(String)` - A user-provided known-MAC name matched more than one configured MAC.
- `TaskJoin(String)` - A spawned async task failed to join.

**Trait Implementations:**

- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`
- **From**
  - `fn from(source: std::io::Error) -> Self`
- **From**
  - `fn from(source: hex::FromHexError) -> Self`
- **From**
  - `fn from(source: serde_json::Error) -> Self`
- **From**
  - `fn from(source: reqwest::Error) -> Self`
- **Display**
  - `fn fmt(self: &Self, __formatter: & mut ::core::fmt::Formatter) -> ::core::fmt::Result`
- **Error**
  - `fn source(self: &Self) -> ::core::option::Option<&dyn ::thiserror::__private18::Error>`



## xjtuportal::error::Result

*Type Alias*: `std::result::Result<T, PortalError>`

Convenient result alias used throughout the crate.

All fallible public APIs return [`PortalError`] through this alias so callers
can match a single error type.



