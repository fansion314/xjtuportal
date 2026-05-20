**xjtuportal > session**

# Module: session

## Contents

**Structs**

- [`PortalSession`](#portalsession) - Raw session record returned by the portal.
- [`Session`](#session) - A validated portal session ready for display or logout.
- [`SessionListResponse`](#sessionlistresponse) - Raw response returned by the encrypted v3 session-list endpoint.

**Functions**

- [`choose_logout_mac`](#choose_logout_mac) - Chooses which MAC address to log out when an account is over the device limit.
- [`normalize_mac`](#normalize_mac) - Normalizes a MAC address to lowercase colon-separated form.

---

## xjtuportal::session::PortalSession

*Struct*

Raw session record returned by the portal.

Field names intentionally follow the portal's mixed naming style so serde can
deserialize the encrypted JSON response without an intermediate map.

**Fields:**
- `device_type: Option<String>` - Raw `deviceType` field.
- `framed_ip_address: Option<String>` - Raw `framed_ip_address` field.
- `calling_station_id: Option<String>` - Raw `calling_station_id` MAC field.
- `acct_start_time: Option<String>` - Raw session start time field.
- `acct_unique_id: Option<String>` - Raw accounting unique ID field.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::session::Session

*Struct*

A validated portal session ready for display or logout.

**Fields:**
- `mac: String` - Normalized MAC address used for local comparison.
- `api_mac: String` - API-provided MAC string preserved for logout payloads.
- `device_type: String` - Portal-reported device type.
- `user_ip: String` - Portal-reported user IP.
- `start_time: String` - Portal-reported accounting start time.
- `unique_id: String` - Portal accounting unique ID required by the logout API.

**Traits:** Eq

**Trait Implementations:**

- **PartialEq**
  - `fn eq(self: &Self, other: &Session) -> bool`
- **Clone**
  - `fn clone(self: &Self) -> Session`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::session::SessionListResponse

*Struct*

Raw response returned by the encrypted v3 session-list endpoint.

**Fields:**
- `concurrency: serde_json::Value` - Portal concurrency metadata. It is currently not needed by the CLI.
- `sessions: Vec<PortalSession>` - Raw session records.

**Methods:**

- `fn into_sessions(self: Self) -> Vec<Session>` - Converts the raw response into valid, deduplicated sessions.

**Trait Implementations:**

- **Deserialize**
  - `fn deserialize<__D>(__deserializer: __D) -> _serde::__private228::Result<Self, <__D as >::Error>`
- **Debug**
  - `fn fmt(self: &Self, f: & mut $crate::fmt::Formatter) -> $crate::fmt::Result`



## xjtuportal::session::choose_logout_mac

*Function*

Chooses which MAC address to log out when an account is over the device limit.

The strategy is:

1. Prefer the first active session not listed in `known_macs`.
2. If every active session is known, choose by `known_macs` order.
3. If no configured known MAC matches, fall back to the first valid session.

Invalid MAC strings are ignored.

```rust
fn choose_logout_mac<K>(session_macs: &[&str], known_macs: &[K]) -> Option<String>
```



## xjtuportal::session::normalize_mac

*Function*

Normalizes a MAC address to lowercase colon-separated form.

Accepts one- or two-digit hex octets separated by `:` or `-` and returns a
six-octet `aa:bb:cc:dd:ee:ff` string.

# Errors

Returns [`PortalError::InvalidMac`] if the input is not six hexadecimal
octets.

```rust
fn normalize_mac(mac: &str) -> crate::error::Result<String>
```



