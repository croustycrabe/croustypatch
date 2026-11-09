//! Application-level secrets: AES-256-GCM column encryption, Argon2id password hashing and
//! zeroising containers so key material does not linger in RAM.
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use argon2::password_hash::{rand_core::OsRng as PhOsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("decryption failed")]
    Decrypt,
    #[error("master key invalid: {0}")]
    MasterKey(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// The master key. Never printed (`Debug` is redacted) and wiped on drop.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

impl MasterKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(Zeroizing::new(b))
    }

    pub fn from_b64(s: &str) -> Result<Self, SecretError> {
        let mut raw = B64.decode(s.trim()).map_err(|e| SecretError::MasterKey(e.to_string()))?;
        if raw.len() != 32 {
            raw.zeroize();
            return Err(SecretError::MasterKey("expected 32 bytes (base64)".into()));
        }
        let mut k = [0u8; 32];
        k.copy_from_slice(&raw);
        raw.zeroize();
        Ok(Self::from_bytes(k))
    }

    /// Load from env value, else from `path` (mode 0600), else generate and persist to `path`.
    pub fn load_or_create(env_value: Option<&str>, path: &Path) -> Result<Self, SecretError> {
        if let Some(v) = env_value.filter(|v| !v.is_empty()) {
            return Self::from_b64(v);
        }
        if path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    return Err(SecretError::MasterKey(format!(
                        "{} is accessible by group/others (mode {:o}); chmod 600 it",
                        path.display(),
                        mode
                    )));
                }
            }
            return Self::from_b64(&std::fs::read_to_string(path)?);
        }
        let mut k = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut k);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(path, B64.encode(k).as_bytes())?;
        Ok(Self::from_bytes(k))
    }

    /// Derive an independent sub-key for a purpose label.
    pub fn derive(&self, label: &str) -> Zeroizing<Vec<u8>> {
        let raw = croustypatch_common::crypto::hmac_sha256_hex(&self.0[..], label.as_bytes());
        Zeroizing::new(hex::decode(raw).expect("hex"))
    }

    /// AES-256-GCM, output = base64(nonce || ciphertext).
    pub fn encrypt(&self, plaintext: &[u8]) -> String {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.0[..]));
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher.encrypt(&nonce, plaintext).expect("AES-GCM encryption cannot fail for in-memory buffers");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        B64.encode(out)
    }

    pub fn decrypt(&self, encoded: &str) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        let raw = B64.decode(encoded).map_err(|_| SecretError::Decrypt)?;
        if raw.len() < 12 + 16 {
            return Err(SecretError::Decrypt);
        }
        let (n, ct) = raw.split_at(12);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.0[..]));
        cipher.decrypt(Nonce::from_slice(n), ct).map(Zeroizing::new).map_err(|_| SecretError::Decrypt)
    }

    pub fn decrypt_string(&self, encoded: &str) -> Result<Zeroizing<String>, SecretError> {
        let b = self.decrypt(encoded)?;
        String::from_utf8(b.to_vec()).map(Zeroizing::new).map_err(|_| SecretError::Decrypt)
    }
}

pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(data)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut PhOsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).expect("argon2 hashing").to_string()
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false)
}
