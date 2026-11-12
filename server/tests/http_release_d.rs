//! Integration (0.8): planned compliance audits and audit reporting.
mod support;

use actix_web::test::TestRequest;
use chrono::{DateTime, Duration, Utc};
use croustypatch_server::compliance::{planned_due, run_planned, PLANNED_ACTION};
use croustypatch_server::schema::{audit_log, compliance_reports, hosts, settings, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use uuid::Uuid;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn a_planned_audit_is_due_once_per_moment_and_only_soon_after_it() {
    let daily = json!({"schedule": {"frequency": "daily", "at": "03:00", "timezone": "Europe/Paris"}});
    // 03:00 in Paris is 02:00 UTC in November
    let slot = at("2026-11-12T02:00:00Z");
    assert_eq!(planned_due(&daily, None, None, at("2026-11-12T02:00:30Z")), Some(slot));
    assert_eq!(planned_due(&daily, None, None, at("2026-11-12T01:59:00Z")), None, "yesterday's moment is long past");
    assert_eq!(planned_due(&daily, Some(slot), None, at("2026-11-12T02:20:00Z")), None, "already done");
    assert_eq!(planned_due(&daily, Some(slot - Duration::days(1)), None, at("2026-11-12T05:00:00Z")), Some(slot), "caught up a few hours later");
    assert_eq!(planned_due(&daily, None, None, at("2026-11-12T09:00:00Z")), None, "not 7 hours later");
    assert_eq!(planned_due(&daily, None, Some(at("2026-11-12T02:30:00Z")), at("2026-11-12T03:00:00Z")), None, "a schedule saved after the moment waits for the next one");
    // weekly on Thursday (weekday 3): 2026-11-12 is a Thursday
    let weekly = json!({"schedule": {"frequency": "weekly", "weekday": 3, "at": "22:30", "timezone": "UTC"}});
    assert_eq!(planned_due(&weekly, None, None, at("2026-11-12T22:31:00Z")), Some(at("2026-11-12T22:30:00Z")));
    assert_eq!(planned_due(&weekly, None, None, at("2026-11-13T22:31:00Z")), None);
    for off in [json!({}), json!({"schedule": {"frequency": "off", "at": "03:00"}}), json!({"schedule": {"frequency": "hourly"}})] {
        assert_eq!(planned_due(&off, None, None, at("2026-11-12T03:00:00Z")), None, "{off}");
    }
}

#[actix_web::test]
async fn the_fleet_is_audited_on_schedule_and_the_report_says_how_each_machine_scores() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (on, off, quarantined) = {
        let mut c = db.conn();
        let mk = |c: &mut PgConnection, n: &str, status: &str, oscap: bool| -> Uuid {
            diesel::insert_into(hosts::table).values((hosts::hostname.eq(n), hosts::public_key_pem.eq("pem"), hosts::status.eq(status), hosts::modules.eq(json!({"openscap": oscap})))).returning(hosts::id).get_result(c).unwrap()
        };
        (mk(&mut c, "srv-audited", "offline", true), mk(&mut c, "srv-no-oscap", "online", false), mk(&mut c, "srv-quarantined", "quarantined", true))
    };

    // the schedule is checked when saved
    let put = |v: Value| TestRequest::put().uri("/api/v1/settings/compliance").cookie(ck.clone()).set_json(v);
    for bad in [json!({"frequency": "hourly"}), json!({"frequency": "daily", "at": "25:00"}), json!({"frequency": "daily", "at": "3:00"}), json!({"frequency": "weekly", "weekday": 7}), json!({"frequency": "monthly", "day_of_month": 31}), json!({"frequency": "daily", "timezone": "Mars/Olympus"})] {
        let (st, b) = call(&app, put(json!({"threshold": 80, "profile": "", "schedule": bad.clone()}))).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    // daily, a minute or two ago (UTC)
    let hhmm = (Utc::now() - Duration::minutes(2)).format("%H:%M").to_string();
    let (st, b) = call(&app, put(json!({"threshold": 80, "profile": "", "schedule": {"frequency": "daily", "weekday": 0, "day_of_month": 1, "at": hhmm, "timezone": "UTC"}}))).await;
    assert!(st.is_success(), "{b}");
    assert_eq!(run_planned(&mut db.conn(), Utc::now()).unwrap(), 0, "saved after this moment: it waits for tomorrow's");

    // the same schedule, saved yesterday: today's audit runs, once
    diesel::update(settings::table.find("compliance")).set(settings::updated_at.eq(Utc::now() - Duration::days(1))).execute(&mut db.conn()).unwrap();
    assert_eq!(run_planned(&mut db.conn(), Utc::now()).unwrap(), 1, "only the host with OpenSCAP on and not quarantined");
    let queued: Vec<(Uuid, String, String)> = tasks::table.filter(tasks::kind.eq("compliance")).select((tasks::host_id, tasks::status, tasks::requested_by)).load(&mut db.conn()).unwrap();
    assert_eq!(queued, vec![(on, "queued".to_string(), "schedule".to_string())], "an offline host gets its audit when it comes back");
    let _ = (off, quarantined);
    let ev: Value = audit_log::table.filter(audit_log::action.eq(PLANNED_ACTION)).select(audit_log::details).first(&mut db.conn()).unwrap();
    assert_eq!((ev["hosts"].clone(), ev["hostnames"].clone()), (json!(1), json!(["srv-audited"])));
    assert_eq!(run_planned(&mut db.conn(), Utc::now()).unwrap(), 0, "once per moment");

    // the next planned audit is shown on the Compliance page
    let (_, fleet) = call(&app, TestRequest::get().uri("/api/v1/compliance").cookie(ck.clone())).await;
    let next: DateTime<Utc> = fleet["planned_next"].as_str().expect("planned_next").parse().unwrap();
    assert!(next > Utc::now() && next < Utc::now() + Duration::days(1), "{next}");

    // audit reporting: each machine's latest audit, worst first, below the threshold marked; the never-audited count
    {
        let mut c = db.conn();
        let rules = json!([{"rule_id": "xccdf_org.ssgproject.content_rule_a", "severity": "high"}, {"rule_id": "xccdf_org.ssgproject.content_rule_b", "severity": "medium"}]);
        diesel::insert_into(compliance_reports::table)
            .values((compliance_reports::host_id.eq(on), compliance_reports::profile.eq("xccdf_org.ssgproject.content_profile_cis_level1_server"), compliance_reports::datastream.eq("ssg-ubuntu2404-ds-resolute.xml"), compliance_reports::passed.eq(60), compliance_reports::failed.eq(40), compliance_reports::not_applicable.eq(3), compliance_reports::other.eq(0), compliance_reports::score.eq(60.0), compliance_reports::failed_rules.eq(rules), compliance_reports::results_sha256.eq("ab".repeat(32)), compliance_reports::collected_at.eq(Utc::now())))
            .execute(&mut c)
            .unwrap();
    }
    let (st, p) = call(&app, TestRequest::post().uri("/api/v1/reports/preview").cookie(ck.clone()).set_json(json!({"name": "R", "language": "en", "sections": {"summary": false, "pending": false, "history": false, "failures": false, "reboots": false, "patch_days": false, "compliance": true}}))).await;
    assert_eq!(st, 200, "{p}");
    let text = p["text"].as_str().unwrap();
    assert!(text.contains("Compliance"), "{text}");
    assert!(text.contains("Below the threshold (80 %)") && text.contains("Never audited"), "{text}");
    assert!(text.contains("srv-audited") && text.contains("cis_level1_server") && text.contains("60.0 % ▼"), "{text}");
    assert!(text.contains("Next planned audit"), "{text}");
    assert!(p["html"].as_str().unwrap().contains("srv-audited"));
}

// ------------------------------------------------------------------------------------------------- 0.9

#[actix_web::test]
async fn the_logs_page_merges_tasks_notifications_reports_and_the_servers_own_lines() {
    use croustypatch_server::schema::{notification_channels, notifications, task_events};
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let now = Utc::now();
    {
        let mut c = db.conn();
        let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("srv-logs"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut c).unwrap();
        let task: Uuid = diesel::insert_into(tasks::table).values((tasks::host_id.eq(host), tasks::kind.eq("patch"), tasks::status.eq("failed"), tasks::message.eq("x"))).returning(tasks::id).get_result(&mut c).unwrap();
        for (i, (st, msg)) in [("queued", "requested by admin"), ("running", "applying 3 packages"), ("failed", "dpkg was interrupted")].iter().enumerate() {
            diesel::insert_into(task_events::table).values((task_events::task_id.eq(task), task_events::ts.eq(now - Duration::minutes(10 - i as i64)), task_events::status.eq(*st), task_events::message.eq(*msg))).execute(&mut c).unwrap();
        }
        let ch: Uuid = diesel::insert_into(notification_channels::table).values((notification_channels::name.eq("SIEM"), notification_channels::kind.eq("webhook"), notification_channels::config.eq(json!({})))).returning(notification_channels::id).get_result(&mut c).unwrap();
        diesel::insert_into(notifications::table)
            .values((notifications::channel_id.eq(ch), notifications::event.eq("task.failed"), notifications::severity.eq("high"), notifications::title.eq("Patch failed"), notifications::body.eq(""), notifications::payload.eq(json!({})), notifications::recipients.eq(json!([])), notifications::status.eq("failed"), notifications::attempts.eq(5), notifications::last_error.eq("connection refused"), notifications::created_at.eq(now - Duration::minutes(5))))
            .execute(&mut c)
            .unwrap();
    }
    croustypatch_server::logs::push_server(now - Duration::minutes(1), "warn", "croustypatch_server::scheduler", "planner failed for host".into());

    let get = |q: &str| TestRequest::get().uri(&format!("/api/v1/logs?{q}")).cookie(ck.clone());
    let (st, all) = call(&app, get("limit=50")).await;
    assert_eq!(st, 200, "{all}");
    let rows = all.as_array().unwrap();
    let ts: Vec<&str> = rows.iter().map(|r| r["ts"].as_str().unwrap()).collect();
    assert!(ts.windows(2).all(|w| w[0] >= w[1]), "newest first: {ts:?}");
    let sources: std::collections::HashSet<&str> = rows.iter().map(|r| r["source"].as_str().unwrap()).collect();
    assert!(["tasks", "notifications", "server"].iter().all(|s| sources.contains(s)), "{sources:?}");
    let failed = rows.iter().find(|r| r["message"] == json!("dpkg was interrupted")).unwrap();
    assert_eq!((failed["level"].clone(), failed["hostname"].clone(), failed["title"].clone()), (json!("error"), json!("srv-logs"), json!("patch · failed")));
    let n = rows.iter().find(|r| r["source"] == json!("notifications")).unwrap();
    assert!(n["message"].as_str().unwrap().contains("connection refused") && n["title"] == json!("task.failed → SIEM"), "{n}");

    // filters: errors only, one source, words
    let (_, errs) = call(&app, get("level=error")).await;
    assert!(errs.as_array().unwrap().iter().all(|r| r["level"] == json!("error")) && errs.as_array().unwrap().len() >= 2, "{errs}");
    let (_, srv) = call(&app, get("source=server&level=warn&q=planner")).await;
    assert!(srv.as_array().unwrap().iter().any(|r| r["message"] == json!("planner failed for host")), "{srv}");
    let (_, words) = call(&app, get("q=srv-logs%20dpkg")).await;
    assert_eq!(words.as_array().unwrap().len(), 1, "{words}");
    // paging: strictly older than the last line shown
    let (_, page) = call(&app, get(&format!("source=tasks&before={}", urlencode(failed["ts"].as_str().unwrap())))).await;
    assert_eq!(page.as_array().unwrap().iter().map(|r| r["message"].as_str().unwrap()).collect::<Vec<_>>(), vec!["applying 3 packages", "requested by admin"]);
    let (st, _) = call(&app, get("source=nope")).await;
    assert_eq!(st, 400);
    // like the audit log: not for operators
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/logs")).await;
    assert_eq!(st, 401);
}

fn urlencode(s: &str) -> String {
    s.replace('+', "%2B").replace(':', "%3A")
}

#[actix_web::test]
async fn arch_agents_watch_the_aur_packages_of_the_catalog_and_the_app_center_shows_their_updates() {
    use croustypatch_common::protocol::{AgentMessage, AppUpdate, HostFacts, Inventory};
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = {
        let mut c = db.conn();
        diesel::insert_into(hosts::table)
            .values((hosts::hostname.eq("crshell"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::package_manager.eq("pacman"), hosts::modules.eq(json!({"portal": true})), hosts::facts.eq(json!({"capabilities": croustypatch_common::protocol::capability::ALL}))))
            .returning(hosts::id)
            .get_result(&mut c)
            .unwrap()
    };
    let recipe = json!({"pacman": {"command": {"install": "makepkg -si", "uninstall": "pacman -R visual-studio-code-bin", "check": "/usr/bin/code", "latest": {"aur": "visual-studio-code-bin"}}}});
    let (st, a) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(json!({"name": "Visual Studio Code", "recipes": recipe}))).await;
    assert_eq!(st, 201, "{a}");
    let s = croustypatch_server::modules::settings_for(&mut db.conn(), host).unwrap().unwrap();
    assert_eq!(s.watch_aur, vec!["visual-studio-code-bin".to_string()]);

    // installed through the app center, then the agent reports a newer version in the AUR
    {
        let mut c = db.conn();
        diesel::insert_into(tasks::table).values((tasks::host_id.eq(host), tasks::kind.eq("app"), tasks::status.eq("succeeded"), tasks::message.eq("ok"), tasks::finished_at.eq(Some(Utc::now())), tasks::exec.eq(Some(json!({"app_id": a["id"], "install": true}))))).execute(&mut c).unwrap();
    }
    let state = actix_web::web::Data::new(db.state());
    let inv = Inventory {
        facts: HostFacts { hostname: "crshell".into(), package_manager: Some(croustypatch_common::model::PackageManager::Pacman), capabilities: croustypatch_common::protocol::capability::ALL.iter().map(|s| s.to_string()).collect(), ..Default::default() },
        packages: vec![],
        reboot_required: false,
        collected_at: Utc::now(),
        app_updates: vec![AppUpdate { package: "visual-studio-code-bin".into(), installed: "1.145.2-1".into(), latest: "1.146.0-1".into() }],
    };
    croustypatch_server::gateway::handle_messages(&state, host, vec![AgentMessage::Inventory(inv)]).await;
    let (st, v) = call(&app, TestRequest::get().uri(&format!("/api/v1/portal/hosts/{host}")).cookie(ck.clone())).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["apps"][0]["update"], json!({"package": "visual-studio-code-bin", "installed": "1.145.2-1", "latest": "1.146.0-1"}));
    // updating is installing again
    let (st, r) = call(&app, TestRequest::post().uri("/api/v1/portal/install").cookie(ck.clone()).set_json(json!({"host_id": host, "app_id": a["id"], "action": "INSTALL"}))).await;
    assert_eq!(st, 202, "{r}");
    // and an inventory without updates clears it
    let inv = Inventory { facts: HostFacts { hostname: "crshell".into(), package_manager: Some(croustypatch_common::model::PackageManager::Pacman), ..Default::default() }, packages: vec![], reboot_required: false, collected_at: Utc::now(), app_updates: vec![] };
    croustypatch_server::gateway::handle_messages(&state, host, vec![AgentMessage::Inventory(inv)]).await;
    let (_, v) = call(&app, TestRequest::get().uri(&format!("/api/v1/portal/hosts/{host}")).cookie(ck.clone())).await;
    assert_eq!(v["apps"][0]["update"], Value::Null);
}

