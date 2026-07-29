//! AES-256-GCM encryption for secrets at rest.
//!
//! Provides [`MasterKey`] for wrapping a single encryption key, [`KeyRing`] for
//! holding several versioned keys at once, and [`encrypt`] / [`decrypt`]
//! functions that produce unique nonces per value.
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

use std::collections::HashMap;
use std::env::var;
use std::fmt;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use thiserror::Error;
use tracing::warn;

/// Key version assigned to secrets encrypted before key versioning existed.
///
/// The `add_secret_key_version` migration backfills every existing row with
/// this version, and the deprecated single-key `IRONFLOW_SECRET_KEY` variable
/// is interpreted as this version.
pub const LEGACY_KEY_VERSION: i32 = 1;

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

    /// A key ring entry is not of the form `version:hex`.
    #[error("invalid key ring entry {entry:?}: {reason}")]
    InvalidKeyRingEntry {
        /// The offending entry, with the key material stripped.
        entry: String,
        /// Why the entry was rejected.
        reason: String,
    },

    /// The same version appears more than once in the key ring.
    #[error("duplicate key version {0} in key ring")]
    DuplicateKeyVersion(i32),

    /// The key ring contains no key at all.
    #[error("key ring is empty")]
    EmptyKeyRing,

    /// The requested active version is not present in the key ring.
    #[error("active key version {requested} is not in the key ring (available: {available})")]
    ActiveVersionMissing {
        /// The version asked for.
        requested: i32,
        /// Comma-separated list of the versions actually configured.
        available: String,
    },

    /// The active key version is not a positive integer.
    #[error("{SECRET_ACTIVE_VERSION_ENV} must be a positive integer, got {0:?}")]
    InvalidActiveVersion(String),
}

/// Environment variable holding the versioned key ring (`version:hex,...`).
pub const SECRET_KEYS_ENV: &str = "IRONFLOW_SECRET_KEYS";

/// Environment variable selecting the version used for new encryptions.
pub const SECRET_ACTIVE_VERSION_ENV: &str = "IRONFLOW_SECRET_ACTIVE_KEY_VERSION";

/// Deprecated environment variable holding a single, unversioned key.
///
/// Still honoured, and interpreted as [`LEGACY_KEY_VERSION`], so existing
/// deployments keep working. Ignored when [`SECRET_KEYS_ENV`] is set.
pub const SECRET_KEY_ENV: &str = "IRONFLOW_SECRET_KEY";

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

/// A set of versioned encryption keys, one of which is active.
///
/// Every key in the ring can decrypt; only the active one encrypts. This is
/// what makes key rotation possible without downtime: add a key, make it
/// active, re-encrypt the stock in the background, then drop the old key.
///
/// The [`Debug`] and [`fmt::Display`] implementations redact the key material
/// and show only the versions.
///
/// # Examples
///
/// ```
/// use ironflow_store::crypto::KeyRing;
///
/// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
/// let spec = format!("1:{},2:{}", "aa".repeat(32), "bb".repeat(32));
/// let ring = KeyRing::from_spec(&spec, Some(2))?;
///
/// assert_eq!(ring.active_version(), 2);
/// assert_eq!(ring.versions(), vec![1, 2]);
/// assert!(ring.key_for(1).is_some());
/// # Ok(())
/// # }
/// ```
pub struct KeyRing {
    keys: HashMap<i32, MasterKey>,
    active_version: i32,
}

impl KeyRing {
    /// Build a ring holding a single key, active at [`LEGACY_KEY_VERSION`].
    ///
    /// This is how the deprecated single-key configuration is represented, so
    /// that existing deployments keep working unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::{KeyRing, MasterKey, LEGACY_KEY_VERSION};
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let ring = KeyRing::single(MasterKey::from_bytes(&[7u8; 32])?);
    /// assert_eq!(ring.active_version(), LEGACY_KEY_VERSION);
    /// # Ok(())
    /// # }
    /// ```
    pub fn single(key: MasterKey) -> Self {
        Self::with_active(LEGACY_KEY_VERSION, key)
    }

