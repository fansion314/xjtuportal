use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use tracing_subscriber::EnvFilter;
use xjtuportal::{config::AppConfig, error::PortalError, run, RunStatus};

#[derive(Debug, Parser)]
#[command(version, about = "Automatic XJTU campus portal login")]
struct Args {
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    init_logging(&args.log_level);

    let config = match AppConfig::read(&args.config) {
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

    match run(config).await {
        Ok(RunStatus::Success) => ExitCode::SUCCESS,
        Ok(RunStatus::PartialFailure) => ExitCode::from(1),
        Err(PortalError::InvalidConfig(err)) => {
            tracing::error!("invalid config: {err}");
            ExitCode::from(2)
        }
        Err(err) => {
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
