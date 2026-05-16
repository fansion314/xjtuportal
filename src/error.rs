use std::net::IpAddr;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PortalError>;

#[derive(Debug, Error)]
pub enum PortalError {
    #[error("读取配置文件失败 {path}: {source}")]
    ConfigRead {
        path: String,
        source: std::io::Error,
    },
    #[error("解析配置文件失败 {path}: {source}")]
    ConfigParse {
        path: String,
        source: toml::de::Error,
    },
    #[error("编辑配置文件失败 {path}: {source}")]
    ConfigEdit {
        path: String,
        source: toml_edit::TomlError,
    },
    #[error("写入配置文件失败 {path}: {source}")]
    ConfigWrite {
        path: String,
        source: std::io::Error,
    },
    #[error("配置无效: {0}")]
    InvalidConfig(String),
    #[error("创建 HTTP 客户端失败: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("HTTP 请求失败: {0}")]
    Request(reqwest::Error),
    #[error("校园网网关返回 HTTP 状态 {status}: {body}")]
    PortalHttpStatus {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("校园网网关响应缺少 Location 头")]
    MissingRedirect,
    #[error("校园网网关返回了不支持的网络状态 {0}")]
    UnsupportedNetworkStatus(reqwest::StatusCode),
    #[error("解析 URL 失败 {url}: {source}")]
    UrlParse {
        url: String,
        source: url::ParseError,
    },
    #[error("序列化 JSON 失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("解码十六进制数据失败: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("AES 解密失败")]
    Decrypt,
    #[error("AES 加密失败")]
    Encrypt,
    #[error("HTTP 头字段值无效: {0}")]
    InvalidHeaderValue(String),
    #[error("网络接口 {name} 没有可用的 IPv4 地址")]
    InterfaceAddressMissing { name: String },
    #[error("检查本机网络接口失败: {0}")]
    InterfaceInspect(#[from] std::io::Error),
    #[error("登录被拒绝: code={code:?}, error={error:?}, description={description}")]
    LoginRejected {
        code: Option<i64>,
        error: Option<i64>,
        description: String,
    },
    #[error("账号已达到设备数量上限，且自动下线功能未启用")]
    DeviceLimitReached,
    #[error("没有可下线的设备会话")]
    NoLogoutCandidate,
    #[error("自动下线后仍然达到设备数量上限: {0}")]
    StillOverloaded(String),
    #[error("登录响应中没有包含会话接口所需的 token")]
    MissingToken,
    #[error("MAC 地址无效: {0}")]
    InvalidMac(String),
    #[error("本机 IP 地址无效: {0}")]
    InvalidLocalIp(IpAddr),
    #[error("解析网关地址失败: {0}")]
    GatewayResolve(String),
    #[error("无法通过本机 IP {0} 识别当前设备会话")]
    CurrentSessionNotFound(String),
    #[error("没有设备会话匹配 {0}")]
    SessionNotFound(String),
    #[error("有多个设备会话匹配名称 {0}")]
    AmbiguousSessionName(String),
}
