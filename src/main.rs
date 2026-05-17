use std::{path::PathBuf, process::ExitCode};

use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use tracing_subscriber::EnvFilter;
use xjtuportal::{
    AccountSessions, NamedSession, RunStatus, config::AppConfig, error::PortalError,
    list_account_sessions, list_default_sessions, logout_account_sessions, logout_default_session,
    run, run_default_login,
};

const ROOT_HELP_TEMPLATE: &str = "\
西安交大校园网自动登录工具

用法: {usage}
不输入任何子命令时，会直接执行全自动登录流程；也可以使用 login/list/logout 管理登录设备。

{all-args}{after-help}";

const COMMAND_HELP_TEMPLATE: &str = "\
{about-with-newline}
用法: {usage}

{all-args}{after-help}";

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
    #[arg(short, long, value_name = "FILE", global = true, help = "配置文件路径")]
    config: Option<PathBuf>,
    #[arg(
        long,
        default_value = "info",
        global = true,
        help = "日志级别，例如 error、warn、info、debug"
    )]
    log_level: String,
    #[arg(
        long,
        global = true,
        help = "即使配置了多个 targets，也只使用 [default_account] 按单目标模式执行"
    )]
    one: bool,
    #[arg(
        short = 'h',
        long = "help",
        global = true,
        action = ArgAction::Help,
        help = "显示帮助信息"
    )]
    help: Option<bool>,
    #[arg(
        short = 'V',
        long = "version",
        global = true,
        action = ArgAction::Version,
        help = "显示版本信息"
    )]
    version: Option<bool>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 执行自动登录；多目标配置会登录所有 targets。
    #[command(help_template = COMMAND_HELP_TEMPLATE)]
    Login,
    /// 列出当前已经登录的设备；多目标配置会按账号分组展示。
    #[command(help_template = COMMAND_HELP_TEMPLATE)]
    List,
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
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
        Some(Command::Login) if args.one => run_default_login(config, Some(config_path))
            .await
            .map(|()| None),
        Some(Command::Login) => run(config, Some(config_path)).await.map(run_exit_code),
        Some(Command::List) if multi_account_mode => {
            list_account_sessions(config, Some(config_path))
                .await
                .map(|groups| {
                    print_account_sessions(&groups);
                    None
                })
        }
        Some(Command::List) => {
            list_default_sessions(config, Some(config_path))
                .await
                .map(|sessions| {
                    print_sessions(&sessions);
                    None
                })
        }
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

fn run_exit_code(status: RunStatus) -> Option<ExitCode> {
    match status {
        RunStatus::Success => None,
        RunStatus::PartialFailure => Some(ExitCode::from(1)),
    }
}

fn default_config_path() -> std::io::Result<PathBuf> {
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

fn print_account_sessions(groups: &[AccountSessions]) {
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("账号: {}", group.account);
        print_sessions(&group.sessions);
    }
}

fn print_sessions(sessions: &[NamedSession]) {
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

fn pad_display(value: &str, width: usize) -> String {
    let display_width = terminal_display_width(value);
    let padding = width.saturating_sub(display_width);
    format!("{value}{}", " ".repeat(padding))
}

fn terminal_display_width(value: &str) -> usize {
    value.chars().map(char_display_width).sum()
}

fn char_display_width(value: char) -> usize {
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

fn display_name(name: &str) -> &str {
    if name.is_empty() { "未知" } else { name }
}

fn display_value(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

fn exit_code_for_error(err: PortalError) -> ExitCode {
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

fn init_logging(level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}
