//! Tests for `crypto`.
use croustypatch_common::crypto::*;

#[test]
fn rsa_roundtrip_and_tamper() {
    let kp = generate_rsa(2048).unwrap();
    let sig = rsa_sign_b64(&kp.private_pem, b"hello").unwrap();
    rsa_verify_b64(&kp.public_pem, b"hello", &sig).unwrap();
    assert!(rsa_verify_b64(&kp.public_pem, b"hellp", &sig).is_err());
    let other = generate_rsa(2048).unwrap();
    assert!(rsa_verify_b64(&other.public_pem, b"hello", &sig).is_err());
}

#[test]
fn public_key_checks() {
    let kp = generate_rsa(2048).unwrap();
    assert!(check_public_pem(&kp.public_pem, 2048).is_ok());
    assert!(check_public_pem(&kp.public_pem, 3072).is_err());
    assert!(check_public_pem("not a key", 2048).is_err());
}

#[test]
fn hmac_roundtrip() {
    let s = hmac_sha256_hex(b"k", b"payload");
    assert!(hmac_sha256_verify_hex(b"k", b"payload", &s));
    assert!(!hmac_sha256_verify_hex(b"k2", b"payload", &s));
    assert!(!hmac_sha256_verify_hex(b"k", b"payload", "zz"));
}
