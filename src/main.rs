//! Command-line interface for the unattended campus portal login tool.
//!
//! The binary keeps user interaction intentionally minimal: without a subcommand
//! it runs the automatic login flow directly. Subcommands expose targeted login,
//! session listing, logout, and shell completion generation while delegating all
//! portal behavior to the library crate.

use std::{path::PathBuf, process::ExitCode};

use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use tracing_subscriber::EnvFilter;
use xjtuportal::{
    AccountSessions, NamedSession, RunStatus, config::AppConfig, error::PortalError,
    list_account_sessions, list_account_sessions_for_account, list_default_sessions,
    logout_account_sessions, logout_default_session, run, run_account_login, run_default_login,
    run_target_login,
};

/// Help template for the root command.
const ROOT_HELP_TEMPLATE: &str = "\
西安交大校园网自动登录工具

用法: {usage}
不输入任何子命令时，会直接执行全自动登录流程；也可以使用 login/list/logout 管理登录设备。

{all-args}{after-help}";

/// Help template shared by subcommands.
const COMMAND_HELP_TEMPLATE: &str = "\
{about-with-newline}
用法: {usage}

{all-args}{after-help}";

/// Parsed root CLI arguments.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "西安交大校园网自动登录工具",
    long_about = "西安交大校园网自动登录工具。不输入任何子命令时，会直接执行全自动登录流程；也可以使用 login/list/logout 管理登录设备。",
    disable_help_flag = true,
    disable_help_subcommand = true,
    disable_version_flag = true,
    propagate_version = true,
    help_template = ROOT_HELP_TEMPLATE,
    next_help_heading = "选项",
    subcommand_help_heading = "命令"
)]
struct Args {
    /// Optional path to `config.toml`.
    #[arg(short, long, value_name = "FILE", global = true, help = "配置文件路径")]
    config: Option<PathBuf>,
    /// Tracing log filter level.
    #[arg(
        long,
        default_value = "info",
        global = true,
        help = "日志级别，例如 error、warn、info、debug"
    )]
    log_level: String,
    /// Forces legacy simple-mode behavior even when advanced targets exist.
    #[arg(
        long,
        global = true,
        help = "即使配置了多个 targets，也只使用 [default_account] 按单目标模式执行"
    )]
    one: bool,
    /// Explicit help flag because the default clap help flag is customized.
    #[arg(
        short = 'h',
        long = "help",
        global = true,
        action = ArgAction::Help,
        help = "显示帮助信息"
    )]
    help: Option<bool>,
    /// Explicit version flag because the default clap version flag is customized.
    #[arg(
        short = 'V',
        long = "version",
        global = true,
        action = ArgAction::Version,
        help = "显示版本信息"
    )]
    version: Option<bool>,
    /// Optional command; absence means run automatic login.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Supported CLI subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// 执行自动登录；多目标配置会登录所有 targets。
    #[command(help_template = COMMAND_HELP_TEMPLATE)]
    Login {
        /// 只登录指定 target ID。
        #[arg(value_name = "TARGET_ID", help_heading = "参数")]
        target_id: Option<String>,
        /// 只登录指定账号 ID 的全部 targets。
        #[arg(long, value_name = "ACCOUNT_ID", conflicts_with = "target_id")]
        account: Option<String>,
    },
    /// 列出当前已经登录的设备；多目标配置会按账号分组展示。
    #[command(alias = "ls", help_template = COMMAND_HELP_TEMPLATE)]
    List {
        /// 只查看指定账号 ID 的已登录设备。
        #[arg(value_name = "ACCOUNT_ID", help_heading = "参数")]
        account_id: Option<String>,
    },
    /// 下线当前设备，或下线指定 MAC/名称对应的设备；多目标配置需要指定 MAC/名称。
    #[command(help_template = COMMAND_HELP_TEMPLATE)]
    Logout {
        /// MAC 地址，或 logout.known_macs 中配置的名称。
        #[arg(help_heading = "参数")]
        selector: Option<String>,
    },
    /// 生成 shell 自动补全脚本。
    #[command(help_template = COMMAND_HELP_TEMPLATE)]
    Completions {
        #[arg(
            value_enum,
            value_name = "SHELL",
            help = "要生成补全脚本的 shell 类型",
            help_heading = "参数"
        )]
        shell: Shell,
    },
}

