//! Tests for `crypto`.
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use croustypatch_server::crypto::*;

#[test]
fn aes_roundtrip_and_tamper() {
    let k = MasterKey::from_bytes([7; 32]);
    let ct = k.encrypt(b"smtp-secret");
    assert_eq!(&k.decrypt(&ct).unwrap()[..], b"smtp-secret");
    assert_ne!(ct, k.encrypt(b"smtp-secret"), "fresh nonce each time");
    let mut raw = B64.decode(&ct).unwrap();
    *raw.last_mut().unwrap() ^= 1;
    assert!(k.decrypt(&B64.encode(raw)).is_err());
    assert!(MasterKey::from_bytes([8; 32]).decrypt(&ct).is_err());
}

#[test]
fn argon2() {
    let h = hash_password("correct horse");
    assert!(h.starts_with("$argon2id$"));
    assert!(verify_password("correct horse", &h));
    assert!(!verify_password("wrong", &h));
    assert!(!verify_password("x", "garbage"));
}

#[test]
fn master_key_file_permissions() {
    let d = tempdir();
    let p = d.join("master.key");
    let a = MasterKey::load_or_create(None, &p).unwrap();
    let b = MasterKey::load_or_create(None, &p).unwrap();
    assert_eq!(&a.derive("x")[..], &b.derive("x")[..]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(MasterKey::load_or_create(None, &p).is_err());
    }
}

fn tempdir() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("cp-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
