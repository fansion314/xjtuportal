use aes::Aes128;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use cbc::{Decryptor, Encryptor};
use serde::{Serialize, de::DeserializeOwned};

use crate::error::{PortalError, Result};

const KEY: &[u8; 16] = b"1234567890000000";
const IV: &[u8; 16] = b"1234567890000000";

type Aes128CbcEnc = Encryptor<Aes128>;
type Aes128CbcDec = Decryptor<Aes128>;

pub fn encrypt_json<T: Serialize>(value: &T) -> Result<String> {
    let text = serde_json::to_string(value)?;
    encrypt_text(&text)
}

pub fn encrypt_text(text: &str) -> Result<String> {
    let ciphertext =
        Aes128CbcEnc::new(KEY.into(), IV.into()).encrypt_padded_vec_mut::<Pkcs7>(text.as_bytes());
    Ok(hex::encode(ciphertext))
}

pub fn decrypt_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    let plaintext = decrypt_text(text)?;
    serde_json::from_str(&plaintext).map_err(PortalError::from)
}

pub fn decrypt_text(text: &str) -> Result<String> {
    let normalized = normalize_hex_body(text)?;
    let mut ciphertext = hex::decode(normalized)?;
    let plaintext = Aes128CbcDec::new(KEY.into(), IV.into())
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| PortalError::Decrypt)?;
    String::from_utf8(plaintext.to_vec()).map_err(|_| PortalError::Decrypt)
}

fn normalize_hex_body(text: &str) -> Result<String> {
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
    fn encrypts_compact_json_with_python_compatible_vector() {
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
    fn decrypts_plain_and_json_quoted_hex_bodies() {
        let encrypted = encrypt_text(r#"{"code":0,"token":"abcdef"}"#).unwrap();

        let plain: serde_json::Value = decrypt_json(&encrypted).unwrap();
        let quoted: serde_json::Value =
            decrypt_json(&serde_json::to_string(&encrypted).unwrap()).unwrap();

        assert_eq!(plain["code"], 0);
        assert_eq!(quoted["token"], "abcdef");
    }
}
