use reqwest::{
    Client, StatusCode,
    header::{ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::{AccountConfig, NetworkBinding, NetworkConfig},
    crypto::{decrypt_json, encrypt_json},
    error::{PortalError, Result},
    log_network_binding,
    session::{Session, SessionListResponse},
    token_preview,
};

#[derive(Clone)]
pub struct CampusClient {
    http: Client,
    network: NetworkConfig,
}

impl CampusClient {
    pub fn new(network: NetworkConfig, binding: NetworkBinding) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            ),
        );
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN;q=0.9,zh;q=0.8"),
        );

        let mut builder = Client::builder()
            .default_headers(headers)
            .timeout(network.timeout())
            .redirect(Policy::none());
        if let Some(interface_name) = &binding.interface_name {
            builder = bind_interface(builder, interface_name);
        }
        if let Some(ip) = binding.local_ip {
            builder = builder.local_address(ip);
        }

        let http = builder.build()?;
        log_network_binding("client", &binding);
        Ok(Self { http, network })
    }

    #[cfg(test)]
    pub fn with_client(http: Client, network: NetworkConfig) -> Self {
        Self { http, network }
    }

    pub async fn check_network(&self) -> Result<NetworkStatus> {
        let response = self
            .http
            .get(&self.network.test_url)
            .send()
            .await
            .map_err(PortalError::Request)?;
        let status = response.status();

        if status.is_success() {
            return Ok(NetworkStatus::Online);
        }

        if is_redirect(status) {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(PortalError::MissingRedirect)?;

            if location.starts_with("https://") {
                return Ok(NetworkStatus::Online);
            }

            return Ok(NetworkStatus::Redirected(location.to_string()));
        }

        Err(PortalError::UnsupportedNetworkStatus(status))
    }

    pub async fn login(&self, account: &AccountConfig, redirect_url: &str) -> Result<LoginStatus> {
        let fixed_redirect_url = fix_redirect_url(redirect_url)?;
        let request = LoginRequest {
            device_type: "PC",
            redirect_url: &fixed_redirect_url,
            web_auth_user: &account.username,
            web_auth_password: &account.password,
        };
        let body = encrypt_json(&request)?;

        let response = self
            .http
            .post(&self.network.login_url)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(PortalError::Request)?
            .error_for_status()
            .map_err(PortalError::Request)?;
        let encrypted = response.text().await.map_err(PortalError::Request)?;
        let response = decrypt_json::<LoginResponse>(&encrypted)?;

        Ok(classify_login_response(response))
    }

    pub async fn list_sessions(&self, account: &AccountConfig) -> Result<Vec<Session>> {
        let token = self.auth_token(account).await?;
        let response = self
            .http
            .get(format!(
                "{}/portal/api/v2/session/list",
                self.network.gateway_base()
            ))
            .header("Authorization", &token)
            .header("Cookie", format!("token={token}"))
            .send()
            .await
            .map_err(PortalError::Request)?
            .error_for_status()
            .map_err(PortalError::Request)?;
        let response = response
            .json::<SessionListResponse>()
            .await
            .map_err(PortalError::Request)?;
        Ok(response.into_sessions())
    }

    pub async fn logout_session(&self, account: &AccountConfig, unique_id: &str) -> Result<()> {
        let token = self.auth_token(account).await?;
        self.http
            .delete(format!(
                "{}/portal/api/v2/session/acctUniqueId/{}",
                self.network.gateway_base(),
                unique_id
            ))
            .header("Authorization", &token)
            .header("Cookie", format!("token={token}"))
            .send()
            .await
            .map_err(PortalError::Request)?
            .error_for_status()
            .map_err(PortalError::Request)?;
        Ok(())
    }

    async fn auth_token(&self, account: &AccountConfig) -> Result<String> {
        let redirect_url = fake_redirect_url(&self.network);
        let request = TokenRequest {
            device_type: "PC",
            redirect_url: &redirect_url,
            data_type: "login",
            web_auth_user: &account.username,
            web_auth_password: &account.password,
        };

        let response = self
            .http
            .post(format!(
                "{}/portal/api/v2/online",
                self.network.gateway_base()
            ))
            .header(CONTENT_TYPE, "application/json")
            .header("Cookie", format!("redirectUrl={redirect_url}"))
            .json(&request)
            .send()
            .await
            .map_err(PortalError::Request)?
            .error_for_status()
            .map_err(PortalError::Request)?;
        let response = response
            .json::<TokenResponse>()
            .await
            .map_err(PortalError::Request)?;
        response
            .token
            .filter(|token| !token.is_empty())
            .ok_or(PortalError::MissingToken)
    }
}

