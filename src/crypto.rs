//! Portal request and response encryption helpers.
//!
//! The campus portal v3 APIs exchange AES-128-CBC encrypted, hex-encoded
//! payloads. The key, IV, padding, and compact JSON behavior match the Python
//! reverse-engineering scripts in `exp/`; changing any of those constants will
//! break compatibility with the verified endpoint.

use aes::Aes128;
use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use cbc::{Decryptor, Encryptor};
use serde::{Serialize, de::DeserializeOwned};

use crate::error::{PortalError, Result};

/// Fixed AES-128 key required by the portal v3 protocol.
const KEY: &[u8; 16] = b"1234567890000000";
/// Fixed AES-128 IV required by the portal v3 protocol.
const IV: &[u8; 16] = b"1234567890000000";

type Aes128CbcEnc = Encryptor<Aes128>;
type Aes128CbcDec = Decryptor<Aes128>;

/// Serializes a value as compact JSON and encrypts it for a portal request.
///
/// The returned string is lowercase hex and is ready to be used as the raw HTTP
/// request body for v3 portal APIs.
///
/// # Errors
///
/// Returns [`PortalError::Json`] if serialization fails, or
/// [`PortalError::Encrypt`] if encryption fails.
pub fn encrypt_json<T: Serialize>(value: &T) -> Result<String> {
    // 实现说明：serde_json::to_string 默认输出紧凑 JSON，正好符合门户加密前的
    // body 形态；加密细节统一交给 encrypt_text，避免协议参数分散。
    let text = serde_json::to_string(value)?;
    encrypt_text(&text)
}

/// Encrypts plain text with the portal AES-128-CBC settings.
///
/// The plaintext is padded with PKCS#7 and returned as lowercase hex.
///
/// # Errors
///
/// Returns [`PortalError::Encrypt`] if the cipher backend reports an encryption
/// failure.
pub fn encrypt_text(text: &str) -> Result<String> {
    // 实现说明：cbc crate 的 encrypt_padded_vec 会完成 PKCS#7 padding；门户需要
    // hex 文本而不是二进制 body，所以最后统一 hex::encode。
    let ciphertext =
        Aes128CbcEnc::new(KEY.into(), IV.into()).encrypt_padded_vec::<Pkcs7>(text.as_bytes());
    Ok(hex::encode(ciphertext))
}

/// Decrypts a portal response and deserializes the JSON payload.
///
/// The input may be either a plain hex body or a JSON string containing the hex
/// body, because the portal has been observed returning both shapes.
///
/// # Errors
///
/// Returns [`PortalError::Hex`] for malformed hex, [`PortalError::Decrypt`] for
/// invalid ciphertext or UTF-8, and [`PortalError::Json`] if the decrypted JSON
/// does not match `T`.
pub fn decrypt_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    // 实现说明：先恢复明文字符串，再让 serde 根据调用方的响应结构做强类型解析。
    let plaintext = decrypt_text(text)?;
    serde_json::from_str(&plaintext).map_err(PortalError::from)
}

/// Decrypts a portal hex response into UTF-8 text.
///
/// Quoted JSON string bodies are accepted and unwrapped before hex decoding.
///
/// # Errors
///
/// Returns [`PortalError::Hex`] for malformed hex and [`PortalError::Decrypt`]
/// for padding, cipher, or UTF-8 failures.
pub fn decrypt_text(text: &str) -> Result<String> {
    // 实现说明：normalize_hex_body 兼容网关偶尔包一层 JSON 字符串的响应；随后按
    // AES-CBC + PKCS#7 还原明文。
    let normalized = normalize_hex_body(text)?;
    let ciphertext = hex::decode(normalized)?;
    let plaintext = Aes128CbcDec::new(KEY.into(), IV.into())
        .decrypt_padded_vec::<Pkcs7>(&ciphertext)
        .map_err(|_| PortalError::Decrypt)?;
    String::from_utf8(plaintext).map_err(|_| PortalError::Decrypt)
}

/// Normalizes an encrypted response body to its raw hex text.
///
/// This helper trims whitespace and unwraps a JSON string if the HTTP body is
/// quoted.
///
/// # Errors
///
/// Returns [`PortalError::Json`] if a quoted body is not a valid JSON string.
fn normalize_hex_body(text: &str) -> Result<String> {
    // 实现说明：只在首尾都是双引号时调用 serde，避免把普通 hex 文本误当 JSON。
    let trimmed = text.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        serde_json::from_str::<String>(trimmed).map_err(PortalError::from)
    } else {
        Ok(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    /// Test payload matching the portal login request field names.
    struct Payload<'a> {
        #[serde(rename = "deviceType")]
        device_type: &'a str,
        #[serde(rename = "redirectUrl")]
        redirect_url: &'a str,
        #[serde(rename = "webAuthUser")]
        web_auth_user: &'a str,
        #[serde(rename = "webAuthPassword")]
        web_auth_password: &'a str,
    }

    #[test]
    /// Verifies that encrypted JSON stays byte-compatible with the Python probe.
    fn encrypts_compact_json_with_python_compatible_vector() {
        // 实现说明：固定向量能同时守住 JSON 紧凑序列化、AES 参数、padding 和
        // hex 输出大小写。
        let payload = Payload {
            device_type: "PC",
            redirect_url: "http://10.184.6.32/",
            web_auth_user: "u",
            web_auth_password: "p",
        };

        let encrypted = encrypt_json(&payload).unwrap();

        assert_eq!(
            encrypted,
            "d6eeef71f42c5e7f48a8aee242df00ea3f0533f4b36302af0e4d5ca6f21f837b\
             a13cba2d341a47658392810d8438b7df5ae1e19f242f4e591bd8eed4761053685b659\
             8a2693d8113b1de4875f9632ebac06c8c3cbc26111bd59683d1a8f0b3c9"
                .replace(char::is_whitespace, "")
        );
    }

    #[test]
    /// Verifies decryption for both raw and JSON-quoted hex bodies.
    fn decrypts_plain_and_json_quoted_hex_bodies() {
        // 实现说明：先用生产加密函数制造响应体，再分别覆盖两个 normalize 分支。
        let encrypted = encrypt_text(r#"{"code":0,"token":"abcdef"}"#).unwrap();

        let plain: serde_json::Value = decrypt_json(&encrypted).unwrap();
        let quoted: serde_json::Value =
            decrypt_json(&serde_json::to_string(&encrypted).unwrap()).unwrap();

        assert_eq!(plain["code"], 0);
        assert_eq!(quoted["token"], "abcdef");
    }
}
