//! AES-256-GCM encryption for secrets at rest.
//!
//! Provides [`MasterKey`] for wrapping the encryption key,
//! and [`encrypt`] / [`decrypt`] functions that produce unique nonces
//! per value.
//!
//! # Examples
//!
//! ```
//! use ironflow_store::crypto::{MasterKey, encrypt, decrypt};
//!
//! # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
//! let key = MasterKey::from_hex(
//!     "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
//! )?;
//! let (ciphertext, nonce) = encrypt(&key, b"my-secret-value")?;
//! let plaintext = decrypt(&key, &ciphertext, &nonce)?;
//! assert_eq!(plaintext, b"my-secret-value");
//! # Ok(())
//! # }
//! ```

use std::fmt;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use thiserror::Error;

/// AES-256-GCM nonce size in bytes (96 bits).
const NONCE_SIZE: usize = 12;

/// AES-256 key size in bytes (256 bits).
const KEY_SIZE: usize = 32;

/// Errors from cryptographic operations.
///
/// # Examples
///
/// ```
/// use ironflow_store::crypto::CryptoError;
///
/// let err = CryptoError::InvalidKeyLength { expected: 32, got: 16 };
/// assert!(err.to_string().contains("32"));
/// ```
#[derive(Debug, Error)]
pub enum CryptoError {
    /// The key has an invalid length.
    #[error("invalid key length: expected {expected} bytes, got {got}")]
    InvalidKeyLength {
        /// Expected number of bytes.
        expected: usize,
        /// Actual number of bytes.
        got: usize,
    },

    /// The key is not valid hex.
    #[error("invalid hex in key: {0}")]
    InvalidHex(String),

    /// Encryption failed.
    #[error("encryption failed")]
    EncryptionFailed,

    /// Decryption failed (wrong key, corrupted data, or tampered ciphertext).
    #[error("decryption failed")]
    DecryptionFailed,
}

/// A validated AES-256 master key.
///
/// Created from a hex string or raw bytes. The caller is responsible for
/// reading the key from whatever source (env var, vault, config file).
///
/// The [`Debug`] and [`fmt::Display`] implementations redact the key material.
///
/// # Examples
///
/// ```
/// use ironflow_store::crypto::MasterKey;
///
/// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
/// let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// let key = MasterKey::from_hex(hex)?;
/// println!("{key}"); // prints "MasterKey(***)"
/// # Ok(())
/// # }
/// ```
pub struct MasterKey {
    inner: Key<Aes256Gcm>,
}

impl MasterKey {
    /// Create a master key from a hex-encoded string.
    ///
    /// The string must be exactly 64 hex characters (32 bytes decoded).
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidHex`] if the string is not valid hex.
    /// Returns [`CryptoError::InvalidKeyLength`] if the decoded key is not 32 bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::MasterKey;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    /// let key = MasterKey::from_hex(hex)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_hex(hex: &str) -> Result<Self, CryptoError> {
        let bytes = hex_decode(hex)?;
        if bytes.len() != KEY_SIZE {
            return Err(CryptoError::InvalidKeyLength {
                expected: KEY_SIZE,
                got: bytes.len(),
            });
        }
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        Ok(Self { inner: *key })
    }

    /// Create a master key from raw 32 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidKeyLength`] if the slice is not 32 bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::MasterKey;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let bytes = [0u8; 32];
    /// let key = MasterKey::from_bytes(&bytes)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != KEY_SIZE {
            return Err(CryptoError::InvalidKeyLength {
                expected: KEY_SIZE,
                got: bytes.len(),
            });
        }
        let key = Key::<Aes256Gcm>::from_slice(bytes);
        Ok(Self { inner: *key })
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(***)")
    }
}

impl fmt::Display for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(***)")
    }
}

/// Encrypt plaintext with AES-256-GCM using a random nonce.
///
/// Returns `(ciphertext, nonce)`. The nonce is 12 bytes and must be stored
/// alongside the ciphertext for decryption.
///
/// # Errors
///
/// Returns [`CryptoError::EncryptionFailed`] if the cipher operation fails.
///
/// # Examples
///
/// ```
/// use ironflow_store::crypto::{MasterKey, encrypt};
///
/// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
/// let key = MasterKey::from_bytes(&[0u8; 32])?;
/// let (ciphertext, nonce) = encrypt(&key, b"secret")?;
/// assert_eq!(nonce.len(), 12);
/// # Ok(())
/// # }
/// ```
pub fn encrypt(key: &MasterKey, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    let cipher = Aes256Gcm::new(&key.inner);

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::EncryptionFailed)?;

    Ok((ciphertext, nonce_bytes.to_vec()))
}

/// Decrypt ciphertext with AES-256-GCM.
///
/// # Errors
///
/// Returns [`CryptoError::DecryptionFailed`] if the key is wrong, the nonce
/// does not match, or the ciphertext has been tampered with.
///
/// # Examples
///
/// ```
/// use ironflow_store::crypto::{MasterKey, encrypt, decrypt};
///
/// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
/// let key = MasterKey::from_bytes(&[0u8; 32])?;
/// let (ct, nonce) = encrypt(&key, b"hello")?;
/// let pt = decrypt(&key, &ct, &nonce)?;
/// assert_eq!(pt, b"hello");
/// # Ok(())
/// # }
/// ```
pub fn decrypt(key: &MasterKey, ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(&key.inner);
    let nonce = Nonce::from_slice(nonce);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptionFailed)
}

