//! Minimal JWT (RS256 for agents, HS256 for UI sessions).
//!
//! The header `alg` is checked against the algorithm the caller expects, so an
//! attacker cannot downgrade RS256 to HS256 or to `none`.
use crate::crypto::{self, CryptoError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64U, Engine};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("malformed token")]
    Malformed,
    #[error("unexpected algorithm")]
    Algorithm,
    #[error("invalid signature")]
    Signature,
    #[error("token expired")]
    Expired,
    #[error("token not yet valid")]
    NotYetValid,
    #[error("crypto: {0}")]
    Crypto(#[from] CryptoError),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alg {
    RS256,
    HS256,
}

impl Alg {
    fn name(self) -> &'static str {
        match self {
            Alg::RS256 => "RS256",
            Alg::HS256 => "HS256",
        }
    }
}

/// Registered claims used across the platform.
#[derive(Serialize, serde::Deserialize, Debug, Clone)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    /// Optional free-form scope (e.g. user role id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

fn encode<T: Serialize>(alg: Alg, claims: &T, signer: impl FnOnce(&[u8]) -> Result<Vec<u8>, JwtError>) -> Result<String, JwtError> {
    let header = serde_json::json!({ "alg": alg.name(), "typ": "JWT" });
    let h = B64U.encode(serde_json::to_vec(&header).map_err(|_| JwtError::Malformed)?);
    let p = B64U.encode(serde_json::to_vec(claims).map_err(|_| JwtError::Malformed)?);
    let signing_input = format!("{h}.{p}");
    let sig = signer(signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", B64U.encode(sig)))
}

pub fn sign_rs256<T: Serialize>(private_pem: &str, claims: &T) -> Result<String, JwtError> {
    encode(Alg::RS256, claims, |m| Ok(crypto::rsa_sign(private_pem, m)?))
}

pub fn sign_hs256<T: Serialize>(secret: &[u8], claims: &T) -> Result<String, JwtError> {
    encode(Alg::HS256, claims, |m| Ok(hex::decode(crypto::hmac_sha256_hex(secret, m)).expect("hex")))
}

/// Split and check the header; returns (signing_input, payload bytes, signature bytes).
fn split(token: &str, expect: Alg) -> Result<(String, Vec<u8>, Vec<u8>), JwtError> {
    let mut it = token.split('.');
    let (h, p, s) = match (it.next(), it.next(), it.next(), it.next()) {
        (Some(h), Some(p), Some(s), None) => (h, p, s),
        _ => return Err(JwtError::Malformed),
    };
    let header: serde_json::Value = serde_json::from_slice(&B64U.decode(h).map_err(|_| JwtError::Malformed)?).map_err(|_| JwtError::Malformed)?;
    if header.get("alg").and_then(|v| v.as_str()) != Some(expect.name()) {
        return Err(JwtError::Algorithm);
    }
    Ok((
        format!("{h}.{p}"),
        B64U.decode(p).map_err(|_| JwtError::Malformed)?,
        B64U.decode(s).map_err(|_| JwtError::Malformed)?,
    ))
}

/// Read the `sub` claim without verifying — needed to look up the verification key.
pub fn peek_subject(token: &str) -> Option<String> {
    let p = token.split('.').nth(1)?;
    let v: serde_json::Value = serde_json::from_slice(&B64U.decode(p).ok()?).ok()?;
    v.get("sub")?.as_str().map(str::to_owned)
}

fn check_times(c: &Claims, now: i64, leeway: i64) -> Result<(), JwtError> {
    if c.exp + leeway < now {
        return Err(JwtError::Expired);
    }
    if c.iat - leeway > now {
        return Err(JwtError::NotYetValid);
    }
    Ok(())
}

pub fn verify_rs256<T: DeserializeOwned>(public_pem: &str, token: &str) -> Result<T, JwtError> {
    let (input, payload, sig) = split(token, Alg::RS256)?;
    crypto::rsa_verify(public_pem, input.as_bytes(), &sig).map_err(|_| JwtError::Signature)?;
    serde_json::from_slice(&payload).map_err(|_| JwtError::Malformed)
}

pub fn verify_hs256<T: DeserializeOwned>(secret: &[u8], token: &str) -> Result<T, JwtError> {
    let (input, payload, sig) = split(token, Alg::HS256)?;
    if !crypto::hmac_sha256_verify_hex(secret, input.as_bytes(), &hex::encode(sig)) {
        return Err(JwtError::Signature);
    }
    serde_json::from_slice(&payload).map_err(|_| JwtError::Malformed)
}

/// Verify an RS256 token carrying [`Claims`] and check its validity window.
pub fn verify_claims_rs256(public_pem: &str, token: &str, now: i64, leeway: i64) -> Result<Claims, JwtError> {
    let c: Claims = verify_rs256(public_pem, token)?;
    check_times(&c, now, leeway)?;
    Ok(c)
}

pub fn verify_claims_hs256(secret: &[u8], token: &str, now: i64, leeway: i64) -> Result<Claims, JwtError> {
    let c: Claims = verify_hs256(secret, token)?;
    check_times(&c, now, leeway)?;
    Ok(c)
}
