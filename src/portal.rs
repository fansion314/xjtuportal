//! HTTP client for the campus portal v3 protocol.
//!
//! This module owns the encrypted endpoint calls and the redirect probe. All
//! clients disable automatic redirects so `http://1.1.1.1` can reveal the portal
//! `Location` header when the network is captive. Login, session list, and
//! logout all use the verified v3 encrypted APIs.

use reqwest::{
    Client, StatusCode,
    header::{
        ACCEPT, ACCEPT_LANGUAGE, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HeaderMap,
        HeaderValue, ORIGIN, PRAGMA, REFERER, USER_AGENT,
    },
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
};

/// Portal HTTP client bound to a network configuration and optional interface.
#[derive(Clone)]
pub struct CampusClient {
    /// Underlying reqwest client with redirects disabled.
    http: Client,
    /// URL and timeout configuration used by portal calls.
    network: NetworkConfig,
}

impl CampusClient {
    /// Builds a portal client for a network binding.
    ///
    /// `binding.interface_name` is applied through `reqwest` interface binding
    /// on supported platforms; `binding.local_ip` is applied as an additional
    /// source-address hint.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::HttpClient`] if reqwest cannot construct the
    /// client.
    pub fn new(network: NetworkConfig, binding: NetworkBinding) -> Result<Self> {
        // 实现说明：默认 header 模拟常见浏览器请求；禁用 redirect 是探测 captive
        // portal 的关键，因为 Location 才包含 redirectUrl/nasip。
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

    /// Creates a client from a prebuilt reqwest client for tests.
    #[cfg(test)]
    pub fn with_client(http: Client, network: NetworkConfig) -> Self {
        // 实现说明：测试可注入 mock server client，同时复用生产解析逻辑。
        Self { http, network }
    }

    /// Checks whether the current binding is already online or captive-redirected.
    ///
    /// A successful response means online. A plain HTTP redirect means the portal
    /// login flow should use the `Location` value. HTTPS redirects are treated as
    /// online to avoid following unrelated secure redirects as portal redirects.
    ///
    /// # Errors
    ///
    /// Returns request errors, [`PortalError::MissingRedirect`] for redirect
    /// statuses without `Location`, or [`PortalError::UnsupportedNetworkStatus`]
    /// for other HTTP statuses.
    pub async fn check_network(&self) -> Result<NetworkStatus> {
        // 实现说明：network.test_url 默认为 http://1.1.1.1，配合 Policy::none 获取
        // 原始状态和 Location。
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

    /// Attempts portal login for an account using a redirect URL.
    ///
    /// The request is encrypted as compact JSON and sent to the v3 login
    /// endpoint. Non-success HTTP responses are still decrypted when possible
    /// because the portal may encode meaningful login status in the encrypted
    /// body.
    ///
    /// # Errors
    ///
    /// Returns URL, encryption, request, HTTP status, or decryption errors when
    /// the login exchange cannot be completed.
    pub async fn login(&self, account: &AccountConfig, redirect_url: &str) -> Result<LoginStatus> {
        // 实现说明：fix_redirect_url 补齐空 path，否则门户可能拒绝裸 host URL；响应
        // 交给 classify_login_response 统一判断成功/超限/失败。
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
            .map_err(PortalError::Request)?;
        let status = response.status();
        let encrypted = response.text().await.map_err(PortalError::Request)?;
        if !status.is_success() {
            if let Ok(response) = decrypt_json::<LoginResponse>(&encrypted) {
                return Ok(classify_login_response(response));
            }
            return Err(PortalError::PortalHttpStatus {
                status,
                body: encrypted,
            });
        }
        let response = decrypt_json::<LoginResponse>(&encrypted)?;

        Ok(classify_login_response(response))
    }

    /// Lists active sessions using a token returned by a login response.
    ///
    /// The session-list v3 endpoint expects an empty body and returns an
    /// encrypted response.
    ///
    /// # Errors
    ///
    /// Returns request, header, decryption, or response-normalization errors.
    pub async fn list_sessions(&self, token: &str) -> Result<Vec<Session>> {
        // 实现说明：v3 session/list 使用 Authorization token + 空 body；SessionListResponse
        // 负责过滤掉不完整记录。
        let response = self
            .http
            .post(&self.network.session_list_url)
            .headers(self.session_headers(token)?)
            .body(Vec::new())
            .send()
            .await
            .map_err(PortalError::Request)?
            .error_for_status()
            .map_err(PortalError::Request)?;
        let encrypted = response.text().await.map_err(PortalError::Request)?;
        let response = decrypt_json::<SessionListResponse>(&encrypted)?;
        Ok(response.into_sessions())
    }

    /// Logs out a single active session by accounting unique ID and API MAC.
    ///
    /// `mac` should be the original MAC string returned by the portal, not a
    /// locally normalized MAC, because the logout endpoint may expect the
    /// original separator format.
    ///
    /// # Errors
    ///
    /// Returns encryption, header, or request errors if the logout call fails.
    pub async fn logout_session(&self, token: &str, unique_id: &str, mac: &str) -> Result<()> {
        // 实现说明：请求体字段名必须是 acctUniqueId；HTTP body 仍走 v3 加密。
        let request = LogoutRequest {
            acct_unique_id: unique_id,
            mac,
        };
        let body = encrypt_json(&request)?;
        self.http
            .post(&self.network.session_logout_url)
            .headers(self.session_headers(token)?)
            .body(body)
            .send()
            .await
            .map_err(PortalError::Request)?
            .error_for_status()
            .map_err(PortalError::Request)?;
        Ok(())
    }

    /// Builds common headers required by session APIs.
    ///
    /// # Errors
    ///
    /// Returns [`PortalError::InvalidHeaderValue`] if the token, Origin, or
    /// Referer cannot be represented as valid HTTP header values.
    fn session_headers(&self, token: &str) -> Result<HeaderMap> {
        // 实现说明：这些 header 模拟门户前端调用 session API 的形态；Origin/Referer
        // 基于 gateway_base 生成，避免和可覆盖的 endpoint URL 强耦合。
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(token)
                .map_err(|err| PortalError::InvalidHeaderValue(err.to_string()))?,
        );
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            ORIGIN,
            HeaderValue::from_str(&self.network.gateway_base())
                .map_err(|err| PortalError::InvalidHeaderValue(err.to_string()))?,
        );
        headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        headers.insert(
            REFERER,
            HeaderValue::from_str(&format!("{}/wenet/auth", self.network.gateway_base()))
                .map_err(|err| PortalError::InvalidHeaderValue(err.to_string()))?,
        );
        Ok(headers)
    }
}

