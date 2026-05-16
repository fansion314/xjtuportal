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
xjtuportal login
xjtuportal list
xjtuportal logout
xjtuportal logout router
xjtuportal completions fish
xjtuportal --log-level debug
xjtuportal --version
```

Copy `config.example.toml` to `config.toml` and edit the account, password, and
optional interface targets. Without `--config`, the CLI reads `config.toml` next
to the executable file; pass `--config ./config.toml` to use a file from the
current directory. If `[[targets]]` is omitted, the program logs in once with
`[default_account]` through the default route. If targets are configured, they
are processed in order. On Linux/OpenWrt, an interface `name` is bound with
reqwest's interface binding, which uses `SO_BINDTODEVICE`; optional `local_ip`
source binding can be configured as an additional hint.

The `login`, `list`, and `logout` subcommands use only `[default_account]` and
the default route. `list` prints the current account's portal sessions and uses
`logout.known_macs` names when available. `logout <selector>` accepts either a
MAC address or a configured name. Plain `logout` first uses `logout.current_mac`;
if that is not set and only one session exists, it logs out that session. Direct
IP auto-detection is only a best effort for hosts connected directly to the
portal network; behind a router/NAT, configure `logout.current_mac` or pass a
known name explicitly.

When the unauthenticated portal probe redirects with a `nasip=...` query value,
the CLI updates `network.nas_ip` in the active config file so later online
`list`/`logout` commands can build the portal redirect URL without guessing it.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
