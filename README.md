# XJTUPortal

A small Rust CLI for automatic XJTU campus portal login.

This rewrite keeps only the unattended login flow. It uses the current
AES-encrypted portal login endpoint, supports optional automatic device logout
when the account reaches the concurrent device limit, and can run multiple
account/interface targets from one TOML file.

## Usage

```bash
cargo run -- --config ./config.toml
```

Useful flags:

```bash
xjtuportal --config ./config.toml
xjtuportal --log-level debug
xjtuportal --version
```

Copy `config.example.toml` to `config.toml` and edit the account, password, and
optional interface targets. If `[[targets]]` is omitted, the program logs in once
with `[default_account]` through the default route. If targets are configured,
they are processed in order. On Linux/OpenWrt, an interface `name` is bound with
reqwest's interface binding, which uses `SO_BINDTODEVICE`; optional `local_ip`
source binding can be configured as an additional hint.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
