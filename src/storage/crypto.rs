use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{fmt, sync::Arc};

/// AES-256-GCM envelope for API keys at rest.
///
/// Every value gets a fresh random 96-bit nonce and is bound to its owner
/// via associated data (e.g. `user:123`), so a ciphertext copied onto a
/// different row fails authentication instead of silently decrypting.
pub struct Vault {
    cipher: Aes256Gcm,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Vault(***)")
    }
}

impl Vault {
    pub fn from_base64(master_key: &str) -> Result<Self, String> {
        let bytes = STANDARD
            .decode(master_key.trim())
            .map_err(|_| "MASTER_KEY must be valid base64 (use `openssl rand -base64 32`).")?;
        if bytes.len() != 32 {
            return Err(format!(
                "MASTER_KEY must decode to exactly 32 bytes (got {}). Use `openssl rand -base64 32`.",
                bytes.len()
            ));
        }
        let cipher = Aes256Gcm::new_from_slice(&bytes).map_err(|_| "invalid MASTER_KEY")?;
        Ok(Self { cipher })
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| "encryption failed".to_string())?;
        Ok((ct, nonce.to_vec()))
    }

    pub fn decrypt(&self, ciphertext: &[u8], nonce: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
        if nonce.len() != 12 {
            return Err("invalid nonce length".into());
        }
        self.cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| "decryption failed (wrong MASTER_KEY or corrupted row)".to_string())
    }
}

/// An API key held in memory. `Debug`/`Display` never reveal it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(Arc<str>);

impl ApiKey {
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        let arc: Arc<str> = key.into();
        // Fail fast on empty keys in debug; production callers should use
        // `try_new` for a proper error. Stored keys are validated in
        // `Store::set_key`.
        debug_assert!(!arc.trim().is_empty(), "ApiKey must not be empty");
        Self(arc)
    }

    /// Validated constructor: rejects empty/whitespace keys.
    pub fn try_new(key: impl Into<Arc<str>>) -> Result<Self, String> {
        let arc: Arc<str> = key.into();
        if arc.trim().is_empty() {
            return Err("API key must not be empty".into());
        }
        Ok(Self(arc))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Non-reversible display hint like `AIza…x9Qk`.
    pub fn hint(&self) -> String {
        let k = self.expose();
        let n = k.chars().count();
        if n <= 8 {
            return "…".to_string();
        }
        let head: String = k.chars().take(4).collect();
        let tail: String = k.chars().skip(n - 4).collect();
        format!("{head}…{tail}")
    }

    /// Stable in-process fingerprint used for per-key concurrency limits.
    /// Deterministic FNV-1a 64-bit over the key bytes: same key always maps
    /// to the same gate/pause entry within and across runs. (Fairness only —
    /// not a cryptographic identifier; collisions only merge rate limits.)
    pub fn fingerprint(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let mut h = FNV_OFFSET;
        for b in self.0.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(FNV_PRIME);
        }
        // Mix in length to separate prefixes from full keys.
        h ^= self.0.len() as u64;
        h = h.wrapping_mul(FNV_PRIME);
        h
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({})", self.hint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> Vault {
        Vault::from_base64(&STANDARD.encode([7u8; 32])).unwrap()
    }

    #[test]
    fn roundtrip() {
        let v = vault();
        let (ct, n) = v.encrypt(b"secret-key", b"user:1").unwrap();
        assert_ne!(ct, b"secret-key");
        assert_eq!(v.decrypt(&ct, &n, b"user:1").unwrap(), b"secret-key");
    }

    #[test]
    fn nonces_are_unique() {
        let v = vault();
        let (c1, n1) = v.encrypt(b"same", b"a").unwrap();
        let (c2, n2) = v.encrypt(b"same", b"a").unwrap();
        assert_ne!(n1, n2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn aad_binds_owner() {
        let v = vault();
        let (ct, n) = v.encrypt(b"secret", b"user:1").unwrap();
        assert!(v.decrypt(&ct, &n, b"user:2").is_err());
    }

    #[test]
    fn wrong_master_key_fails() {
        let (ct, n) = vault().encrypt(b"secret", b"a").unwrap();
        let other = Vault::from_base64(&STANDARD.encode([9u8; 32])).unwrap();
        assert!(other.decrypt(&ct, &n, b"a").is_err());
    }

    #[test]
    fn rejects_short_master_key() {
        assert!(Vault::from_base64(&STANDARD.encode([1u8; 16])).is_err());
        assert!(Vault::from_base64("not base64!!").is_err());
    }

    #[test]
    fn api_key_never_debug_prints() {
        let k = ApiKey::new("AIzaSyD-secret-middle-part-1234");
        let dbg = format!("{k:?}");
        assert!(!dbg.contains("secret"));
        assert!(dbg.contains("AIza"));
        assert!(dbg.contains("1234"));
    }
}