    /// Build a ring holding a single key at an explicit version.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::{KeyRing, MasterKey};
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let ring = KeyRing::with_active(4, MasterKey::from_bytes(&[7u8; 32])?);
    /// assert_eq!(ring.active_version(), 4);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_active(version: i32, key: MasterKey) -> Self {
        let mut keys = HashMap::new();
        keys.insert(version, key);
        Self {
            keys,
            active_version: version,
        }
    }

    /// Parse a key ring from a `version:hex,version:hex` specification.
    ///
    /// Whitespace around entries and around the `:` separator is ignored.
    /// Each key is 64 hex characters (32 bytes). When `active` is `None`, the
    /// highest version in the ring becomes active.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::EmptyKeyRing`] if the spec holds no entry,
    /// [`CryptoError::InvalidKeyRingEntry`] if an entry is malformed or its
    /// version is not a positive integer, [`CryptoError::DuplicateKeyVersion`]
    /// if a version is repeated, [`CryptoError::InvalidHex`] or
    /// [`CryptoError::InvalidKeyLength`] if the key material is not 32 hex
    /// bytes, and [`CryptoError::ActiveVersionMissing`] if `active` names a
    /// version the ring does not hold.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::KeyRing;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let spec = format!("1:{}, 2:{}", "aa".repeat(32), "bb".repeat(32));
    /// let ring = KeyRing::from_spec(&spec, None)?;
    /// assert_eq!(ring.active_version(), 2); // highest version wins by default
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_spec(spec: &str, active: Option<i32>) -> Result<Self, CryptoError> {
        let mut keys: HashMap<i32, MasterKey> = HashMap::new();

        for raw in spec.split(',') {
            let entry = raw.trim();
            if entry.is_empty() {
                continue;
            }

            let (version_part, key_part) =
                entry
                    .split_once(':')
                    .ok_or_else(|| CryptoError::InvalidKeyRingEntry {
                        entry: redact_entry(entry),
                        reason: "expected the form <version>:<hex key>".to_string(),
                    })?;

            let version = version_part.trim().parse::<i32>().map_err(|_| {
                CryptoError::InvalidKeyRingEntry {
                    entry: redact_entry(entry),
                    reason: format!("version {:?} is not an integer", version_part.trim()),
                }
            })?;

            if version < 1 {
                return Err(CryptoError::InvalidKeyRingEntry {
                    entry: redact_entry(entry),
                    reason: "version must be 1 or greater".to_string(),
                });
            }

            let key = MasterKey::from_hex(key_part.trim())?;

            if keys.insert(version, key).is_some() {
                return Err(CryptoError::DuplicateKeyVersion(version));
            }
        }

        if keys.is_empty() {
            return Err(CryptoError::EmptyKeyRing);
        }

        let active_version = match active {
            Some(v) => v,
            None => *keys.keys().max().expect("ring is not empty"),
        };

        if !keys.contains_key(&active_version) {
            let mut available: Vec<i32> = keys.keys().copied().collect();
            available.sort_unstable();
            return Err(CryptoError::ActiveVersionMissing {
                requested: active_version,
                available: join_versions(&available),
            });
        }

        Ok(Self {
            keys,
            active_version,
        })
    }

    /// Build the key ring from the environment.
    ///
    /// Reads [`SECRET_KEYS_ENV`], [`SECRET_ACTIVE_VERSION_ENV`], and the
    /// deprecated [`SECRET_KEY_ENV`]. Returns `Ok(None)` when none is set,
    /// meaning the secret store stays disabled.
    ///
    /// # Errors
    ///
    /// See [`KeyRing::from_env_values`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::crypto::KeyRing;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// match KeyRing::from_env()? {
    ///     Some(ring) => println!("encrypting with version {}", ring.active_version()),
    ///     None => println!("secret store disabled"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Option<Self>, CryptoError> {
        Self::from_env_values(
            var(SECRET_KEYS_ENV).ok().as_deref(),
            var(SECRET_ACTIVE_VERSION_ENV).ok().as_deref(),
            var(SECRET_KEY_ENV).ok().as_deref(),
        )
    }

    /// Build the key ring from explicit configuration values.
    ///
    /// `keys` takes precedence over the deprecated single `legacy_key`; when
    /// both are given, `legacy_key` is ignored with a warning rather than
    /// rejected, so a deployment carrying an inherited variable still boots.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidActiveVersion`] if `active` is not a
    /// positive integer, plus any error from [`KeyRing::from_spec`] or
    /// [`MasterKey::from_hex`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::KeyRing;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let spec = format!("1:{},2:{}", "aa".repeat(32), "bb".repeat(32));
    /// let ring = KeyRing::from_env_values(Some(&spec), Some("2"), None)?
    ///     .expect("a ring was configured");
    /// assert_eq!(ring.active_version(), 2);
    ///
    /// assert!(KeyRing::from_env_values(None, None, None)?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env_values(
        keys: Option<&str>,
        active: Option<&str>,
        legacy_key: Option<&str>,
    ) -> Result<Option<Self>, CryptoError> {
        let active = match active.map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => Some(
                raw.parse::<i32>()
                    .ok()
                    .filter(|v| *v >= 1)
                    .ok_or_else(|| CryptoError::InvalidActiveVersion(raw.to_string()))?,
            ),
            None => None,
        };

        if let Some(spec) = keys.map(str::trim).filter(|s| !s.is_empty()) {
            if legacy_key.is_some() {
                warn!(
                    "{SECRET_KEY_ENV} is set but ignored: {SECRET_KEYS_ENV} takes precedence. \
                     {SECRET_KEY_ENV} is deprecated, remove it."
                );
            }
            return Ok(Some(Self::from_spec(spec, active)?));
        }

        match legacy_key.map(str::trim).filter(|s| !s.is_empty()) {
            Some(hex) => {
                let version = active.unwrap_or(LEGACY_KEY_VERSION);
                Ok(Some(Self::with_active(version, MasterKey::from_hex(hex)?)))
            }
            None => Ok(None),
        }
    }

    /// The version every new encryption uses.
    pub fn active_version(&self) -> i32 {
        self.active_version
    }

    /// The key for a given version, or `None` if the ring does not hold it.
    pub fn key_for(&self, version: i32) -> Option<&MasterKey> {
        self.keys.get(&version)
    }

    /// The key used for new encryptions.
    ///
    /// # Panics
    ///
    /// Never in practice: the constructors guarantee the active version is in
    /// the ring, and the ring is immutable afterwards.
    pub fn active_key(&self) -> &MasterKey {
        self.keys
            .get(&self.active_version)
            .expect("active version is always present in the ring")
    }

    /// All versions held by the ring, ascending.
    pub fn versions(&self) -> Vec<i32> {
        let mut versions: Vec<i32> = self.keys.keys().copied().collect();
        versions.sort_unstable();
        versions
    }

    /// Versions present in `used` that the ring cannot decrypt, ascending.
    ///
    /// A non-empty result means some stored secret is unreadable with the
    /// current configuration -- the server must refuse to start.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::{KeyRing, MasterKey};
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let ring = KeyRing::single(MasterKey::from_bytes(&[7u8; 32])?);
    /// assert_eq!(ring.missing_versions(&[1, 2, 3]), vec![2, 3]);
    /// assert!(ring.missing_versions(&[1]).is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn missing_versions(&self, used: &[i32]) -> Vec<i32> {
        let mut missing: Vec<i32> = used
            .iter()
            .copied()
            .filter(|v| !self.keys.contains_key(v))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        missing
    }

    /// Versions that can be removed from the configuration without breaking
    /// the next startup: configured, not active, and unused by any secret.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::crypto::KeyRing;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let spec = format!("1:{},2:{}", "aa".repeat(32), "bb".repeat(32));
    /// let ring = KeyRing::from_spec(&spec, Some(2))?;
    /// assert_eq!(ring.retirable_versions(&[2]), vec![1]);
    /// assert!(ring.retirable_versions(&[1, 2]).is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn retirable_versions(&self, used: &[i32]) -> Vec<i32> {
        let mut retirable: Vec<i32> = self
            .versions()
            .into_iter()
            .filter(|v| *v != self.active_version && !used.contains(v))
            .collect();
        retirable.sort_unstable();
        retirable
    }
}

