use std::{path::PathBuf, process::ExitCode};

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use tracing_subscriber::EnvFilter;
use xjtuportal::{
    NamedSession, RunStatus, config::AppConfig, error::PortalError, list_default_sessions,
    logout_default_session, run, run_default_login,
};

#[derive(Debug, Parser)]
#[command(version, about = "Automatic XJTU campus portal login")]
struct Args {
    #[arg(short, long, value_name = "FILE", global = true)]
    config: Option<PathBuf>,
    #[arg(long, default_value = "info", global = true)]
    log_level: String,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the single-account login flow from [default_account].
    Login,
    /// List sessions for [default_account].
    List,
    /// Logout the current session, or a session selected by MAC/name.
    Logout {
        /// MAC address, or a name configured in logout.known_macs.
        selector: Option<String>,
    },
    /// Generate shell completion scripts.
    Completions {
        #[arg(value_enum)]
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
                tracing::error!("failed to resolve default config path: {err}");
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
                    println!(
                        "logged out {} ({})",
                        session.mac,
                        display_name(&session.name)
                    );
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
        "{:<18}  {:<17}  {:<15}  {:<8}  start_time",
        "name", "mac", "ip", "device"
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
    if name.is_empty() { "unknown" } else { name }
}

fn display_value(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

fn exit_code_for_error(err: PortalError) -> ExitCode {
    match err {
        PortalError::InvalidConfig(err) => {
            tracing::error!("invalid config: {err}");
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
