use std::net::IpAddr;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PortalError>;

#[derive(Debug, Error)]
pub enum PortalError {
    #[error("failed to read config {path}: {source}")]
    ConfigRead {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    ConfigParse {
        path: String,
        source: toml::de::Error,
    },
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("HTTP request failed: {0}")]
    Request(reqwest::Error),
    #[error("portal response is missing Location header")]
    MissingRedirect,
    #[error("portal returned unsupported network status {0}")]
    UnsupportedNetworkStatus(reqwest::StatusCode),
    #[error("failed to parse URL {url}: {source}")]
    UrlParse {
        url: String,
        source: url::ParseError,
    },
    #[error("failed to serialize JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to decode hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("AES decrypt failed")]
    Decrypt,
    #[error("AES encrypt failed")]
    Encrypt,
    #[error("interface {name} has no usable IPv4 address")]
    InterfaceAddressMissing { name: String },
    #[error("failed to inspect local interfaces: {0}")]
    InterfaceInspect(#[from] std::io::Error),
    #[error("login rejected: code={code:?}, error={error:?}, description={description}")]
    LoginRejected {
        code: Option<i64>,
        error: Option<i64>,
        description: String,
    },
    #[error("device limit reached and automatic logout is disabled")]
    DeviceLimitReached,
    #[error("there is no session candidate to logout")]
    NoLogoutCandidate,
    #[error("still overloaded after automatic logout: {0}")]
    StillOverloaded(String),
    #[error("session token response did not include a token")]
    MissingToken,
    #[error("invalid MAC address {0}")]
    InvalidMac(String),
    #[error("invalid local IP address {0}")]
    InvalidLocalIp(IpAddr),
}
