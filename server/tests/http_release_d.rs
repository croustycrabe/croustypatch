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
