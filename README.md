# XJTUPortal

西安交大校园网自动登录命令行工具。

它的默认目标很简单：放好配置文件后，直接运行 `xjtuportal` 就自动检查网络并登录。额外提供
`login`、`list`、`logout` 三个子命令，方便手动登录、查看已登录设备、下线设备。

## 安装后的配置位置

如果不传 `--config`，程序会读取“可执行文件同目录”的 `config.toml`。

例如你把程序放在 `/usr/local/bin/xjtuportal`，默认配置文件就是：

```text
/usr/local/bin/config.toml
```

开发调试时如果使用 `cargo run`，可执行文件在 `target/debug/xjtuportal`，所以建议显式指定配置：

```bash
cargo run -- --config ./config.toml
```

## 最简单的个人配置

复制简单示例：

```bash
cp config.example.toml config.toml
```

然后只需要先改账号和密码：

```toml
[default_account]
username = "3124351080@xjtu"
password = "change-me"
```

`[logout]` 是下线设备相关配置。刚开始可以保持示例默认值，等你运行过 `list` 看到设备 MAC 后，再给常用设备加名字：

```toml
[logout]
enabled = true
known_macs = [
  { mac = "11:22:33:44:55:66", name = "router" },
  { mac = "aa:bb:cc:dd:ee:ff", name = "phone" },
]
```

这些名字只在本地使用，方便 `list` 显示和 `logout` 选择设备。

## 常用命令

自动登录：

```bash
xjtuportal
```

手动执行默认账号登录：

```bash
xjtuportal login
```

查看当前账号已经登录的设备：

```bash
xjtuportal list
```

下线当前设备：

```bash
xjtuportal logout
```

按名称或 MAC 下线指定设备：

```bash
xjtuportal logout router
xjtuportal logout 11:22:33:44:55:66
```

使用当前目录里的配置文件：

```bash
xjtuportal --config ./config.toml login
xjtuportal list --config ./config.toml
```

生成 shell 自动补全：

```bash
xjtuportal completions fish
```

## 关于 list/logout 的第一次使用

校园网的设备列表接口需要一个登录 token。程序会自动用默认账号获取它。

如果当前还没登录，校园网会把 `http://1.1.1.1` 重定向到网关，并在 URL 里带上 `nasip=...`。程序会自动把这个值写回当前配置文件的 `network.nas_ip`，以后已经在线时执行 `list`/`logout` 就不用猜这个地址。

如果你已经在线，但配置里还没有 `network.nas_ip`，可以先退出登录后运行一次：

```bash
xjtuportal login --config ./config.toml
```

也可以手动用下面的命令观察校园网返回的重定向：

```bash
curl -sS -D - -o /dev/null --max-time 5 --path-as-is 'http://1.1.1.1'
```

## 自动下线策略

登录时如果账号达到设备数量上限，并且 `[logout].enabled = true`，程序会自动选择一个已有设备下线，然后重试登录。

选择顺序是：

1. 优先下线不在 `logout.known_macs` 里的设备。
2. 如果所有设备都在 `known_macs` 里，就按 `known_macs` 的配置顺序选择。
3. 如果仍然无法选择，就报错，不会乱下线。

直接运行 `xjtuportal logout` 时，程序会优先使用 `logout.current_mac`。如果没有配置且只有一个在线设备，就下线那个设备；否则会尝试用本机 IP 匹配当前设备。

如果程序运行在路由器后面，校园网看到的通常是路由器 WAN 口 MAC，而不是你电脑的 MAC。这种情况下建议配置：

```toml
[logout]
current_mac = "11:22:33:44:55:66"
```

## 高级配置

需要多账号、多出口或指定网卡时，参考：

```bash
cp config.advanced.example.toml config.toml
```

高级配置里有三组核心对象：

- `[[accounts]]`：定义多个校园网账号。
- `[[interfaces]]`：定义可绑定的本机网络接口。
- `[[targets]]`：把某个账号和某个接口组合成一个登录目标。

在 Linux/OpenWrt 上，`interfaces.name` 会传给 `reqwest::ClientBuilder::interface()`，底层使用
`SO_BINDTODEVICE`。这比只绑定 `local_ip` 更可靠，尤其是 mwan3 之类策略路由环境里。

`login`、`list`、`logout` 子命令始终只使用 `[default_account]` 和默认路由；不输入子命令时的自动登录流程才会处理 `[[targets]]`。

## 验证命令

开发者修改代码后建议运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
