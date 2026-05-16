use std::{path::PathBuf, process::ExitCode};

use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use tracing_subscriber::EnvFilter;
use xjtuportal::{
    NamedSession, RunStatus, config::AppConfig, error::PortalError, list_default_sessions,
    logout_default_session, run, run_default_login,
};

const HELP_TEMPLATE: &str = "\
{about-with-newline}
用法: {usage}

{all-args}{after-help}";

#[derive(Debug, Parser)]
#[command(
    version,
    about = "西安交大校园网自动登录工具",
    long_about = "西安交大校园网自动登录工具。不输入任何子命令时，会直接执行全自动登录流程；也可以使用 login/list/logout 管理当前默认账号的登录设备。",
    disable_help_flag = true,
    disable_help_subcommand = true,
    disable_version_flag = true,
    propagate_version = true,
    help_template = HELP_TEMPLATE,
    next_help_heading = "选项",
    subcommand_help_heading = "命令"
)]
struct Args {
    #[arg(short, long, value_name = "文件", global = true, help = "配置文件路径")]
    config: Option<PathBuf>,
    #[arg(
        long,
        default_value = "info",
        global = true,
        help = "日志级别，例如 error、warn、info、debug"
    )]
    log_level: String,
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
    /// 使用 [default_account] 执行单账号登录。
    #[command(help_template = HELP_TEMPLATE)]
    Login,
    /// 列出 [default_account] 当前已经登录的设备。
    #[command(help_template = HELP_TEMPLATE)]
    List,
    /// 下线当前设备，或下线指定 MAC/名称对应的设备。
    #[command(help_template = HELP_TEMPLATE)]
    Logout {
        /// MAC 地址，或 logout.known_macs 中配置的名称。
        #[arg(help_heading = "参数")]
        selector: Option<String>,
    },
    /// 生成 shell 自动补全脚本。
    #[command(help_template = HELP_TEMPLATE)]
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

    let result = match args.command {
        None => run(config, Some(config_path))
            .await
            .map(|status| match status {
                RunStatus::Success => None,
                RunStatus::PartialFailure => Some(ExitCode::from(1)),
            }),
        Some(Command::Login) => run_default_login(config, Some(config_path))
            .await
            .map(|()| None),
        Some(Command::List) => {
            list_default_sessions(config, Some(config_path))
                .await
                .map(|sessions| {
                    print_sessions(&sessions);
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

fn default_config_path() -> std::io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    Ok(executable
        .parent()
        .map(|directory| directory.join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml")))
}

fn print_sessions(sessions: &[NamedSession]) {
    println!(
        "{:<12}  {:<17}  {:<15}  {:<8}  登录时间",
        "名称", "MAC", "IP", "设备"
    );
    for session in sessions {
        println!(
            "{:<18}  {:<17}  {:<15}  {:<8}  {}",
            display_name(&session.name),
            session.mac,
            display_value(&session.user_ip),
            display_value(&session.device_type),
            display_value(&session.start_time),
        );
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
