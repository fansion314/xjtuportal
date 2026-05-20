**xjtuportal > crypto**

# Module: crypto

## Contents

**Functions**

- [`decrypt_json`](#decrypt_json) - Decrypts a portal response and deserializes the JSON payload.
- [`decrypt_text`](#decrypt_text) - Decrypts a portal hex response into UTF-8 text.
- [`encrypt_json`](#encrypt_json) - Serializes a value as compact JSON and encrypts it for a portal request.
- [`encrypt_text`](#encrypt_text) - Encrypts plain text with the portal AES-128-CBC settings.

---

## xjtuportal::crypto::decrypt_json

*Function*

Decrypts a portal response and deserializes the JSON payload.

The input may be either a plain hex body or a JSON string containing the hex
body, because the portal has been observed returning both shapes.

# Errors

Returns [`PortalError::Hex`] for malformed hex, [`PortalError::Decrypt`] for
invalid ciphertext or UTF-8, and [`PortalError::Json`] if the decrypted JSON
does not match `T`.

```rust
fn decrypt_json<T>(text: &str) -> crate::error::Result<T>
```



## xjtuportal::crypto::decrypt_text

*Function*

Decrypts a portal hex response into UTF-8 text.

Quoted JSON string bodies are accepted and unwrapped before hex decoding.

# Errors

Returns [`PortalError::Hex`] for malformed hex and [`PortalError::Decrypt`]
for padding, cipher, or UTF-8 failures.

```rust
fn decrypt_text(text: &str) -> crate::error::Result<String>
```



## xjtuportal::crypto::encrypt_json

*Function*

Serializes a value as compact JSON and encrypts it for a portal request.

The returned string is lowercase hex and is ready to be used as the raw HTTP
request body for v3 portal APIs.

# Errors

Returns [`PortalError::Json`] if serialization fails, or
[`PortalError::Encrypt`] if encryption fails.

```rust
fn encrypt_json<T>(value: &T) -> crate::error::Result<String>
```



## xjtuportal::crypto::encrypt_text

*Function*

Encrypts plain text with the portal AES-128-CBC settings.

The plaintext is padded with PKCS#7 and returned as lowercase hex.

# Errors

Returns [`PortalError::Encrypt`] if the cipher backend reports an encryption
failure.

```rust
fn encrypt_text(text: &str) -> crate::error::Result<String>
```



