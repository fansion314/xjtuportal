pub mod config;
pub mod crypto;
pub mod error;
pub mod interface;
pub mod portal;
pub mod session;

use std::{collections::HashMap, sync::Arc};

use config::{AppConfig, NetworkBinding, ResolvedTarget};
use error::{PortalError, Result};
use portal::{CampusClient, LoginStatus, NetworkStatus};
use session::choose_logout_mac;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    PartialFailure,
}

pub async fn run(config: AppConfig) -> Result<RunStatus> {
    let targets = config.resolved_targets()?;
    let target_groups = group_targets_by_username(targets);
    let config = Arc::new(config);
    let mut tasks = JoinSet::new();

    for targets in target_groups {
        let config = config.clone();
        tasks.spawn(async move { run_target_group(config, targets).await });
    }

    let mut failed = false;

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(group_failed) => failed |= group_failed,
            Err(err) => {
                failed = true;
                error!(error = %err, "target group task failed");
            }
        }
    }

    if failed {
        Ok(RunStatus::PartialFailure)
    } else {
        Ok(RunStatus::Success)
    }
}

fn group_targets_by_username(targets: Vec<ResolvedTarget>) -> Vec<Vec<ResolvedTarget>> {
    let mut group_indexes = HashMap::<String, usize>::new();
    let mut groups: Vec<Vec<ResolvedTarget>> = Vec::new();

    for target in targets {
        let username = target.account.username.clone();
        let group_index = *group_indexes.entry(username).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[group_index].push(target);
    }

    groups
}

async fn run_target_group(config: Arc<AppConfig>, targets: Vec<ResolvedTarget>) -> bool {
    let mut failed = false;

    for target in targets {
        if let Err(err) = run_target(&config, &target).await {
            failed = true;
            error!(target = %target.id, error = %err, "target failed");
        }
    }

    failed
}

async fn run_target(config: &AppConfig, target: &ResolvedTarget) -> Result<()> {
    let binding = target.network_binding()?;
    let client = CampusClient::new(config.network.clone(), binding.clone())?;

    info!(
        target = %target.id,
        account = %target.account.username,
        interface = target.interface_label().as_deref().unwrap_or("default"),
        bind_device = binding.interface_name.as_deref().unwrap_or("default"),
        local_ip = binding.local_ip.map(|ip| ip.to_string()).as_deref().unwrap_or("default"),
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
        LoginStatus::Overloaded { description, token } => {
            warn!(target = %target.id, description = %description, "device limit reached");
            if !config.logout.enabled {
                return Err(PortalError::DeviceLimitReached);
            }
            let token = token.ok_or(PortalError::MissingToken)?;
            logout_one_and_retry(config, target, client, redirect_url, &token).await
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
    token: &str,
) -> Result<()> {
    let sessions = client.list_sessions(token).await?;
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
        .logout_session(token, &session.unique_id, &session.api_mac)
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
        LoginStatus::Overloaded { description, .. } => {
            Err(PortalError::StillOverloaded(description))
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

pub fn token_preview(token: Option<&str>) -> Option<String> {
    token.map(|value| value.chars().take(10).collect())
}

pub(crate) fn log_network_binding(target_id: &str, binding: &NetworkBinding) {
    debug!(
        target = target_id,
        bind_device = binding.interface_name.as_deref().unwrap_or("default"),
        local_ip = binding
            .local_ip
            .map(|ip| ip.to_string())
            .as_deref()
            .unwrap_or("default"),
        "created HTTP client"
    );
}