impl fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "KeyRing {{ versions: [{}], active: {}, keys: *** }}",
            join_versions(&self.versions()),
            self.active_version
        )
    }
}

impl fmt::Display for KeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "KeyRing(versions=[{}], active={})",
            join_versions(&self.versions()),
            self.active_version
        )
    }
}

/// Render versions as a comma-separated list for error messages.
pub(crate) fn join_versions(versions: &[i32]) -> String {
    versions
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Strip key material from a key ring entry so it can appear in an error.
fn redact_entry(entry: &str) -> String {
    match entry.split_once(':') {
        Some((version, _)) => format!("{}:***", version.trim()),
        None => "***".to_string(),
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

    fn hex_key(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn two_key_spec() -> String {
        format!("1:{},2:{}", hex_key(0xaa), hex_key(0xbb))
    }

    #[test]
    fn key_ring_single_uses_legacy_version() {
        let ring = KeyRing::single(MasterKey::from_bytes(&[7u8; 32]).unwrap());
        assert_eq!(ring.active_version(), LEGACY_KEY_VERSION);
        assert_eq!(ring.versions(), vec![1]);
        assert!(ring.key_for(1).is_some());
        assert!(ring.key_for(2).is_none());
    }

    #[test]
    fn key_ring_with_active_uses_given_version() {
        let ring = KeyRing::with_active(4, MasterKey::from_bytes(&[7u8; 32]).unwrap());
        assert_eq!(ring.active_version(), 4);
        assert_eq!(ring.versions(), vec![4]);
    }

    #[test]
    fn key_ring_from_spec_multiple_keys() {
        let ring = KeyRing::from_spec(&two_key_spec(), Some(2)).unwrap();
        assert_eq!(ring.active_version(), 2);
        assert_eq!(ring.versions(), vec![1, 2]);
        assert!(ring.key_for(1).is_some());
        assert!(ring.key_for(2).is_some());
    }

    #[test]
    fn key_ring_from_spec_defaults_to_highest_version() {
        let spec = format!("3:{},1:{}", hex_key(0xaa), hex_key(0xbb));
        let ring = KeyRing::from_spec(&spec, None).unwrap();
        assert_eq!(ring.active_version(), 3);
    }

    #[test]
    fn key_ring_from_spec_tolerates_whitespace() {
        let spec = format!("  1 : {} ,  2 : {}  ", hex_key(0xaa), hex_key(0xbb));
        let ring = KeyRing::from_spec(&spec, Some(1)).unwrap();
        assert_eq!(ring.versions(), vec![1, 2]);
    }

    #[test]
    fn key_ring_from_spec_ignores_trailing_separator() {
        let spec = format!("1:{},", hex_key(0xaa));
        let ring = KeyRing::from_spec(&spec, None).unwrap();
        assert_eq!(ring.versions(), vec![1]);
    }

    #[test]
    fn key_ring_from_spec_rejects_empty() {
        assert!(matches!(
            KeyRing::from_spec("", None).unwrap_err(),
            CryptoError::EmptyKeyRing
        ));
        assert!(matches!(
            KeyRing::from_spec("  ,  ", None).unwrap_err(),
            CryptoError::EmptyKeyRing
        ));
    }

    #[test]
    fn key_ring_from_spec_rejects_missing_separator() {
        let err = KeyRing::from_spec(&hex_key(0xaa), None).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidKeyRingEntry { .. }));
    }

    #[test]
    fn key_ring_from_spec_rejects_non_numeric_version() {
        let spec = format!("v1:{}", hex_key(0xaa));
        let err = KeyRing::from_spec(&spec, None).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidKeyRingEntry { .. }));
        assert!(err.to_string().contains("v1"));
    }

    #[test]
    fn key_ring_from_spec_rejects_non_positive_version() {
        for version in ["0", "-1"] {
            let spec = format!("{version}:{}", hex_key(0xaa));
            let err = KeyRing::from_spec(&spec, None).unwrap_err();
            assert!(matches!(err, CryptoError::InvalidKeyRingEntry { .. }));
        }
    }

    #[test]
    fn key_ring_from_spec_rejects_duplicate_version() {
        let spec = format!("1:{},1:{}", hex_key(0xaa), hex_key(0xbb));
        let err = KeyRing::from_spec(&spec, None).unwrap_err();
        assert!(matches!(err, CryptoError::DuplicateKeyVersion(1)));
    }

    #[test]
    fn key_ring_from_spec_rejects_invalid_hex() {
        let spec = format!("1:{}", "zz".repeat(32));
        let err = KeyRing::from_spec(&spec, None).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidHex(_)));
    }

    #[test]
    fn key_ring_from_spec_rejects_short_key() {
        let err = KeyRing::from_spec("1:abcd", None).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidKeyLength { .. }));
    }

    #[test]
    fn key_ring_from_spec_rejects_unknown_active_version() {
        let err = KeyRing::from_spec(&two_key_spec(), Some(9)).unwrap_err();
        assert!(matches!(
            err,
            CryptoError::ActiveVersionMissing { requested: 9, .. }
        ));
        let msg = err.to_string();
        assert!(msg.contains('9'));
        assert!(msg.contains("1, 2"));
    }

    #[test]
    fn key_ring_error_never_leaks_key_material() {
        let key = hex_key(0xaa);
        let spec = format!("bad:{key}");
        let msg = KeyRing::from_spec(&spec, None).unwrap_err().to_string();
        assert!(!msg.contains(&key));
        assert!(msg.contains("***"));
    }

    #[test]
    fn key_ring_active_key_matches_active_version() {
        let ring = KeyRing::from_spec(&two_key_spec(), Some(2)).unwrap();
        let (ciphertext, nonce) = encrypt(ring.active_key(), b"payload").unwrap();

        let with_v2 = decrypt(ring.key_for(2).unwrap(), &ciphertext, &nonce).unwrap();
        assert_eq!(with_v2, b"payload");

        let with_v1 = decrypt(ring.key_for(1).unwrap(), &ciphertext, &nonce);
        assert!(with_v1.is_err());
    }

    #[test]
    fn key_ring_missing_versions() {
        let ring = KeyRing::from_spec(&two_key_spec(), Some(2)).unwrap();
        assert!(ring.missing_versions(&[]).is_empty());
        assert!(ring.missing_versions(&[1, 2]).is_empty());
        assert_eq!(ring.missing_versions(&[1, 3]), vec![3]);
        assert_eq!(ring.missing_versions(&[5, 4, 3]), vec![3, 4, 5]);
    }

    #[test]
    fn key_ring_missing_versions_deduplicates() {
        let ring = KeyRing::single(MasterKey::from_bytes(&[7u8; 32]).unwrap());
        assert_eq!(ring.missing_versions(&[3, 3, 3]), vec![3]);
    }

    #[test]
    fn key_ring_retirable_versions() {
        let ring = KeyRing::from_spec(&two_key_spec(), Some(2)).unwrap();
        assert_eq!(ring.retirable_versions(&[2]), vec![1]);
        assert!(ring.retirable_versions(&[1, 2]).is_empty());
        // The active version is never retirable, even with no secret using it.
        assert_eq!(ring.retirable_versions(&[]), vec![1]);
    }

    #[test]
    fn from_env_values_without_anything_disables_the_store() {
        assert!(
            KeyRing::from_env_values(None, None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            KeyRing::from_env_values(Some("  "), Some(""), Some(" "))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn from_env_values_uses_the_key_ring() {
        let ring = KeyRing::from_env_values(Some(&two_key_spec()), Some("2"), None)
            .unwrap()
            .unwrap();
        assert_eq!(ring.active_version(), 2);
        assert_eq!(ring.versions(), vec![1, 2]);
    }

    #[test]
    fn from_env_values_defaults_active_to_highest_version() {
        let ring = KeyRing::from_env_values(Some(&two_key_spec()), None, None)
            .unwrap()
            .unwrap();
        assert_eq!(ring.active_version(), 2);
    }

    #[test]
    fn from_env_values_reads_the_legacy_key_as_version_one() {
        let ring = KeyRing::from_env_values(None, None, Some(&hex_key(0xaa)))
            .unwrap()
            .unwrap();
        assert_eq!(ring.active_version(), LEGACY_KEY_VERSION);
        assert_eq!(ring.versions(), vec![1]);
    }

    #[test]
    fn from_env_values_gives_the_key_ring_precedence_over_the_legacy_key() {
        let ring = KeyRing::from_env_values(Some(&two_key_spec()), Some("1"), Some(&hex_key(0xcc)))
            .unwrap()
            .unwrap();

        // Version 1 comes from the ring, not from the legacy variable.
        assert_eq!(ring.versions(), vec![1, 2]);
        let (ciphertext, nonce) = encrypt(ring.active_key(), b"payload").unwrap();
        let ring_v1 = MasterKey::from_hex(&hex_key(0xaa)).unwrap();
        assert!(decrypt(&ring_v1, &ciphertext, &nonce).is_ok());
    }

    #[test]
    fn from_env_values_rejects_a_non_numeric_active_version() {
        let err = KeyRing::from_env_values(Some(&two_key_spec()), Some("two"), None).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidActiveVersion(_)));
        assert!(
            err.to_string()
                .contains("IRONFLOW_SECRET_ACTIVE_KEY_VERSION")
        );
    }

    #[test]
    fn from_env_values_rejects_a_non_positive_active_version() {
        for raw in ["0", "-3"] {
            let err = KeyRing::from_env_values(Some(&two_key_spec()), Some(raw), None).unwrap_err();
            assert!(matches!(err, CryptoError::InvalidActiveVersion(_)));
        }
    }

    #[test]
    fn from_env_values_rejects_an_invalid_legacy_key() {
        let err = KeyRing::from_env_values(None, None, Some("not-hex")).unwrap_err();
        assert!(matches!(
            err,
            CryptoError::InvalidHex(_) | CryptoError::InvalidKeyLength { .. }
        ));
    }

    #[test]
    fn key_ring_debug_and_display_redact() {
        let ring = KeyRing::from_spec(&two_key_spec(), Some(2)).unwrap();
        let debug = format!("{ring:?}");
        let display = format!("{ring}");

        assert!(debug.contains("1, 2"));
        assert!(debug.contains("active: 2"));
        assert!(debug.contains("***"));
        assert!(!debug.contains(&hex_key(0xaa)));

        assert!(display.contains("active=2"));
        assert!(!display.contains(&hex_key(0xbb)));
    }
}