/// Applies OS-level interface binding on platforms supported by reqwest.
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
    // 实现说明：在 Linux/OpenWrt 上这会走 SO_BINDTODEVICE，能配合 mwan3 避免只靠
    // source IP 仍被策略路由送到错误 WAN。
    builder.interface(interface_name)
}

/// Leaves the client unbound on platforms where reqwest has no interface binding.
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
    // 实现说明：保持 API 行为可编译；调用方仍可使用 local_ip 作为弱提示。
    builder
}

/// Result of the captive-portal network probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkStatus {
    /// The test URL succeeded, so no login is needed.
    Online,
    /// The test URL redirected to a portal login URL.
    Redirected(String),
}

/// Classified portal login response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStatus {
    /// Login succeeded. A token may be present for subsequent session APIs.
    Success {
        /// Optional token returned by the portal.
        token: Option<String>,
    },
    /// Login reached the device limit. Some responses include a session API token.
    Overloaded {
        /// Portal-provided description of the limit condition.
        description: String,
        /// Optional token used to list and logout sessions.
        token: Option<String>,
    },
    /// Login failed for reasons other than device limit.
    Failed {
        /// Optional `code` from the portal.
        code: Option<i64>,
        /// Optional `error` from the portal.
        error: Option<i64>,
        /// Portal-provided failure description.
        description: String,
    },
}

/// Encrypted v3 login request body before encryption.
#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    /// Portal device type. The reverse-engineered flow uses `PC`.
    #[serde(rename = "deviceType")]
    device_type: &'a str,
    /// Redirect URL obtained from the captive portal probe.
    #[serde(rename = "redirectUrl")]
    redirect_url: &'a str,
    /// Account username field expected by the portal.
    #[serde(rename = "webAuthUser")]
    web_auth_user: &'a str,
    /// Account password field expected by the portal.
    #[serde(rename = "webAuthPassword")]
    web_auth_password: &'a str,
}

