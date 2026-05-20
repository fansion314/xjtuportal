# ARCHITECTURE.md

## 1. Project Overview

`xjtuportal` 是一个面向西安交通大学校园网的 Rust CLI，用于在无人值守环境中完成校园网登录、会话查询和自动下线。

核心使用场景：

- 在 OpenWrt、Linux 主机或普通桌面环境中定时运行，自动保持校园网在线。
- 多账号、多网卡环境下，为每个账号或出口接口执行独立登录。
- 当账号达到设备数量上限时，按配置策略自动下线一个已有设备后重试登录。
- 手动查看或下线当前账号的在线设备。

非目标：

- 不恢复旧的 Go/YAML 实现。
- 不提供交互式 shell UI。
- 不实现通用 captive portal 框架；当前协议是面向 XJTU 校园网 v3 接口的专用实现。
- 不用旧 v2 session/token/logout API 作为 fallback。
- 不把配置格式扩展为 JSON/YAML/环境变量优先的复杂系统；配置是 TOML-only。

当前架构的主要取舍：默认无子命令自动登录；协议细节集中在 `portal`/`crypto`；运行编排集中在 `lib.rs`；多账号并发、同账号顺序执行；`network.nas_ip` 可从 redirect 自动写回。

## 2. High-Level Architecture

整体分为四层：

```text
CLI args / stdout
        |
        v
Application orchestration (login/list/logout flows)
        |
        v
Configuration, session selection, interface discovery
        |
        v
Portal HTTP client + AES-CBC crypto
        |
        v
XJTU campus portal v3 endpoints
```

入口是 `src/main.rs`。它解析命令行、读取配置、选择要执行的库函数，然后把结果映射为 stdout 和进程退出码。

核心应用层在 `src/lib.rs`。这里负责把 `AppConfig` 解析成目标、创建 `CampusClient`、检查网络状态、登录、获取 session token、列出会话、选择下线候选并重试登录。

协议层在 `src/portal.rs` 和 `src/crypto.rs`。`portal.rs` 只负责 HTTP 请求、header、redirect 探测和响应分类；`crypto.rs` 只负责 v3 接口要求的 AES-128-CBC/PKCS#7/hex/compact JSON。

辅助能力：`config.rs` 负责 TOML 和 target 解析；`session.rs` 负责 session 清洗、MAC 和候选策略；`interface.rs` 查询本机接口；`error.rs` 定义统一错误。

## 3. Code Map

改 CLI 参数或子命令：从 `src/main.rs` 开始。`Args` 和 `Command` 定义 clap 参数结构；`main()` 中的 `match args.command` 决定 CLI 到库函数的映射；stdout 表格输出也在这里。

改配置格式：从 `src/config.rs` 开始。`AppConfig` 是根 TOML 模型；`NetworkConfig` 存放运行时 URL、timeout、`nas_ip`；高级配置由 `AccountConfig`、`InterfaceConfig`、`TargetConfig` 组成。同步更新两个 example TOML。

改登录/list/logout 主流程：从 `src/lib.rs` 开始。公共入口包括 `run`、`run_*_login`、`list_*`、`logout_*`。单目标核心路径是 `run_target -> login_with_optional_logout -> logout_one_and_retry`；session token 路径是 `session_client_for_target -> login_for_session_token`。

改网络请求逻辑：从 `src/portal.rs` 开始。`CampusClient::new` 负责 reqwest client、header、timeout、redirect policy、interface/local IP binding；`check_network` 做 captive redirect 探测；`login`、`list_sessions`、`logout_session` 对应三个 v3 API。

改加密协议：从 `src/crypto.rs` 开始。`encrypt_json` / `decrypt_json` 是协议层主要 API；key、IV、padding、hex、compact JSON 必须和 `exp/` 中的 Python 逆向脚本保持一致。

改 session 解析或自动下线策略：从 `src/session.rs` 开始。`SessionListResponse::into_sessions` 决定有效 session；`normalize_mac` 决定本地比较格式；`choose_logout_mac` 决定自动下线候选顺序。

改错误处理：从 `src/error.rs` 开始。新错误应加入 `PortalError`，并考虑 `src/main.rs` 的退出码映射。配置错误通常是 exit code `2`，运行时错误是 `1`。

新增一种后端、平台或协议：

- 如果是同一 portal 的 URL/header 变化，优先改 `NetworkConfig` 和 `CampusClient`。
- 如果是不同加密或不同接口形状，先抽出 `portal.rs` 中的协议边界，再考虑 trait。
- 如果是不同 OS 的接口发现，优先扩展 `interface.rs`，不要把平台分支散落到业务流程。

## 4. Main Execution Flow

