pub mod config;
pub mod crypto;
pub mod error;
pub mod interface;
pub mod portal;
pub mod session;

use std::net::IpAddr;

use config::{AppConfig, ResolvedTarget};
use error::{PortalError, Result};
use portal::{CampusClient, LoginStatus, NetworkStatus};
use session::choose_logout_mac;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    PartialFailure,
}

pub async fn run(config: AppConfig) -> Result<RunStatus> {
    let targets = config.resolved_targets()?;
    let mut failed = false;

    for target in targets {
        if let Err(err) = run_target(&config, &target).await {
            failed = true;
            error!(target = %target.id, error = %err, "target failed");
        }
    }

    if failed {
        Ok(RunStatus::PartialFailure)
    } else {
        Ok(RunStatus::Success)
    }
}

async fn run_target(config: &AppConfig, target: &ResolvedTarget) -> Result<()> {
    let local_ip = target.local_ip()?;
    let client = CampusClient::new(config.network.clone(), local_ip)?;

    info!(
        target = %target.id,
        account = %target.account.username,
        interface = target.interface_label().as_deref().unwrap_or("default"),
        local_ip = local_ip.map(|ip| ip.to_string()).as_deref().unwrap_or("default"),
        "checking network"
    );

    match client.check_network().await? {
        NetworkStatus::Online => {
            info!(target = %target.id, "already online");
            Ok(())
        }
        NetworkStatus::Redirected(redirect_url) => {
            login_with_optional_logout(config, target, &client, &redirect_url).await
        }
    }
}

async fn login_with_optional_logout(
    config: &AppConfig,
    target: &ResolvedTarget,
    client: &CampusClient,
    redirect_url: &str,
) -> Result<()> {
    match client.login(&target.account, redirect_url).await? {
        LoginStatus::Success { token_preview } => {
            info!(target = %target.id, token = %token_preview.unwrap_or_default(), "login success");
            Ok(())
        }
        LoginStatus::Overloaded { description } => {
            warn!(target = %target.id, description = %description, "device limit reached");
            if !config.logout.enabled {
                return Err(PortalError::DeviceLimitReached);
            }
            logout_one_and_retry(config, target, client, redirect_url).await
        }
        LoginStatus::Failed {
            code,
            error,
            description,
        } => Err(PortalError::LoginRejected {
            code,
            error,
            description,
        }),
    }
}

async fn logout_one_and_retry(
    config: &AppConfig,
    target: &ResolvedTarget,
    client: &CampusClient,
    redirect_url: &str,
) -> Result<()> {
    let sessions = client.list_sessions(&target.account).await?;
    let session_macs = sessions
        .iter()
        .map(|session| session.mac.as_str())
        .collect::<Vec<_>>();
    let logout_mac = choose_logout_mac(&session_macs, &config.logout.known_macs)
        .ok_or(PortalError::NoLogoutCandidate)?;
    let session = sessions
        .iter()
        .find(|session| session.mac == logout_mac)
        .ok_or(PortalError::NoLogoutCandidate)?;

    info!(target = %target.id, mac = %session.mac, "logging out existing session");
    client
        .logout_session(&target.account, &session.unique_id)
        .await?;

    match client.login(&target.account, redirect_url).await? {
        LoginStatus::Success { token_preview } => {
            info!(
                target = %target.id,
                token = %token_preview.unwrap_or_default(),
                "login success after automatic logout"
            );
            Ok(())
        }
        LoginStatus::Overloaded { description } => Err(PortalError::StillOverloaded(description)),
        LoginStatus::Failed {
            code,
            error,
            description,
        } => Err(PortalError::LoginRejected {
            code,
            error,
            description,
        }),
    }
}

pub fn token_preview(token: Option<&str>) -> Option<String> {
    token.map(|value| value.chars().take(10).collect())
}

pub(crate) fn log_local_binding(target_id: &str, local_ip: Option<IpAddr>) {
    debug!(
        target = target_id,
        local_ip = local_ip
            .map(|ip| ip.to_string())
            .as_deref()
            .unwrap_or("default"),
        "created HTTP client"
    );
}
