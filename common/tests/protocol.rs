//! Tests for `protocol`.
use chrono::{DateTime, Duration, Utc};
use croustypatch_common::protocol::*;
use uuid::Uuid;

fn payload(agent: Uuid, now: DateTime<Utc>) -> CommandPayload {
    CommandPayload {
        task_id: Uuid::new_v4(),
        agent_id: agent,
        issued_at: now,
        expires_at: now + Duration::minutes(5),
        nonce: "n".into(),
        action: CommandAction::Scan,
    }
}

#[test]
fn command_verification() {
    let kp = croustypatch_common::crypto::generate_rsa(2048).unwrap();
    let me = Uuid::new_v4();
    let now = Utc::now();
    let cmd = sign_command(&kp.private_pem, &payload(me, now)).unwrap();
    assert!(verify_command(&kp.public_pem, &cmd, me, now).is_ok());
    assert!(matches!(verify_command(&kp.public_pem, &cmd, Uuid::new_v4(), now), Err(CommandError::WrongAgent)));
    assert!(matches!(verify_command(&kp.public_pem, &cmd, me, now + Duration::minutes(10)), Err(CommandError::Expired)));
    let mut tampered = cmd.clone();
    tampered.payload = tampered.payload.replace("scan", "quarantine");
    assert!(matches!(verify_command(&kp.public_pem, &tampered, me, now), Err(CommandError::BadSignature)));
}