最典型的无子命令运行路径：

1. `main.rs` 解析 CLI 参数，定位并读取 `config.toml`。
2. `run(config, Some(config_path))` 进入库层。
3. `AppConfig::resolved_targets` 解析 targets；没有 targets 时使用 `[default_account]`。
4. `group_targets_by_username` 按账号分组，不同账号并发、同账号顺序。
5. `run_target` 构造 `NetworkBinding` 和 `CampusClient`。
6. `CampusClient::check_network` 请求 `network.test_url`，禁用重定向。
7. 已在线则完成；收到 portal redirect 则提取 `nasip` 并异步写回配置。
8. `CampusClient::login` 加密登录请求并分类响应。
9. 成功则完成；设备数超限且允许自动下线时，用 token 请求 session list。
10. `choose_logout_mac` 选择设备，`logout_session` 下线，然后重试 login。
11. 所有任务结束后等待配置写回，`RunStatus` 映射为进程退出码。

## 5. Architectural Boundaries

`main.rs` 不应该知道 portal 协议细节。它只负责 CLI、配置文件路径、stdout 和 exit code。

`portal.rs` 不应该读取 TOML，也不应该选择自动下线候选。它只做 HTTP 协议交换和响应分类。

`crypto.rs` 不应该依赖配置、网络或 session 类型。它是纯协议加解密工具。

`config.rs` 不应该发起网络请求，除了通过本机接口查询解析 `InterfaceConfig::local_ip`。

`session.rs` 不应该知道 HTTP endpoint、token 或配置文件路径。它只处理 session 数据、MAC 规范化和候选选择。

`interface.rs` 不应该构造 reqwest client。实际绑定由 `CampusClient::new` 使用 `NetworkBinding` 完成。

错误类型可以被所有模块依赖，但 `error.rs` 不应该反向依赖具体业务模块。

## 6. Architectural Invariants

- 项目是 Rust CLI；不要恢复 Go/YAML 代码路径或交互式 shell UI。
- 配置是 TOML-only。
- 运行时 URL 属于 `[network]`。
- 默认运行不带子命令时执行自动登录。
- redirect 探测必须使用 plain HTTP `http://1.1.1.1`，并且 HTTP client 禁用自动重定向。
- 登录必须使用 v3 encrypted endpoint：`POST /portal-conversion/api/v3/portal/connect`。
- session list 必须使用 v3 encrypted endpoint：`POST /portal-conversion/api/v3/session/list`，空 body。
- session logout 必须使用 v3 encrypted endpoint：`POST /portal-conversion/api/v3/session/acctUniqueId`。
- 不要重新引入旧 v2 session/token/logout endpoint。
- session API token 来自登录响应，尤其是设备数超限响应；缺 token 时应清晰失败。
- AES 参数必须是 AES-128-CBC，key 和 IV 都是 `1234567890000000`，PKCS#7 padding，hex body，compact JSON。
- 配置了 interface name 时，HTTP client 必须用 `reqwest::ClientBuilder::interface(name)`。
- `local_ip` 只是附加 source-address hint，不能替代 interface binding。
- 自动下线策略必须保持：先选不在 known_macs 的 session；全都已知时按 known_macs 顺序；否则 fallback 到第一个有效 session。
- 本地比较使用 normalized MAC，logout payload 使用 API 返回的原始 MAC 字符串。

## 7. Cross-Cutting Concerns

错误和退出码：配置错误映射为 exit code `2`；网络、HTTP、portal、crypto、session 选择失败通常映射为 `1`。

日志：使用 `tracing`；token 只打印短前缀；多目标日志应包含 target、account、interface、binding 信息。

配置写回：`network.nas_ip` 写回由 `ConfigUpdateWriter` 控制；一次运行最多启动一个写回任务；使用 `toml_edit` 保留注释和排版。

MAC 处理：本地比较必须 normalize；redirect URL 使用 dash-separated MAC；logout payload 使用 portal API 返回的原始 MAC。

并发：不同账号可以并发；同一账号 target 顺序执行；list/logout 跨账号允许部分成功，但全部失败时要返回错误。

## 8. Extension Points

新增 CLI 功能：

- 在 `Command` 增加子命令。
- 在 `main()` 中映射到新的库函数。
- 尽量不要把业务逻辑写进 `main.rs`。

新增配置项：

- 加到 `config.rs` 对应结构。
- 给出 serde default，避免破坏旧配置。
- 更新 example TOML 和相关测试。

新增 portal API：

- 在 `portal.rs` 为 HTTP exchange 增加方法。
- 请求/响应加解密复用 `crypto.rs`。
- 响应数据清洗可放在独立模块，避免 `portal.rs` 膨胀。

