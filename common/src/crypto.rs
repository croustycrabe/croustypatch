//! RSA keys, signatures, hashing and HMAC helpers.
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("key error: {0}")]
    Key(String),
    #[error("bad signature")]
    BadSignature,
    #[error("encoding error: {0}")]
    Encoding(String),
}

pub struct KeyPair {
    pub private_pem: String,
    pub public_pem: String,
}

/// Generate a fresh RSA keypair (PKCS#8 / SPKI PEM).
pub fn generate_rsa(bits: usize) -> Result<KeyPair, CryptoError> {
    let sk = RsaPrivateKey::new(&mut rand::rngs::OsRng, bits).map_err(|e| CryptoError::Key(e.to_string()))?;
    let private_pem = sk
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| CryptoError::Key(e.to_string()))?
        .to_string();
    let public_pem = RsaPublicKey::from(&sk)
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| CryptoError::Key(e.to_string()))?;
    Ok(KeyPair { private_pem, public_pem })
}

/// Parse a public key PEM and enforce a minimum modulus size.
pub fn check_public_pem(public_pem: &str, min_bits: usize) -> Result<(), CryptoError> {
    use rsa::traits::PublicKeyParts;
    let pk = RsaPublicKey::from_public_key_pem(public_pem).map_err(|e| CryptoError::Key(e.to_string()))?;
    if pk.size() * 8 < min_bits {
        return Err(CryptoError::Key(format!("RSA key must be at least {min_bits} bits")));
    }
    Ok(())
}

/// RSASSA-PKCS1-v1_5 / SHA-256 signature, raw bytes.
pub fn rsa_sign(private_pem: &str, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let sk = RsaPrivateKey::from_pkcs8_pem(private_pem).map_err(|e| CryptoError::Key(e.to_string()))?;
    Ok(SigningKey::<Sha256>::new(sk).sign(msg).to_vec())
}

pub fn rsa_verify(public_pem: &str, msg: &[u8], sig: &[u8]) -> Result<(), CryptoError> {
    let pk = RsaPublicKey::from_public_key_pem(public_pem).map_err(|e| CryptoError::Key(e.to_string()))?;
    let sig = Signature::try_from(sig).map_err(|_| CryptoError::BadSignature)?;
    VerifyingKey::<Sha256>::new(pk).verify(msg, &sig).map_err(|_| CryptoError::BadSignature)
}

/// Sign and return standard base64.
pub fn rsa_sign_b64(private_pem: &str, msg: &[u8]) -> Result<String, CryptoError> {
    Ok(B64.encode(rsa_sign(private_pem, msg)?))
}

pub fn rsa_verify_b64(public_pem: &str, msg: &[u8], sig_b64: &str) -> Result<(), CryptoError> {
    let sig = B64.decode(sig_b64).map_err(|e| CryptoError::Encoding(e.to_string()))?;
    rsa_verify(public_pem, msg, &sig)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    m.update(data);
    hex::encode(m.finalize().into_bytes())
}

/// Constant-time HMAC check.
pub fn hmac_sha256_verify_hex(key: &[u8], data: &[u8], sig_hex: &str) -> bool {
    let Ok(sig) = hex::decode(sig_hex) else { return false };
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    m.update(data);
    m.verify_slice(&sig).is_ok()
}

/// Constant-time byte comparison for secrets (tokens, hashes).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// URL-safe random token with `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}
