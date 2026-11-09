//! Tests for `jwt`.
use croustypatch_common::crypto;
use croustypatch_common::jwt::*;

fn claims(exp: i64) -> Claims {
    Claims { sub: "a".into(), iss: "t".into(), iat: 1000, exp, jti: "j".into(), scope: None }
}

#[test]
fn hs256_ok_expired_and_tamper() {
    let t = sign_hs256(b"secret", &claims(2000)).unwrap();
    assert!(verify_claims_hs256(b"secret", &t, 1500, 0).is_ok());
    assert!(matches!(verify_claims_hs256(b"secret", &t, 3000, 0), Err(JwtError::Expired)));
    assert!(matches!(verify_claims_hs256(b"other", &t, 1500, 0), Err(JwtError::Signature)));
    assert_eq!(peek_subject(&t).as_deref(), Some("a"));
}

#[test]
fn rs256_ok_and_algorithm_confusion_rejected() {
    let kp = crypto::generate_rsa(2048).unwrap();
    let t = sign_rs256(&kp.private_pem, &claims(2000)).unwrap();
    assert!(verify_claims_rs256(&kp.public_pem, &t, 1500, 0).is_ok());
    // An RS256 token must not verify as HS256 (and vice versa).
    assert!(matches!(verify_hs256::<Claims>(kp.public_pem.as_bytes(), &t), Err(JwtError::Algorithm)));
    let h = sign_hs256(b"k", &claims(2000)).unwrap();
    assert!(matches!(verify_rs256::<Claims>(&kp.public_pem, &h), Err(JwtError::Algorithm)));
    assert!(matches!(verify_hs256::<Claims>(b"k", "a.b"), Err(JwtError::Malformed)));
}