/// Decode a hex string into bytes.
fn hex_decode(hex: &str) -> Result<Vec<u8>, CryptoError> {
    if !hex.len().is_multiple_of(2) {
        return Err(CryptoError::InvalidHex(
            "odd number of characters".to_string(),
        ));
    }

    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| CryptoError::InvalidHex(e.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_HEX_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn master_key_from_hex_valid() {
        let key = MasterKey::from_hex(TEST_HEX_KEY);
        assert!(key.is_ok());
    }

    #[test]
    fn master_key_from_hex_invalid_length() {
        let err = MasterKey::from_hex("0123456789abcdef").unwrap_err();
        assert!(matches!(err, CryptoError::InvalidKeyLength { .. }));
    }

    #[test]
    fn master_key_from_hex_invalid_chars() {
        let bad = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        let err = MasterKey::from_hex(bad).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidHex(_)));
    }

    #[test]
    fn master_key_from_hex_odd_length() {
        let err = MasterKey::from_hex("abc").unwrap_err();
        assert!(matches!(err, CryptoError::InvalidHex(_)));
    }

    #[test]
    fn master_key_from_bytes_valid() {
        let key = MasterKey::from_bytes(&[42u8; 32]);
        assert!(key.is_ok());
    }

    #[test]
    fn master_key_from_bytes_invalid_length() {
        let err = MasterKey::from_bytes(&[0u8; 16]).unwrap_err();
        assert!(matches!(
            err,
            CryptoError::InvalidKeyLength {
                expected: 32,
                got: 16
            }
        ));
    }

    #[test]
    fn master_key_debug_redacts() {
        let key = MasterKey::from_bytes(&[0u8; 32]).unwrap();
        let debug = format!("{key:?}");
        assert_eq!(debug, "MasterKey(***)");
        assert!(!debug.contains("0000"));
    }

    #[test]
    fn master_key_display_redacts() {
        let key = MasterKey::from_bytes(&[0u8; 32]).unwrap();
        let display = format!("{key}");
        assert_eq!(display, "MasterKey(***)");
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = MasterKey::from_hex(TEST_HEX_KEY).unwrap();
        let plaintext = b"my secret token value";

        let (ciphertext, nonce) = encrypt(&key, plaintext).unwrap();
        assert_ne!(ciphertext, plaintext);
        assert_eq!(nonce.len(), NONCE_SIZE);

        let decrypted = decrypt(&key, &ciphertext, &nonce).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_unique_nonces() {
        let key = MasterKey::from_bytes(&[1u8; 32]).unwrap();
        let (_, nonce1) = encrypt(&key, b"same").unwrap();
        let (_, nonce2) = encrypt(&key, b"same").unwrap();
        assert_ne!(nonce1, nonce2);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key1 = MasterKey::from_bytes(&[1u8; 32]).unwrap();
        let key2 = MasterKey::from_bytes(&[2u8; 32]).unwrap();

        let (ciphertext, nonce) = encrypt(&key1, b"secret").unwrap();
        let err = decrypt(&key2, &ciphertext, &nonce).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let key = MasterKey::from_bytes(&[3u8; 32]).unwrap();
        let (mut ciphertext, nonce) = encrypt(&key, b"data").unwrap();

        ciphertext[0] ^= 0xff;
        let err = decrypt(&key, &ciphertext, &nonce).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
    }

    #[test]
    fn decrypt_with_wrong_nonce_fails() {
        let key = MasterKey::from_bytes(&[4u8; 32]).unwrap();
        let (ciphertext, _) = encrypt(&key, b"data").unwrap();

        let wrong_nonce = vec![0u8; NONCE_SIZE];
        let err = decrypt(&key, &ciphertext, &wrong_nonce).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
    }

    #[test]
    fn encrypt_empty_plaintext() {
        let key = MasterKey::from_bytes(&[5u8; 32]).unwrap();
        let (ciphertext, nonce) = encrypt(&key, b"").unwrap();
        let decrypted = decrypt(&key, &ciphertext, &nonce).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn encrypt_large_plaintext() {
        let key = MasterKey::from_bytes(&[6u8; 32]).unwrap();
        let large = vec![0xABu8; 1_000_000];
        let (ciphertext, nonce) = encrypt(&key, &large).unwrap();
        let decrypted = decrypt(&key, &ciphertext, &nonce).unwrap();
        assert_eq!(decrypted, large);
    }

    #[test]
    fn hex_decode_valid() {
        let result = hex_decode("48656c6c6f").unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn hex_decode_empty() {
        let result = hex_decode("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn crypto_error_display() {
        assert!(
            CryptoError::EncryptionFailed
                .to_string()
                .contains("encryption")
        );
        assert!(
            CryptoError::DecryptionFailed
                .to_string()
                .contains("decryption")
        );
    }
}
