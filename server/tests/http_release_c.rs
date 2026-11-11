//! Integration (0.7): Docker menu, updates from the app center, OpenSCAP audit profiles, app icons.
mod support;

use actix_web::cookie::Cookie;
use actix_web::test::TestRequest;
use croustypatch_server::api::portal::create_update_request;
use croustypatch_server::schema::{audit_log, compliance_reports, host_packages, hosts, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call, login, session};
use uuid::Uuid;

fn add_host(c: &mut PgConnection, name: &str, modules: Value, facts: Value) -> Uuid {
    diesel::insert_into(hosts::table)
        .values((hosts::hostname.eq(name), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::package_manager.eq("apt"), hosts::modules.eq(modules), hosts::facts.eq(facts)))
        .returning(hosts::id)
        .get_result(c)
        .unwrap()
}

fn add_pkg(c: &mut PgConnection, host: Uuid, name: &str, source: &str, version: &str, candidate: Option<&str>, meta: Value) {
    diesel::insert_into(host_packages::table)
        .values((host_packages::host_id.eq(host), host_packages::name.eq(name), host_packages::source.eq(source), host_packages::version.eq(version), host_packages::candidate.eq(candidate), host_packages::meta.eq(meta)))
        .execute(c)
        .unwrap();
}

/// A user with a built-in role, logged in (a new account has to change its temporary password first).
async fn user_with_role<S>(app: &S, ck: &Cookie<'static>, name: &str, role: &str) -> Cookie<'static>
where
    S: actix_web::dev::Service<actix_http::Request, Response = actix_web::dev::ServiceResponse, Error = actix_web::Error>,
{
    let (_, roles) = call(app, TestRequest::get().uri("/api/v1/roles").cookie(ck.clone())).await;
    let id = roles.as_array().unwrap().iter().find(|r| r["name"] == role).unwrap()["id"].clone();
    let (st, b) = call(app, TestRequest::post().uri("/api/v1/users").cookie(ck.clone()).set_json(json!({"username": name, "password": "temporary-pass-12", "role_id": id}))).await;
    assert_eq!(st, 201, "{b}");
    let first = session(&login(app, name, "temporary-pass-12").await);
    call(app, TestRequest::post().uri("/api/v1/auth/password").cookie(first).set_json(json!({"current_password": "temporary-pass-12", "new_password": "a-new-password-123"}))).await;
    session(&login(app, name, "a-new-password-123").await)
}

fn finish_tasks(db: &support::TestDb, kind: &str) {
    diesel::update(tasks::table.filter(tasks::kind.eq(kind))).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
}

// ------------------------------------------------------------------------------------------ Docker menu

#[actix_web::test]
async fn the_docker_menu_lists_containers_and_updates_only_images() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (a, b) = {
        let mut c = db.conn();
        let a = add_host(&mut c, "docker-a", json!({"docker": true}), json!({}));
        let b = add_host(&mut c, "docker-b", json!({"docker": true}), json!({}));
        add_pkg(&mut c, a, "docker:web", "docker", "nginx:1.27 @aaaa", Some("nginx:1.27 @bbbb"), json!({"image": "nginx:1.27", "project": "site", "service": "web", "state": "running", "health": "healthy", "started_at": "2026-11-01T10:00:00Z"}));
        add_pkg(&mut c, a, "docker:db", "docker", "postgres:17 @cccc", None, json!({"image": "postgres:17", "state": "running"}));
        add_pkg(&mut c, b, "docker:cache", "docker", "redis:7 @dddd", Some("redis:7 @eeee"), json!({"image": "redis:7", "state": "exited"}));
        add_pkg(&mut c, b, "openssl", "system", "3.0", Some("3.1"), json!({}));
        (a, b)
    };
    let get = |q: &str| TestRequest::get().uri(&format!("/api/v1/docker{q}")).cookie(ck.clone());
    let (st, all) = call(&app, get("")).await;
    assert_eq!(st, 200, "{all}");
    assert_eq!(all["summary"], json!({"containers": 3, "with_updates": 2, "hosts": 2}), "system packages are not containers");
    let web = all["containers"].as_array().unwrap().iter().find(|c| c["name"] == "web").unwrap();
    assert_eq!((web["image"].clone(), web["current"].clone(), web["candidate"].clone(), web["update"].clone()), (json!("nginx:1.27"), json!("aaaa"), json!("bbbb"), json!(true)));
    assert_eq!((web["project"].clone(), web["service"].clone(), web["state"].clone(), web["health"].clone()), (json!("site"), json!("web"), json!("running"), json!("healthy")));
    let (_, only) = call(&app, get("?updates=true")).await;
    assert_eq!(only["summary"]["containers"], json!(2));
    let (_, one) = call(&app, get(&format!("?host={b}"))).await;
    assert_eq!(one["containers"].as_array().unwrap().len(), 1);
    let (_, found) = call(&app, get("?q=pos")).await;
    assert_eq!(found["containers"][0]["name"], json!("db"));
    let (_, none) = call(&app, get("?q=100%25")).await;
    assert_eq!(none["containers"].as_array().unwrap().len(), 0, "a % in the search is not a wildcard");

    // updating: containers and system packages make two tasks, a container never waits for the system patch
    let (st, r) = call(&app, TestRequest::post().uri("/api/v1/tasks").cookie(ck.clone()).set_json(json!({"host_ids": [b], "packages": []}))).await;
    assert!(st.is_success(), "{r}");
    let mut kinds: Vec<(String, Value)> = tasks::table.filter(tasks::host_id.eq(b)).select((tasks::kind, tasks::packages)).load(&mut db.conn()).unwrap();
    kinds.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(kinds.iter().map(|k| k.0.as_str()).collect::<Vec<_>>(), vec!["docker", "patch"]);
    assert_eq!(kinds[0].1[0]["name"], json!("docker:cache"));
    assert_eq!(kinds[1].1[0]["name"], json!("openssl"));
    let (_, shown) = call(&app, get(&format!("?host={b}"))).await;
    assert_eq!(shown["containers"][0]["open_task"]["status"], json!("queued"), "the page shows the update that is already queued");
    // and only that kind is blocked while one is open
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/tasks").cookie(ck.clone()).set_json(json!({"host_ids": [b], "packages": ["docker:cache"]}))).await;
    assert!(st == 409 || st.is_success(), "a second request for the same containers is refused or reports nothing new");
    let none_open: i64 = tasks::table.filter(tasks::host_id.eq(a)).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(none_open, 0, "other hosts are untouched");

    // read access is enough to look, not to update
    let auditor = user_with_role(&app, &ck, "audit1", "auditor").await;
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/docker").cookie(auditor.clone())).await;
    assert_eq!(st, 200);
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/tasks").cookie(auditor).set_json(json!({"host_ids": [a], "packages": []}))).await;
    assert_eq!(st, 403);
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/docker")).await;
    assert_eq!(st, 401);
}

