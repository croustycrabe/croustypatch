//! Tests for `identity`.
use croustypatch_common::{crypto, jwt};
use uuid::Uuid;
use croustypatch_agent::identity::*;

#[test]
fn jwt_verifies_with_matching_public_key() {
    let kp = crypto::generate_rsa(2048).unwrap();
    let id = Identity { agent_id: Uuid::new_v4(), server_public_key_pem: String::new(), private_key_pem: kp.private_pem };
    let t = id.jwt().unwrap();
    let c = jwt::verify_claims_rs256(&kp.public_pem, &t, chrono::Utc::now().timestamp(), 0).unwrap();
    assert_eq!(c.sub, id.agent_id.to_string());
    assert_eq!(c.iss, "agent");
    assert_ne!(t, id.jwt().unwrap(), "every token has a fresh jti");
}
