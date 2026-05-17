# XJTUPortal

西安交大校园网自动登录命令行工具。

它的默认目标很简单：放好配置文件后，直接运行 `xjtuportal` 就自动检查网络并登录。额外提供
`login`、`list`、`logout` 三个子命令，方便手动登录、查看已登录设备、下线设备。

## 安装后的配置位置

如果不传 `--config`，程序会按下面的顺序查找 `config.toml`：

1. 可执行文件同目录。
2. 当前工作目录。

例如你把程序放在 `/usr/local/bin/xjtuportal`，默认配置文件就是：

```text
/usr/local/bin/config.toml
```

如果可执行文件旁边没有配置，程序会继续读取你运行命令时所在目录下的 `config.toml`。开发调试时也可以显式指定配置：

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
username = "3124000000@xjtu"
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

手动执行登录：

```bash
xjtuportal login
```

查看已登录设备：

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

配置了多账号时，`login`、`list`、`logout` 会按 `[[accounts]]` 和 `[[targets]]` 执行多账号流程。临时只想使用 `[default_account]` 和默认路由时，加 `--one`：

```bash
xjtuportal --one login
xjtuportal --one list
xjtuportal --one logout router
```

多账号配置下，也可以只操作其中一部分：

```bash
# 只登录指定 target
xjtuportal login wan1-main

# 登录指定账号的全部 targets
xjtuportal login --account main

# 只查看指定账号的已登录设备
xjtuportal list main

# list 的简写
xjtuportal ls main
```

生成 shell 自动补全：

```bash
xjtuportal completions fish
```

## 关于 list/logout 的第一次使用

校园网的设备列表接口需要一个登录 token。单账号模式下程序会自动用 `[default_account]` 获取它；多账号模式下会分别用 `[[accounts]]` 中的账号获取。

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

配置了 `[[accounts]]` 后，`login`、`list`、`logout` 和不带子命令的自动登录都会进入多账号流程。`[[targets]]` 用来为账号指定网卡；某个账号没有对应 target 时，`list`/`logout` 会像单账号模式一样使用默认路由查询该账号。

需要临时绕过多账号配置时，可以加 `--one`，强制本次只使用 `[default_account]` 和默认路由。

## 验证命令

开发者修改代码后建议运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