/// Program entry point.
///
/// Returns process exit code `0` for success, `1` for runtime failures or
/// partial multi-target failures, and `2` for invalid configuration.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // 实现说明：current_thread runtime 足够支撑 reqwest async I/O，同时适合小型 CLI
    // 和 OpenWrt 等资源有限环境。
    let args = Args::parse();
    if let Some(Command::Completions { shell }) = &args.command {
        let mut command = Args::command();
        generate(*shell, &mut command, "xjtuportal", &mut std::io::stdout());
        return ExitCode::SUCCESS;
    }

    init_logging(&args.log_level);

    let config_path = match args.config {
        Some(path) => path,
        None => match default_config_path() {
            Ok(path) => path,
            Err(err) => {
                tracing::error!("无法确定默认配置文件路径: {err}");
                return ExitCode::from(2);
            }
        },
    };

    let config = match AppConfig::read(&config_path) {
        Ok(config) => config,
        Err(err @ PortalError::ConfigRead { .. })
        | Err(err @ PortalError::ConfigParse { .. })
        | Err(err @ PortalError::InvalidConfig(_)) => {
            tracing::error!("{err}");
            return ExitCode::from(2);
        }
        Err(err) => {
            tracing::error!("{err}");
            return ExitCode::from(1);
        }
    };

    let multi_account_mode = !args.one && !config.accounts.is_empty();
    let result = match args.command {
        None if args.one => run_default_login(config, Some(config_path))
            .await
            .map(|()| None),
        None => run(config, Some(config_path)).await.map(run_exit_code),
        Some(Command::Login {
            target_id: Some(target_id),
            account: None,
        }) => run_target_login(config, Some(config_path), &target_id)
            .await
            .map(|()| None),
        Some(Command::Login {
            target_id: None,
            account: Some(account),
        }) => run_account_login(config, Some(config_path), &account)
            .await
            .map(run_exit_code),
        Some(Command::Login {
            target_id: None,
            account: None,
        }) if args.one => run_default_login(config, Some(config_path))
            .await
            .map(|()| None),
        Some(Command::Login {
            target_id: None,
            account: None,
        }) => run(config, Some(config_path)).await.map(run_exit_code),
        Some(Command::List {
            account_id: Some(account_id),
        }) => list_account_sessions_for_account(config, Some(config_path), &account_id)
            .await
            .map(|group| {
                print_account_sessions(&[group]);
                None
            }),
        Some(Command::List { account_id: None }) if multi_account_mode => {
            list_account_sessions(config, Some(config_path))
                .await
                .map(|groups| {
                    print_account_sessions(&groups);
                    None
                })
        }
        Some(Command::List { account_id: None }) => {
            list_default_sessions(config, Some(config_path))
                .await
                .map(|sessions| {
                    print_sessions(&sessions);
                    None
                })
        }
        Some(Command::Login { .. }) => unreachable!(),
        Some(Command::Logout { selector }) if multi_account_mode => {
            logout_account_sessions(config, selector.as_deref(), Some(config_path))
                .await
                .map(|sessions| {
                    for session in sessions {
                        println!(
                            "已下线 {} 的 {} ({})",
                            session.account,
                            session.session.mac,
                            display_name(&session.session.name)
                        );
                    }
                    None
                })
        }
        Some(Command::Logout { selector }) => {
            logout_default_session(config, selector.as_deref(), Some(config_path))
                .await
                .map(|session| {
                    println!("已下线 {} ({})", session.mac, display_name(&session.name));
                    None
                })
        }
        Some(Command::Completions { .. }) => unreachable!(),
    };

    match result {
        Ok(Some(code)) => code,
        Ok(None) => ExitCode::SUCCESS,
        Err(err) => exit_code_for_error(err),
    }
}

/// Converts a multi-target run status into a process exit code.
fn run_exit_code(status: RunStatus) -> Option<ExitCode> {
    // 实现说明：完全成功用 None 表示继续走 ExitCode::SUCCESS；部分失败映射为 1。
    match status {
        RunStatus::Success => None,
        RunStatus::PartialFailure => Some(ExitCode::from(1)),
    }
}