#[actix_web::test]
async fn agents_get_the_servers_scap_content_by_name_only() {
    use croustypatch_common::crypto;
    use croustypatch_common::jwt::{self, Claims};
    let db = support::TestDb::new();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ssg-ubuntu2404-ds.xml"), "<ds/>").unwrap();
    std::fs::write(dir.path().join("secret.txt"), "no").unwrap();
    std::fs::write(dir.path().join("index.json"), json!({"version": "0.1.82", "files": [{"name": "ssg-ubuntu2404-ds.xml", "sha256": crypto::sha256_hex(b"<ds/>"), "size": 5}]}).to_string()).unwrap();
    let scap = dir.path().to_path_buf();
    let state = db.state_with(|c| c.scap_dir = scap);
    let app = support::http::app_with(&db, state).await;
    let kp = crypto::generate_rsa(2048).unwrap();
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("srv-scap"), hosts::public_key_pem.eq(&kp.public_pem), hosts::status.eq("online"))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    let auth = |jti: &str| {
        let now = Utc::now().timestamp();
        format!("Bearer {}", jwt::sign_rs256(&kp.private_pem, &Claims { sub: host.to_string(), iss: "agent".into(), iat: now - 5, exp: now + 60, jti: jti.into(), scope: None }).unwrap())
    };
    let (st, idx) = call(&app, TestRequest::get().uri("/api/v1/agent/scap-content").insert_header(("authorization", auth("s1")))).await;
    assert_eq!(st, 200, "{idx}");
    assert_eq!(idx["version"], json!("0.1.82"));
    let r = actix_web::test::call_service(&app, TestRequest::get().uri("/api/v1/agent/scap-content/ssg-ubuntu2404-ds.xml").insert_header(("authorization", auth("s2"))).to_request()).await;
    assert_eq!(r.status(), 200);
    assert_eq!(actix_web::test::read_body(r).await, "<ds/>".as_bytes());
    for (i, bad) in ["secret.txt", "index.json", "..%2Fetc%2Fpasswd"].iter().enumerate() {
        let (st, _) = call(&app, TestRequest::get().uri(&format!("/api/v1/agent/scap-content/{bad}")).insert_header(("authorization", auth(&format!("b{i}"))))).await;
        assert_eq!(st, 404, "{bad}");
    }
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/agent/scap-content")).await;
    assert_eq!(st, 401, "agents only");
}