#[cfg(any(
    target_os = "android",
    target_os = "fuchsia",
    target_os = "illumos",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "solaris",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos",
))]
fn bind_interface(builder: reqwest::ClientBuilder, interface_name: &str) -> reqwest::ClientBuilder {
    builder.interface(interface_name)
}

#[cfg(not(any(
    target_os = "android",
    target_os = "fuchsia",
    target_os = "illumos",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "solaris",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos",
)))]
fn bind_interface(
    builder: reqwest::ClientBuilder,
    _interface_name: &str,
) -> reqwest::ClientBuilder {
    builder
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkStatus {
    Online,
    Redirected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStatus {
    Success {
        token_preview: Option<String>,
    },
    Overloaded {
        description: String,
    },
    Failed {
        code: Option<i64>,
        error: Option<i64>,
        description: String,
    },
}

#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    #[serde(rename = "deviceType")]
    device_type: &'a str,
    #[serde(rename = "redirectUrl")]
    redirect_url: &'a str,
    #[serde(rename = "webAuthUser")]
    web_auth_user: &'a str,
    #[serde(rename = "webAuthPassword")]
    web_auth_password: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    #[serde(default)]
    pub code: Option<i64>,
    #[serde(default)]
    pub error: Option<i64>,
    #[serde(default, alias = "errorDescription", alias = "description")]
    pub error_description: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
struct TokenRequest<'a> {
    #[serde(rename = "deviceType")]
    device_type: &'a str,
    #[serde(rename = "redirectUrl")]
    redirect_url: &'a str,
    #[serde(rename = "type")]
    data_type: &'a str,
    #[serde(rename = "webAuthUser")]
    web_auth_user: &'a str,
    #[serde(rename = "webAuthPassword")]
    web_auth_password: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: Option<String>,
}

pub fn classify_login_response(response: LoginResponse) -> LoginStatus {
    if response.code == Some(0) || response.error == Some(0) {
        return LoginStatus::Success {
            token_preview: token_preview(response.token.as_deref()),
        };
    }

    let description = response.error_description.unwrap_or_default();
    let lower_description = description.to_ascii_lowercase();
    if response.code == Some(39)
        || response.error == Some(39)
        || (response.error == Some(81) && lower_description.contains("already have"))
        || lower_description.contains("already have")
    {
        return LoginStatus::Overloaded { description };
    }

    LoginStatus::Failed {
        code: response.code,
        error: response.error,
        description,
    }
}

pub fn fix_redirect_url(raw_url: &str) -> Result<String> {
    let mut parsed = Url::parse(raw_url).map_err(|source| PortalError::UrlParse {
        url: raw_url.to_string(),
        source,
    })?;
    if parsed.path().is_empty() {
        parsed.set_path("/");
    }
    Ok(parsed.to_string())
}

fn fake_redirect_url(network: &NetworkConfig) -> String {
    format!(
        "{}/?userip=10.180.0.1&nasip=10.6.0.1&usermac=00:11:22:33:44:55",
        network.gateway_base()
    )
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND | StatusCode::TEMPORARY_REDIRECT
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_redirect_url_without_path() {
        assert_eq!(
            fix_redirect_url("http://10.184.6.32").unwrap(),
            "http://10.184.6.32/"
        );
    }

    #[test]
    fn classifies_success_response() {
        let status = classify_login_response(LoginResponse {
            code: Some(0),
            error: None,
            error_description: None,
            token: Some("1234567890abcdef".to_string()),
        });

        assert_eq!(
            status,
            LoginStatus::Success {
                token_preview: Some("1234567890".to_string())
            }
        );
    }

    #[test]
    fn classifies_overload_response() {
        let status = classify_login_response(LoginResponse {
            code: None,
            error: Some(81),
            error_description: Some("already have 3 sessions".to_string()),
            token: None,
        });

        assert_eq!(
            status,
            LoginStatus::Overloaded {
                description: "already have 3 sessions".to_string()
            }
        );
    }

    #[test]
    fn classifies_failed_response() {
        let status = classify_login_response(LoginResponse {
            code: None,
            error: Some(81),
            error_description: Some("invalid username or password".to_string()),
            token: None,
        });

        assert_eq!(
            status,
            LoginStatus::Failed {
                code: None,
                error: Some(81),
                description: "invalid username or password".to_string()
            }
        );
    }
}