新增 session 选择策略：

- 从 `session.rs` 或 `lib.rs` 的 selector helper 开始。
- 保持 API MAC 和 normalized MAC 分离。
- 为候选优先级添加单元测试。

新增平台网络能力：

- 优先扩展 `interface.rs` 的查询能力。
- 若涉及实际 socket/client 绑定，修改 `CampusClient::new` 或 `bind_interface`。

## 9. Important Types and Concepts

`AppConfig`：根配置对象，来自 TOML。

`ResolvedTarget`：已经把 account/interface 引用解析完成的运行目标。

`NetworkBinding`：传给 HTTP client 的接口名和可选 local IP。

`CampusClient`：绑定了网络配置和 reqwest client 的 portal 协议客户端。

`NetworkStatus`：网络探测结果，表示已在线或被 portal redirect。

`LoginStatus`：登录响应分类，包含成功、设备数超限、失败。

`Session`：清洗后的有效会话，含 normalized MAC 和 API 原始 MAC。

`NamedSession`：给 CLI 展示使用的 session，叠加 known_macs 中的设备名。

`KnownMacConfig`：用户配置的已知设备，可用于展示名称和自动下线优先级。

`ConfigUpdateWriter`：内部 helper，用于从 redirect 捕获 NAS IP 后安全写回 TOML。

## 10. External Dependencies

`reqwest`：HTTP client，负责 timeout、redirect policy、interface binding、local address。

`tokio`：异步 runtime、任务并发、异步文件写回。

`clap` / `clap_complete`：CLI 参数解析和 shell completions。

`serde` / `serde_json` / `toml` / `toml_edit`：配置、JSON 协议体和保留注释的 TOML 写回。

`aes` / `cbc` / `hex`：portal v3 加解密协议。

`get_if_addrs`：本机接口 IP 枚举。

`url`：redirect URL、gateway base 和 query 参数处理。

`tracing` / `tracing-subscriber`：结构化日志。

`thiserror`：统一错误类型定义。

## 11. Repository Layout

```text
.
├── src/
│   ├── main.rs        # CLI 入口、参数、输出、退出码
│   ├── lib.rs         # 登录/list/logout 编排
│   ├── config.rs      # TOML 配置模型和 target 解析
│   ├── portal.rs      # v3 portal HTTP client
│   ├── crypto.rs      # AES-CBC/hex/JSON 加解密
│   ├── session.rs     # session 清洗、MAC、下线候选
│   ├── interface.rs   # 本机接口 IP/MAC 查询
│   └── error.rs       # 统一错误类型
├── tests/             # 集成测试，覆盖主要运行流
├── exp/               # Python 逆向/验证脚本，作为协议参考
├── config.example.toml
├── config.advanced.example.toml
├── Cargo.toml
└── Cargo.lock
```

## 12. Design Decisions

保留 `lib.rs` 作为编排中心。当前项目规模下，它让一次登录流的控制逻辑集中可见；协议、配置、session 和接口能力已经拆到独立模块。

不为 portal API 过早抽象 trait。当前只有一个后端协议，直接的 `CampusClient` 更容易审计请求细节。等出现第二个真实协议时再抽象。

`network.nas_ip` 自动写回配置。session API 在已在线状态下需要 NAS IP；自动捕获能降低无人值守部署成本。

用 `toml_edit` 写回配置。自动更新不能破坏用户手写注释，这是配置文件长期可维护性的关键。

多目标按 username 分组。不同账号并发有收益；同账号并发会增加设备数限制和自动下线竞态。

MAC 双轨保存。normalized MAC 用于本地比较；API MAC 原样用于 logout，这是兼容 portal 行为的安全选择。

## 13. Things That Are Intentionally Not Documented Here

- 每个函数的完整 API 文档；这些写在 rustdoc 注释中。
- portal 响应样本和逆向过程；参考 `exp/` 和相关测试。
- 用户安装、部署和配置教程；这些应放在 `README.md` 或单独 docs。
- 完整依赖清单；以 `Cargo.toml` 为准。
- 发布流程、版本策略和 changelog。

## 14. Maintenance Notes

当以下情况发生时，应更新本文档：

- 新增、删除或重命名 `src/` 中的核心模块。
- CLI 入口、配置格式或主执行流发生变化。
- portal endpoint、加密协议、session token 来源发生变化。
- 自动下线策略、MAC 处理规则或 interface binding 规则发生变化。
- 引入第二种后端协议、平台抽象或明显的新架构层。

本文档应保持在架构导航层级，不要扩展成 API 手册。具体函数行为优先写进 rustdoc，用户教程优先写进 README 或 `docs/`。
