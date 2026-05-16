# AGENTS.md

Guidance for future agents working in this repository.

## Project Shape

- This project is now a Rust CLI. Do not restore the old Go/YAML implementation or interactive shell UI.
- Keep the CLI focused on unattended campus portal login. The default behavior is to run the automatic login flow directly.
- Configuration is TOML-only. Use `config.example.toml` as the public shape and keep runtime URLs in `[network]`.

## Portal Protocol

- Login uses the v3 encrypted endpoint:
  `POST /portal-conversion/api/v3/portal/connect`.
- Redirect probing must use plain HTTP `http://1.1.1.1` with redirects disabled. Do not switch this to HTTPS or a domain.
- Request/response crypto follows the Python reverse-engineering scripts in `exp/`: AES-128-CBC, key and IV both `1234567890000000`, PKCS#7 padding, hex-encoded bodies, compact JSON.
- Session list and automatic logout must use the verified v3 encrypted APIs:
  - `POST /portal-conversion/api/v3/session/list` with an empty body, encrypted response.
  - `POST /portal-conversion/api/v3/session/acctUniqueId` with encrypted `acctUniqueId` and `mac`.
- Do not use the old Go-era v2 session/token/logout endpoints; they were verified as unavailable.
- The token for session APIs comes from the login response that reports the device-limit condition. If that response lacks a token, fail clearly instead of falling back to old token APIs.

## Interface Binding

- For configured interfaces, use `reqwest::ClientBuilder::interface(name)` so Linux/OpenWrt uses `SO_BINDTODEVICE`.
- Do not rely on source IP binding alone. On OpenWrt with mwan3 policy routing, binding only `local_ip` can still route through the wrong WAN.
- Keep optional `local_ip` as an additional source-address hint, not as the primary interface selection mechanism.

## Automatic Logout

- Preserve the logout candidate strategy:
  first choose a session MAC not in `logout.known_macs`; if all are known, choose by `known_macs` order; otherwise fall back to the first valid session.
- Keep both normalized MACs for local comparison and the API-returned MAC string for the logout payload. The portal may expect the original MAC format.

## Validation

Before considering code changes done, run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

For release metadata changes, update both `Cargo.toml` and the root `xjtuportal` package entry in `Cargo.lock`.