// ------------------------------------------------------------------------------- updates from the app center

#[actix_web::test]
async fn people_start_their_own_updates_from_the_app_center_without_an_approval_step() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let laptop = add_host(&mut c, "laptop-alice", json!({"portal": true, "portal_updates": true}), json!({}));
    let locked = add_host(&mut c, "laptop-bob", json!({"portal": true, "portal_updates": false}), json!({}));
    let no_portal = add_host(&mut c, "ws-carol", json!({"portal": false, "portal_updates": true}), json!({}));
    for h in [laptop, locked, no_portal] {
        add_pkg(&mut c, h, "openssl", "system", "3.0", Some("3.1"), json!({}));
        add_pkg(&mut c, h, "vim", "system", "9.0", Some("9.1"), json!({}));
        add_pkg(&mut c, h, "docker:web", "docker", "nginx @a", Some("nginx @b"), json!({}));
        add_pkg(&mut c, h, "git", "system", "2.43", None, json!({}));
    }

    let v = croustypatch_server::api::portal::updates_view(&mut c, laptop).unwrap();
    assert_eq!(v["allowed"], json!(true), "{v}");
    let names: Vec<&str> = v["pending"].as_array().unwrap().iter().map(|u| u["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["openssl", "vim"], "system updates only, and only what has a newer version");
    assert_eq!(croustypatch_server::api::portal::updates_view(&mut c, locked).unwrap()["allowed"], json!(false), "the administrator keeps servers and chosen machines out");
    assert!(croustypatch_server::api::portal::updates_view(&mut c, no_portal).is_err(), "the app center must be on");

    // the person's request: a queued task, no approval, named after them; everything pending when no name is given
    let id = create_update_request(&mut c, laptop, &["vim".into()], "local:alice@laptop-alice", "127.0.0.1", "alice").unwrap();
    let (kind, status, by, pk): (String, String, String, Value) = tasks::table.find(id).select((tasks::kind, tasks::status, tasks::requested_by, tasks::packages)).first(&mut c).unwrap();
    assert_eq!((kind.as_str(), status.as_str(), by.as_str()), ("patch", "queued", "local:alice@laptop-alice"), "no approval step");
    assert_eq!(pk.as_array().unwrap().len(), 1);
    assert_eq!(pk[0]["name"], json!("vim"));
    let audited: i64 = audit_log::table.filter(audit_log::action.eq("portal.update_requested")).count().get_result(&mut c).unwrap();
    assert_eq!(audited, 1, "who asked is recorded");
    // one at a time
    assert!(create_update_request(&mut c, laptop, &[], "local:alice@laptop-alice", "127.0.0.1", "alice").is_err(), "an update is already waiting on this machine");
    diesel::update(tasks::table.find(id)).set(tasks::status.eq("succeeded")).execute(&mut c).unwrap();
    // names must be pending system updates of this machine: not a container, not an up to date package, not another command
    for bad in ["docker:web", "git", "nonexistent", "-rf", "vim;reboot"] {
        assert!(create_update_request(&mut c, laptop, &[bad.into()], "local:alice@laptop-alice", "127.0.0.1", "alice").is_err(), "{bad}");
    }
    let all = create_update_request(&mut c, laptop, &[], "local:alice@laptop-alice", "127.0.0.1", "alice").unwrap();
    let pk: Value = tasks::table.find(all).select(tasks::packages).first(&mut c).unwrap();
    assert_eq!(pk.as_array().unwrap().len(), 2, "no names: every pending system update, never the containers");
    // not allowed on this machine / app center off
    assert!(create_update_request(&mut c, locked, &[], "local:bob@laptop-bob", "127.0.0.1", "bob").is_err());
    assert!(create_update_request(&mut c, no_portal, &[], "local:carol@ws-carol", "127.0.0.1", "carol").is_err());
    let n: i64 = tasks::table.filter(tasks::host_id.eq_any([locked, no_portal])).count().get_result(&mut c).unwrap();
    assert_eq!(n, 0);
}

// ---------------------------------------------------------------------------------- OpenSCAP audit profiles

const STD: &str = "xccdf_org.ssgproject.content_profile_standard";
const CIS: &str = "xccdf_org.ssgproject.content_profile_cis";
const RULE_A: &str = "xccdf_org.ssgproject.content_rule_package_aide_installed";
const RULE_B: &str = "xccdf_org.ssgproject.content_rule_sysctl_kernel_randomize_va_space";

#[actix_web::test]
async fn audit_profiles_are_custom_per_host_and_sent_to_the_agent_as_a_tailoring() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let facts = json!({"openscap": {"version": "1.4.2", "datastream": "/usr/share/xml/scap/ssg/content/ssg-debian12-ds.xml", "profiles": [{"id": STD, "title": "Standard System Security Profile"}, {"id": CIS, "title": "CIS Benchmark"}]}});
    let (host, bare) = {
        let mut c = db.conn();
        (add_host(&mut c, "srv-1", json!({"openscap": true}), facts), add_host(&mut c, "srv-2", json!({"openscap": true}), json!({})))
    };
    let post = |b: Value| TestRequest::post().uri("/api/v1/compliance/profiles").cookie(ck.clone()).set_json(b);

    for bad in [
        json!({"name": "", "base_profile": CIS}),
        json!({"name": "x", "base_profile": "profile_cis"}),
        json!({"name": "x", "base_profile": "xccdf_a b"}),
        json!({"name": "x", "base_profile": CIS, "disabled_rules": ["not_an_xccdf_id"]}),
        json!({"name": "x", "base_profile": CIS, "disabled_rules": ["xccdf_x;rm -rf /"]}),
        json!({"name": "x".repeat(81), "base_profile": CIS}),
    ] {
        let (st, b) = call(&app, post(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let (st, p) = call(&app, post(json!({"name": "Serveurs", "description": "sans AIDE", "base_profile": CIS, "disabled_rules": [RULE_B, RULE_A, RULE_A]}))).await;
    assert_eq!(st, 201, "{p}");
    assert_eq!(p["disabled_rules"], json!([RULE_A, RULE_B]), "sorted, without duplicates");
    assert_eq!(p["used_by"], json!(0));
    let pid = p["id"].as_str().unwrap().to_string();
    let (st, _) = call(&app, post(json!({"name": "SERVEURS", "base_profile": CIS}))).await;
    assert_eq!(st, 409, "names are unique, whatever the case");

    // read-only users look, administrators change
    let auditor = user_with_role(&app, &ck, "audit1", "auditor").await;
    let (st, list) = call(&app, TestRequest::get().uri("/api/v1/compliance/profiles").cookie(auditor.clone())).await;
    assert_eq!(st, 200, "{list}");
    assert_eq!(list["custom"][0]["name"], json!("Serveurs"));
    assert_eq!(list["hosts_with_openscap"], json!(1), "only the host whose agent found OpenSCAP");
    let avail: Vec<(&str, i64)> = list["available"].as_array().unwrap().iter().map(|a| (a["id"].as_str().unwrap(), a["hosts"].as_i64().unwrap())).collect();
    assert_eq!(avail, vec![(CIS, 1), (STD, 1)]);
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/compliance/profiles").cookie(auditor.clone()).set_json(json!({"name": "y", "base_profile": CIS}))).await;
    assert_eq!(st, 403);
    let (st, _) = call(&app, TestRequest::delete().uri(&format!("/api/v1/compliance/profiles/{pid}")).cookie(auditor)).await;
    assert_eq!(st, 403);

    // a host's profile: '' (default), a datastream profile, or a custom one that exists
    let patch = |h: Uuid, v: &str| TestRequest::patch().uri(&format!("/api/v1/hosts/{h}")).cookie(ck.clone()).set_json(json!({"compliance_profile": v}));
    for bad in ["custom:00000000-0000-0000-0000-000000000000", "custom:nope", "has space", "x;y"] {
        let (st, b) = call(&app, patch(host, bad)).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let (st, h) = call(&app, patch(host, &format!("custom:{pid}"))).await;
    assert_eq!(st, 200, "{h}");
    assert_eq!(h["compliance_profile"], json!(format!("custom:{pid}")));
    let (_, comp) = call(&app, TestRequest::get().uri(&format!("/api/v1/hosts/{host}/compliance")).cookie(ck.clone())).await;
    assert_eq!((comp["profile"].clone(), comp["effective_profile"].clone(), comp["module"].clone(), comp["auto_install"].clone()), (json!(format!("custom:{pid}")), json!(format!("custom:{pid}")), json!(true), json!(true)));
    assert_eq!(comp["openscap"]["version"], json!("1.4.2"));
    let (_, nofacts) = call(&app, TestRequest::get().uri(&format!("/api/v1/hosts/{bare}/compliance")).cookie(ck.clone())).await;
    assert_eq!(nofacts["openscap"], Value::Null, "not installed there");

    // the audit goes out with the tailoring (and the permission to install what is missing)
    let (st, r) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone())).await;
    assert_eq!(st, 202, "{r}");
    let exec: Value = tasks::table.filter(tasks::host_id.eq(host)).filter(tasks::kind.eq("compliance")).select(tasks::exec).first::<Option<Value>>(&mut db.conn()).unwrap().unwrap();
    assert_eq!(exec["profile"], json!(CIS), "the agent evaluates the base profile of the tailoring");
    assert_eq!(exec["auto_install"], json!(true));
    assert_eq!((exec["tailoring"]["name"].clone(), exec["tailoring"]["base"].clone(), exec["tailoring"]["disabled_rules"].clone()), (json!("Serveurs"), json!(CIS), json!([RULE_A, RULE_B])));
    let state = db.state();
    actix_web::rt::time::sleep(std::time::Duration::from_secs(2)).await; // a remote test database may run a moment ahead
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, host).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["tailoring"]["id"], json!(pid), "{payload}");
    assert_eq!(payload["action"]["tailoring"]["disabled_rules"], json!([RULE_A, RULE_B]));
    assert_eq!(payload["action"]["auto_install"], json!(true));
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone())).await;
    assert_eq!(st, 409, "one audit at a time");
    finish_tasks(&db, "compliance");

    // a profile in use is not deleted; once nobody uses it, it can be
    let (_, list) = call(&app, TestRequest::get().uri("/api/v1/compliance/profiles").cookie(ck.clone())).await;
    assert_eq!(list["custom"][0]["used_by"], json!(1));
    let del = || TestRequest::delete().uri(&format!("/api/v1/compliance/profiles/{pid}")).cookie(ck.clone());
    let (st, b) = call(&app, del()).await;
    assert_eq!(st, 409, "{b}");
    // editing keeps the audit trail and the same id
    let (st, upd) = call(&app, TestRequest::put().uri(&format!("/api/v1/compliance/profiles/{pid}")).cookie(ck.clone()).set_json(json!({"name": "Serveurs prod", "base_profile": STD, "disabled_rules": [RULE_A]}))).await;
    assert_eq!(st, 200, "{upd}");
    assert_eq!((upd["id"].clone(), upd["base_profile"].clone(), upd["used_by"].clone()), (json!(pid), json!(STD), json!(1)));

    // an explicit profile in the request wins over the host's own; a plain one carries no tailoring
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone()).set_json(json!({"profile": STD}))).await;
    assert_eq!(st, 202);
    let exec: Value = tasks::table.filter(tasks::host_id.eq(host)).filter(tasks::kind.eq("compliance")).filter(tasks::status.eq("queued")).select(tasks::exec).first::<Option<Value>>(&mut db.conn()).unwrap().unwrap();
    assert_eq!((exec["profile"].clone(), exec["tailoring"].clone()), (json!(STD), Value::Null));
    finish_tasks(&db, "compliance");
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{host}/compliance-scan")).cookie(ck.clone()).set_json(json!({"profile": "bad id"}))).await;
    assert_eq!(st, 400);

    // the fleet default: a custom profile or an XCCDF id; auto-install can be switched off
    let put = |v: Value| TestRequest::put().uri("/api/v1/settings/compliance").cookie(ck.clone()).set_json(v);
    let (st, _) = call(&app, put(json!({"threshold": 80, "profile": "custom:not-a-uuid"}))).await;
    assert_eq!(st, 400);
    let (st, _) = call(&app, put(json!({"threshold": 80, "profile": "", "auto_install": "yes"}))).await;
    assert_eq!(st, 400);
    let (st, b) = call(&app, put(json!({"threshold": 80, "profile": format!("custom:{pid}"), "auto_install": false}))).await;
    assert!(st.is_success(), "{b}");
    call(&app, patch(host, "")).await;
    let (_, list) = call(&app, TestRequest::get().uri("/api/v1/compliance/profiles").cookie(ck.clone())).await;
    assert_eq!(list["custom"][0]["used_by"], json!(2), "every host without a profile of its own follows the fleet default");
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{bare}/compliance-scan")).cookie(ck.clone())).await;
    assert_eq!(st, 202);
    let exec: Value = tasks::table.filter(tasks::host_id.eq(bare)).select(tasks::exec).first::<Option<Value>>(&mut db.conn()).unwrap().unwrap();
    assert_eq!((exec["auto_install"].clone(), exec["tailoring"]["name"].clone(), exec["profile"].clone()), (json!(false), json!("Serveurs prod"), json!(STD)));
    actix_web::rt::time::sleep(std::time::Duration::from_secs(2)).await;
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, bare).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["auto_install"], json!(false), "the agent installs nothing when the administrator said no");
    finish_tasks(&db, "compliance");

    // OpenSCAP off for a host: refused
    {
        let mut c = db.conn();
        diesel::update(hosts::table.find(bare)).set(hosts::modules.eq(json!({"openscap": false}))).execute(&mut c).unwrap();
    }
    let (st, _) = call(&app, TestRequest::post().uri(&format!("/api/v1/hosts/{bare}/compliance-scan")).cookie(ck.clone())).await;
    assert_eq!(st, 409);

    // free the profile, then delete it
    call(&app, put(json!({"threshold": 80, "profile": "", "auto_install": true}))).await;
    let (st, b) = call(&app, del()).await;
    assert_eq!(st, 204, "{b:?}");
    let (st, _) = call(&app, del()).await;
    assert_eq!(st, 404);
    let actions: Vec<String> = audit_log::table.filter(audit_log::action.like("compliance.profile_%")).order(audit_log::seq.asc()).select(audit_log::action).load(&mut db.conn()).unwrap();
    assert_eq!(actions, vec!["compliance.profile_created", "compliance.profile_updated", "compliance.profile_deleted"]);
}