/// Decrypted v3 login response.
#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    /// Optional status code field observed in some responses.
    #[serde(default)]
    pub code: Option<i64>,
    /// Optional error field observed in other responses.
    #[serde(default)]
    pub error: Option<i64>,
    /// Optional human-readable error description.
    #[serde(default, alias = "errorDescription", alias = "description")]
    pub error_description: Option<String>,
    /// Optional token used by session APIs.
    #[serde(default)]
    pub token: Option<String>,
}

/// Encrypted v3 logout request body before encryption.
#[derive(Debug, Serialize)]
struct LogoutRequest<'a> {
    /// Accounting unique ID returned by the session-list API.
    #[serde(rename = "acctUniqueId")]
    acct_unique_id: &'a str,
    /// Portal MAC string returned by the session-list API.
    mac: &'a str,
}

/// Classifies a decrypted login response into success, device-limit, or failure.
///
/// The portal has been observed using either `code` or `error`; both are checked.
pub fn classify_login_response(response: LoginResponse) -> LoginStatus {
    // 实现说明：0 表示成功；39、81 和英文 already-have 文案都表示设备数超限，其它
    // 情况保留原始字段给上层错误展示。
    if response.code == Some(0) || response.error == Some(0) {
        return LoginStatus::Success {
            token: response.token,
        };
    }

    let description = response.error_description.unwrap_or_default();
    let lower_description = description.to_ascii_lowercase();
    if response.code == Some(39)
        || response.error == Some(39)
        || response.error == Some(81)
        || lower_description.contains("already have")
    {
        return LoginStatus::Overloaded {
            description,
            token: response.token,
        };
    }

    LoginStatus::Failed {
        code: response.code,
        error: response.error,
        description,
    }
}

/// Ensures a redirect URL has a path component acceptable to the portal.
///
/// # Errors
///
/// Returns [`PortalError::UrlParse`] if `raw_url` is not a valid URL.
pub fn fix_redirect_url(raw_url: &str) -> Result<String> {
    // 实现说明：url crate 对裸 host 的 path 为空；门户登录通常需要至少 "/"。
    let mut parsed = Url::parse(raw_url).map_err(|source| PortalError::UrlParse {
        url: raw_url.to_string(),
        source,
    })?;
    if parsed.path().is_empty() {
        parsed.set_path("/");
    }
    Ok(parsed.to_string())
}

/// Returns true for redirect statuses that the portal probe understands.
fn is_redirect(status: StatusCode) -> bool {
    // 实现说明：校园网目前会返回 301/302/307；其它 3xx 先不猜测，以便异常时暴露。
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND | StatusCode::TEMPORARY_REDIRECT
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Verifies redirect URLs without a path are normalized.
    fn fixes_redirect_url_without_path() {
        // 实现说明：覆盖 fix_redirect_url 唯一的变形逻辑。
        assert_eq!(
            fix_redirect_url("http://10.184.6.32").unwrap(),
            "http://10.184.6.32/"
        );
    }

    #[test]
    /// Verifies `code = 0` is treated as successful login.
    fn classifies_success_response() {
        // 实现说明：成功分支应保留 token，供 session API 调用。
        let status = classify_login_response(LoginResponse {
            code: Some(0),
            error: None,
            error_description: None,
            token: Some("1234567890abcdef".to_string()),
        });

        assert_eq!(
            status,
            LoginStatus::Success {
                token: Some("1234567890abcdef".to_string()),
            }
        );
    }

    #[test]
    /// Verifies known device-limit responses are classified as overloaded.
    fn classifies_overload_response() {
        // 实现说明：error=81 是已验证的超限形式，同时保留 token。
        let status = classify_login_response(LoginResponse {
            code: None,
            error: Some(81),
            error_description: Some("already have 3 sessions".to_string()),
            token: Some("session-token".to_string()),
        });

        assert_eq!(
            status,
            LoginStatus::Overloaded {
                description: "already have 3 sessions".to_string(),
                token: Some("session-token".to_string())
            }
        );
    }

    #[test]
    /// Verifies unrelated portal errors are exposed as failed login.
    fn classifies_failed_response() {
        // 实现说明：失败分支保留 code/error/description，便于 CLI 打印诊断信息。
        let status = classify_login_response(LoginResponse {
            code: None,
            error: Some(1),
            error_description: Some("invalid username or password".to_string()),
            token: None,
        });

        assert_eq!(
            status,
            LoginStatus::Failed {
                code: None,
                error: Some(1),
                description: "invalid username or password".to_string()
            }
        );
    }
}
