//! Integration: audit chain, bootstrap account, and agent command dispatch against a real PostgreSQL.
mod support;

use croustypatch_common::protocol::{verify_command, CommandAction};
use croustypatch_server::audit::{self, Event};
use croustypatch_server::auth;
use croustypatch_server::gateway;
use croustypatch_server::schema::{hosts, tasks, users};
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

fn record(c: &mut PgConnection, actor: &str, action: &str) {
    audit::record(c, Event::new(actor, action).target("t", "1").details(json!({"k": action}))).unwrap();
}

#[test]
fn audit_log_is_append_only_and_tampering_is_detected() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    for i in 0..5 {
        record(&mut c, "admin", &format!("test.{i}"));
    }
    let v = audit::verify(&mut c).unwrap();
    assert!(v.ok && v.entries == 5, "{v:?}");

    // normal access cannot rewrite history
    for sql in ["UPDATE audit_log SET actor = 'mallory' WHERE seq = 2", "DELETE FROM audit_log WHERE seq = 2", "TRUNCATE audit_log"] {
        let err = diesel::sql_query(sql).execute(&mut c).unwrap_err().to_string();
        assert!(err.contains("append-only"), "{sql}: {err}");
    }
    assert!(audit::verify(&mut c).unwrap().ok);

    // a superuser who disables the trigger is still caught by the hash chain
    diesel::sql_query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_update").execute(&mut c).unwrap();
    diesel::sql_query("UPDATE audit_log SET actor = 'mallory' WHERE seq = 3").execute(&mut c).unwrap();
    let v = audit::verify(&mut c).unwrap();
    assert!(!v.ok);
    assert_eq!(v.first_bad_seq, Some(3));
    // deleting a row in the middle breaks the chain too
    diesel::sql_query("UPDATE audit_log SET actor = 'admin' WHERE seq = 3").execute(&mut c).unwrap();
    assert!(audit::verify(&mut c).unwrap().ok, "restoring the original value heals it");
    diesel::sql_query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_delete").execute(&mut c).ok();
    diesel::sql_query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_truncate").execute(&mut c).ok();
}

#[test]
fn concurrent_appends_keep_one_valid_chain() {
    let db = support::TestDb::new();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let pool = db.pool.clone();
            std::thread::spawn(move || {
                let mut c = pool.get().unwrap();
                for j in 0..5 {
                    record(&mut c, &format!("t{i}"), &format!("n.{j}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let mut c = db.conn();
    let v = audit::verify(&mut c).unwrap();
    assert!(v.ok && v.entries == 40, "advisory lock must serialise the chain: {v:?}");
}

#[test]
fn bootstrap_creates_one_fallback_admin_and_refuses_to_lock_everyone_out() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    auth::ensure_roles(&mut c).unwrap();
    auth::ensure_roles(&mut c).unwrap(); // idempotent

    let b = auth::ensure_fallback_admin(&mut c, None).unwrap().expect("first start creates it");
    assert!(b.password.as_deref().is_some_and(|p| p.len() >= 12), "generated password is strong");
    assert!(auth::ensure_fallback_admin(&mut c, None).unwrap().is_none(), "only on an empty install");

    // a preset password is used but never handed back for display
    let db2 = support::TestDb::new();
    let mut c2 = db2.conn();
    auth::ensure_roles(&mut c2).unwrap();
    assert!(auth::ensure_fallback_admin(&mut c2, Some("preset-password-123".into())).unwrap().unwrap().password.is_none());

    // disabling the fallback with no other admin is refused
    assert!(auth::set_fallback_disabled(&mut c, true).is_err());
    let role: Uuid = croustypatch_server::schema::roles::table.filter(croustypatch_server::schema::roles::name.eq("super_admin")).select(croustypatch_server::schema::roles::id).first(&mut c).unwrap();
    diesel::insert_into(users::table).values((users::username.eq("real-admin"), users::role_id.eq(role), users::password_hash.eq("x"))).execute(&mut c).unwrap();
    assert_eq!(auth::set_fallback_disabled(&mut c, true).unwrap(), 1);
    let disabled: bool = users::table.filter(users::username.eq("admin")).select(users::disabled).first(&mut c).unwrap();
    assert!(disabled);
    auth::set_fallback_disabled(&mut c, false).unwrap();
}

#[test]
fn password_reset_requires_a_strong_local_account() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    auth::ensure_roles(&mut c).unwrap();
    auth::ensure_fallback_admin(&mut c, Some("preset-password-123".into())).unwrap();
    assert!(auth::reset_password(&mut c, "admin", "short").is_err());
    assert!(auth::reset_password(&mut c, "ghost", "long-enough-password").is_err());
    auth::reset_password(&mut c, "admin", "a-new-long-password").unwrap();
    let (must_change, hash): (bool, Option<String>) = users::table.filter(users::username.eq("admin")).select((users::must_change_password, users::password_hash)).first(&mut c).unwrap();
    assert!(must_change);
    assert!(croustypatch_server::crypto::verify_password("a-new-long-password", &hash.unwrap()));
}

#[test]
fn queued_tasks_are_dispatched_once_as_signed_commands_bound_to_the_agent() {
    let db = support::TestDb::new();
    let state = db.state();
    let mut c = db.conn();
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("h"), hosts::public_key_pem.eq("pem"))).returning(hosts::id).get_result(&mut c).unwrap();
    let task: Uuid = diesel::insert_into(tasks::table)
        .values((tasks::host_id.eq(host), tasks::kind.eq("patch"), tasks::status.eq("queued"), tasks::packages.eq(json!([{"name": "libssl3", "from": "1", "to": "2"}]))))
        .returning(tasks::id)
        .get_result(&mut c)
        .unwrap();
    // a task scheduled in the future (maintenance window) must not be sent yet
    diesel::insert_into(tasks::table)
        .values((tasks::host_id.eq(host), tasks::kind.eq("scan"), tasks::status.eq("queued"), tasks::scheduled_for.eq(chrono::Utc::now() + chrono::Duration::hours(2))))
        .execute(&mut c)
        .unwrap();

    // `scheduled_for` defaults to the database's now(): a test database on another machine may run slightly ahead.
    std::thread::sleep(std::time::Duration::from_secs(2));
    let cmds = gateway::claim_due_commands(&mut c, &state, host).unwrap();
    assert_eq!(cmds.len(), 1);
    let payload = verify_command(&state.server_public_pem, &cmds[0], host, chrono::Utc::now()).expect("the agent can verify it");
    assert_eq!(payload.task_id, task);
    match payload.action {
        CommandAction::ApplyUpdates { packages, .. } => assert_eq!(packages, ["libssl3"]),
        other => panic!("unexpected action {other:?}"),
    }
    assert!(verify_command(&state.server_public_pem, &cmds[0], Uuid::new_v4(), chrono::Utc::now()).is_err(), "another agent must refuse it");
    let status: String = tasks::table.find(task).select(tasks::status).first(&mut c).unwrap();
    assert_eq!(status, "sent");
    assert!(gateway::claim_due_commands(&mut c, &state, host).unwrap().is_empty(), "claimed exactly once");
}