#[actix_web::test]
async fn the_editor_suggests_the_rules_that_fail_on_the_fleet() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    {
        let mut c = db.conn();
        let (a, b) = (add_host(&mut c, "a", json!({"openscap": true}), json!({})), add_host(&mut c, "b", json!({"openscap": true}), json!({})));
        for (h, rules) in [
            (a, json!([{"rule_id": RULE_A, "severity": "low"}, {"rule_id": RULE_B, "severity": "high"}])),
            (b, json!([{"rule_id": RULE_A, "severity": "low"}])),
        ] {
            diesel::insert_into(compliance_reports::table)
                .values((compliance_reports::host_id.eq(h), compliance_reports::profile.eq(STD), compliance_reports::passed.eq(5), compliance_reports::failed.eq(2), compliance_reports::not_applicable.eq(0), compliance_reports::other.eq(0), compliance_reports::datastream.eq("ds"), compliance_reports::results_sha256.eq("x"), compliance_reports::failed_rules.eq(rules), compliance_reports::score.eq(50.0), compliance_reports::collected_at.eq(chrono::Utc::now())))
                .execute(&mut c)
                .unwrap();
        }
        // an older report of host a is not what fails today
        diesel::insert_into(compliance_reports::table)
            .values((compliance_reports::host_id.eq(a), compliance_reports::profile.eq(STD), compliance_reports::passed.eq(1), compliance_reports::failed.eq(9), compliance_reports::not_applicable.eq(0), compliance_reports::other.eq(0), compliance_reports::datastream.eq("ds"), compliance_reports::results_sha256.eq("x"), compliance_reports::failed_rules.eq(json!([{"rule_id": "xccdf_org.ssgproject.content_rule_old", "severity": "high"}])), compliance_reports::score.eq(10.0), compliance_reports::collected_at.eq(chrono::Utc::now() - chrono::Duration::days(30)), compliance_reports::created_at.eq(chrono::Utc::now() - chrono::Duration::days(30))))
            .execute(&mut c)
            .unwrap();
    }
    let (st, r) = call(&app, TestRequest::get().uri("/api/v1/compliance/rules").cookie(ck.clone())).await;
    assert_eq!(st, 200, "{r}");
    let rules: Vec<(&str, &str, i64)> = r["rules"].as_array().unwrap().iter().map(|x| (x["rule_id"].as_str().unwrap(), x["severity"].as_str().unwrap(), x["hosts"].as_i64().unwrap())).collect();
    assert_eq!(rules, vec![(RULE_B, "high", 1), (RULE_A, "low", 2)], "worst first, with how many hosts fail them; only the latest audit of each host counts");
}