/// Finds the default configuration path.
///
/// The executable directory is checked first, then the current working
/// directory.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::NotFound`] if neither location contains
/// `config.toml`, or propagates filesystem inspection errors.
fn default_config_path() -> std::io::Result<PathBuf> {
    // 实现说明：先找程序同目录适合部署成单文件服务；再找当前目录适合开发和手动运行。
    let executable = std::env::current_exe()?;
    let executable_config = executable
        .parent()
        .map(|directory| directory.join("config.toml"));
    if let Some(path) = executable_config
        && path.try_exists()?
    {
        return Ok(path);
    }

    let current_config = std::env::current_dir()?.join("config.toml");
    if current_config.try_exists()? {
        return Ok(current_config);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "程序同目录和当前执行目录都没有 config.toml",
    ))
}

/// Prints grouped account sessions to stdout.
fn print_account_sessions(groups: &[AccountSessions]) {
    // 实现说明：账号之间空一行，单账号内部复用 print_sessions 表格格式。
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("账号: {}", group.account);
        print_sessions(&group.sessions);
    }
}

/// Prints a session table to stdout.
fn print_sessions(sessions: &[NamedSession]) {
    // 实现说明：手写 display width 是为了让中文表头/名称和 ASCII 列在终端中对齐。
    println!(
        "{}  {}  {}  {}  登录时间",
        pad_display("名称", 9),
        pad_display("MAC", 18),
        pad_display("IP", 15),
        pad_display("设备", 8),
    );
    for session in sessions {
        println!(
            "{}  {}  {}  {}  {}",
            pad_display(display_name(&session.name), 9),
            pad_display(&session.mac, 18),
            pad_display(display_value(&session.user_ip), 15),
            pad_display(display_value(&session.device_type), 8),
            display_value(&session.start_time),
        );
    }
}

/// Pads a string to a display width using spaces.
fn pad_display(value: &str, width: usize) -> String {
    // 实现说明：width 是终端列宽，不是字节数或 char 数；中文宽字符按 2 列计算。
    let display_width = terminal_display_width(value);
    let padding = width.saturating_sub(display_width);
    format!("{value}{}", " ".repeat(padding))
}

/// Calculates terminal display width for a string.
fn terminal_display_width(value: &str) -> usize {
    // 实现说明：逐字符相加即可满足当前表格需求，不引入额外 Unicode width 依赖。
    value.chars().map(char_display_width).sum()
}

/// Estimates terminal display width for one character.
fn char_display_width(value: char) -> usize {
    // 实现说明：覆盖常见 CJK 宽字符区间；控制字符按 0，其他字符按 1。
    match value {
        '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}' => 0,
        '\u{1100}'..='\u{115f}'
        | '\u{2329}'..='\u{232a}'
        | '\u{2e80}'..='\u{a4cf}'
        | '\u{ac00}'..='\u{d7a3}'
        | '\u{f900}'..='\u{faff}'
        | '\u{fe10}'..='\u{fe19}'
        | '\u{fe30}'..='\u{fe6f}'
        | '\u{ff00}'..='\u{ff60}'
        | '\u{ffe0}'..='\u{ffe6}'
        | '\u{20000}'..='\u{3fffd}' => 2,
        _ => 1,
    }
}

/// Returns a display name, falling back to `未知`.
fn display_name(name: &str) -> &str {
    // 实现说明：空名称只影响展示，不改变底层 session 数据。
    if name.is_empty() { "未知" } else { name }
}

/// Returns a display value, falling back to `-`.
fn display_value(value: &str) -> &str {
    // 实现说明：表格中用短占位符表示 portal 未返回字段。
    if value.is_empty() { "-" } else { value }
}

/// Maps an error to the CLI exit code and logs it.
fn exit_code_for_error(err: PortalError) -> ExitCode {
    // 实现说明：配置问题返回 2，便于脚本区分“需要用户修配置”和“运行时网络/门户失败”。
    match err {
        PortalError::InvalidConfig(err) => {
            tracing::error!("配置无效: {err}");
            ExitCode::from(2)
        }
        err => {
            tracing::error!("{err}");
            ExitCode::from(1)
        }
    }
}

/// Initializes process-wide tracing logging.
fn init_logging(level: &str) {
    // 实现说明：非法 filter 自动回退 info，避免用户传错日志级别时 CLI 在真正工作前
    // 失败。
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}
