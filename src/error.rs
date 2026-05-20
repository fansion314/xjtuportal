//! Error types shared by the CLI and library.
//!
//! This module intentionally keeps all user-facing failures in one enum so the
//! CLI can map configuration problems to exit code `2` while treating network,
//! crypto, and portal failures as runtime errors. Error messages are written in
//! Chinese because they are surfaced directly to command-line users.

use std::net::IpAddr;

use thiserror::Error;

/// Convenient result alias used throughout the crate.
///
/// All fallible public APIs return [`PortalError`] through this alias so callers
/// can match a single error type.
pub type Result<T> = std::result::Result<T, PortalError>;

/// Top-level error type for configuration, local network, HTTP, crypto, and
/// portal protocol failures.
///
/// Variants are intentionally descriptive rather than deeply nested: most
/// errors are printed directly by the CLI, and callers usually care about the
/// broad failure category rather than an implementation-specific source chain.
#[derive(Debug, Error)]
pub enum PortalError {
    /// The configuration file could not be read from disk.
    #[error("读取配置文件失败 {path}: {source}")]
    ConfigRead {
        /// Path that was attempted.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The configuration file was not valid TOML for [`crate::config::AppConfig`].
    #[error("解析配置文件失败 {path}: {source}")]
    ConfigParse {
        /// Path that was parsed.
        path: String,
        /// TOML deserialization error.
        source: toml::de::Error,
    },
    /// The configuration file could not be parsed as editable TOML.
    #[error("编辑配置文件失败 {path}: {source}")]
    ConfigEdit {
        /// Path that was being edited.
        path: String,
        /// `toml_edit` parse or mutation error.
        source: toml_edit::TomlError,
    },
    /// The configuration file could not be written after an automatic update.
    #[error("写入配置文件失败 {path}: {source}")]
    ConfigWrite {
        /// Path that was being written.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Configuration is syntactically valid but semantically unusable.
    #[error("配置无效: {0}")]
    InvalidConfig(String),
    /// Building a `reqwest` HTTP client failed.
    #[error("创建 HTTP 客户端失败: {0}")]
    HttpClient(#[from] reqwest::Error),
    /// Sending an HTTP request or reading its body failed.
    #[error("HTTP 请求失败: {0}")]
    Request(reqwest::Error),
    /// The portal returned a non-success status that could not be interpreted.
    #[error("校园网网关返回 HTTP 状态 {status}: {body}")]
    PortalHttpStatus {
        /// HTTP response status.
        status: reqwest::StatusCode,
        /// Raw response body for diagnostics.
        body: String,
    },
    /// A redirect response did not include a `Location` header.
    #[error("校园网网关响应缺少 Location 头")]
    MissingRedirect,
    /// The redirect probe returned a status outside the supported cases.
    #[error("校园网网关返回了不支持的网络状态 {0}")]
    UnsupportedNetworkStatus(reqwest::StatusCode),
    /// A configured or portal-provided URL could not be parsed.
    #[error("解析 URL 失败 {url}: {source}")]
    UrlParse {
        /// URL text that failed parsing.
        url: String,
        /// Parser error from the `url` crate.
        source: url::ParseError,
    },
    /// JSON serialization or deserialization failed.
    #[error("序列化 JSON 失败: {0}")]
    Json(#[from] serde_json::Error),
    /// Hex decoding failed for an encrypted portal body.
    #[error("解码十六进制数据失败: {0}")]
    Hex(#[from] hex::FromHexError),
    /// AES-CBC decryption or UTF-8 conversion failed.
    #[error("AES 解密失败")]
    Decrypt,
    /// AES-CBC encryption failed.
    #[error("AES 加密失败")]
    Encrypt,
    /// An HTTP header value contained invalid bytes.
    #[error("HTTP 头字段值无效: {0}")]
    InvalidHeaderValue(String),
    /// A named local interface did not have a usable IPv4 address.
    #[error("网络接口 {name} 没有可用的 IPv4 地址")]
    InterfaceAddressMissing { name: String },
    /// Inspecting local network interfaces failed.
    #[error("检查本机网络接口失败: {0}")]
    InterfaceInspect(#[from] std::io::Error),
    /// The portal rejected a login attempt.
    #[error("登录被拒绝: code={code:?}, error={error:?}, description={description}")]
    LoginRejected {
        /// Optional `code` field from the portal.
        code: Option<i64>,
        /// Optional `error` field from the portal.
        error: Option<i64>,
        /// Human-readable portal description.
        description: String,
    },
    /// Login reached the device limit and automatic logout was disabled.
    #[error("账号已达到设备数量上限，且自动下线功能未启用")]
    DeviceLimitReached,
    /// No valid session was available for automatic logout.
    #[error("没有可下线的设备会话")]
    NoLogoutCandidate,
    /// Login still reported device-limit after one automatic logout and retry.
    #[error("自动下线后仍然达到设备数量上限: {0}")]
    StillOverloaded(String),
    /// A login response did not include the token needed by session APIs.
    #[error("登录响应中没有包含会话接口所需的 token")]
    MissingToken,
    /// A MAC address could not be normalized.
    #[error("MAC 地址无效: {0}")]
    InvalidMac(String),
    /// A configured local IP is not a usable IPv4 address.
    #[error("本机 IP 地址无效: {0}")]
    InvalidLocalIp(IpAddr),
    /// The configured gateway could not be resolved for route probing.
    #[error("解析网关地址失败: {0}")]
    GatewayResolve(String),
    /// The CLI could not identify the current device in the session list.
    #[error("无法通过本机 IP {0} 识别当前设备会话")]
    CurrentSessionNotFound(String),
    /// No session matched a user-provided selector.
    #[error("没有设备会话匹配 {0}")]
    SessionNotFound(String),
    /// A user-provided known-MAC name matched more than one configured MAC.
    #[error("有多个设备会话匹配名称 {0}")]
    AmbiguousSessionName(String),
    /// A spawned async task failed to join.
    #[error("后台任务执行失败: {0}")]
    TaskJoin(String),
}