// ------------------------------------------------------------------------------------------- app icons

#[actix_web::test]
async fn app_icons_are_generic_logos_or_small_uploaded_images() {
    use base64::Engine;
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let post = |b: Value| TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(b);
    let png = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode([&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..], &[0u8; 16]].concat()));
    let evil_svg = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode("<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>"));
    for bad in ["brand:not-a-logo", "https://example.org/logo.png", "javascript:alert(1)", "<img src=x onerror=alert(1)>", evil_svg.as_str(), "data:image/png;base64,AAAA", "data:text/html;base64,PGI+"] {
        let (st, b) = call(&app, post(json!({"name": "x", "packages": ["vim"], "icon": bad}))).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let (st, git) = call(&app, post(json!({"name": "Git", "packages": ["git"], "icon": "brand:git"}))).await;
    assert_eq!(st, 201, "{git}");
    assert_eq!(git["icon"], json!("brand:git"));
    assert!(git["icon_url"].as_str().unwrap().starts_with("data:image/svg+xml;base64,"), "the server turns the logo into an image");
    let (st, mine) = call(&app, post(json!({"name": "Interne", "packages": ["vim"], "icon": png}))).await;
    assert_eq!(st, 201, "{mine}");
    assert_eq!(mine["icon"], json!(png));
    assert_eq!(mine["icon_url"], Value::Null, "an uploaded image is its own URL");
    let (_, list) = call(&app, TestRequest::get().uri("/api/v1/portal/apps").cookie(ck.clone())).await;
    assert_eq!(list.as_array().unwrap().len(), 2);

    // the picker lists the bundled logos; the suggestions use them
    let (st, logos) = call(&app, TestRequest::get().uri("/api/v1/portal/brand-icons").cookie(ck.clone())).await;
    assert_eq!(st, 200);
    assert!(logos.as_array().unwrap().len() > 100);
    assert!(logos.as_array().unwrap().iter().any(|l| l["slug"] == "firefox" && l["hex"] == "FF7139"));
    let (_, sugg) = call(&app, TestRequest::get().uri("/api/v1/portal/suggestions").cookie(ck.clone())).await;
    let firefox = sugg.as_array().unwrap().iter().find(|s| s["name"] == "Firefox").unwrap();
    assert_eq!(firefox["icon"], json!("brand:firefox"));
    assert!(firefox["icon_url"].as_str().unwrap().starts_with("data:image/svg+xml;base64,"));
    let jq = sugg.as_array().unwrap().iter().find(|s| s["name"] == "jq").unwrap();
    assert_eq!(jq["icon_url"], Value::Null, "an application without a logo keeps its generic icon");
    // adding a suggestion keeps its logo
    let (st, added) = call(&app, post(json!({"name": firefox["name"], "category": firefox["category"], "description": firefox["description"], "icon": firefox["icon"], "recipes": firefox["recipes"]}))).await;
    assert_eq!(st, 201, "{added}");
    assert!(added["icon_url"].is_string());
    // read access for the picker follows the settings permission
    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/portal/brand-icons")).await;
    assert_eq!(st, 401);
}
