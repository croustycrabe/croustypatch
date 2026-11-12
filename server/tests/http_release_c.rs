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
        add_pkg(&mut c, a, "docker:web", "docker", "nginx:1.27 @aaaa", Some("nginx:1.27 @bbbb"), json!({"image": "nginx:1.27", "project": "site", "service": "web", "state": "running", "health": "healthy", "started_at": "2026-11-01T10:00:00Z", "latest": {"tag": "1.29.0", "major": false, "same_major": null, "apply": {"mode": "file", "file": "/opt/stacks/site/compose.yaml"}}}));
        add_pkg(&mut c, a, "docker:db", "docker", "postgres:17 @cccc", None, json!({"image": "postgres:17", "state": "running"}));
        add_pkg(&mut c, b, "docker:cache", "docker", "redis:7 @dddd", Some("redis:7 @eeee"), json!({"image": "redis:7", "state": "exited", "latest": {"tag": "8", "major": true, "same_major": "7.4"}}));
        add_pkg(&mut c, b, "openssl", "system", "3.0", Some("3.1"), json!({}));
        (a, b)
    };
    let get = |q: &str| TestRequest::get().uri(&format!("/api/v1/docker{q}")).cookie(ck.clone());
    let (st, all) = call(&app, get("")).await;
    assert_eq!(st, 200, "{all}");
    assert_eq!(all["summary"], json!({"containers": 3, "with_updates": 2, "with_newer_version": 2, "hosts": 2}), "system packages are not containers");
    let web = all["containers"].as_array().unwrap().iter().find(|c| c["name"] == "web").unwrap();
    assert_eq!((web["image"].clone(), web["current"].clone(), web["candidate"].clone(), web["update"].clone()), (json!("nginx:1.27"), json!("aaaa"), json!("bbbb"), json!(true)));
    assert_eq!((web["project"].clone(), web["service"].clone(), web["state"].clone(), web["health"].clone()), (json!("site"), json!("web"), json!("running"), json!("healthy")));
    // a newer version tag is information next to the image update the running tag tracks
    assert_eq!(web["latest"], json!({"tag": "1.29.0", "major": false, "same_major": null, "apply": {"mode": "file", "file": "/opt/stacks/site/compose.yaml"}}), "the page learns whether it can be applied, and by changing which file");
    let db_row = all["containers"].as_array().unwrap().iter().find(|c| c["name"] == "db").unwrap();
    assert_eq!(db_row["latest"], Value::Null, "no newer version known");
    let cache = all["containers"].as_array().unwrap().iter().find(|c| c["name"] == "cache").unwrap();
    assert_eq!((cache["latest"]["tag"].clone(), cache["latest"]["major"].clone(), cache["latest"]["same_major"].clone()), (json!("8"), json!(true), json!("7.4")));
    assert_eq!(cache["latest"]["apply"], Value::Null, "an older agent says nothing about it: the page then asks to update the agent");
    let (_, newer) = call(&app, get("?newer=true")).await;
    let names: Vec<&str> = newer["containers"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["web", "cache"], "only the containers with a newer version");
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

    // an update in progress is shown on the containers that are in it, not on every container of the host
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/tasks").cookie(ck.clone()).set_json(json!({"host_ids": [a], "scope": "docker", "packages": ["docker:web"]}))).await;
    assert!(st.is_success());
    let (_, only_a) = call(&app, get(&format!("?host={a}"))).await;
    let busy = |n: &str| only_a["containers"].as_array().unwrap().iter().find(|c| c["name"] == n).unwrap()["open_task"].clone();
    assert_eq!(busy("web")["status"], json!("queued"), "the container being updated");
    assert_eq!(busy("db"), Value::Null, "the other container of the same host is not");
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

/// A `compliance` setting saved by 0.6 has no `auto_install`: it reads as on, as the code that uses it assumes.
#[actix_web::test]
async fn a_compliance_setting_saved_before_0_7_reads_with_the_new_defaults() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    diesel::sql_query("INSERT INTO settings (key, value) VALUES ('compliance', '{\"threshold\": 85, \"profile\": \"\"}')").execute(&mut db.conn()).unwrap();
    let (st, s) = call(&app, TestRequest::get().uri("/api/v1/settings").cookie(ck.clone())).await;
    assert_eq!(st, 200, "{s}");
    assert_eq!((s["compliance"]["threshold"].clone(), s["compliance"]["auto_install"].clone()), (json!(85), json!(true)), "{s}");
    // and saving it back without the key is accepted
    let (st, b) = call(&app, TestRequest::put().uri("/api/v1/settings/compliance").cookie(ck.clone()).set_json(json!({"threshold": 90, "profile": ""}))).await;
    assert!(st.is_success(), "{b}");
}

/// Applications added before 0.7 kept a generic icon; the migration gives them the logo of the suggestion they match.
#[actix_web::test]
async fn the_catalog_logos_migration_upgrades_generic_icons_only() {
    let db = support::TestDb::new();
    {
        let mut c = db.conn();
        // the state of a 0.6 catalog, then the migration's statements run again on it
        for (name, icon) in [("Firefox", "browser"), ("git", "app-window"), ("GIMP", "image"), ("Python 3", "code"), ("ufw", "app-window"), ("Interne", "code"), ("Docker", "brand:podman"), ("Krita", "data:image/png;base64,AAAA")] {
            diesel::sql_query(format!("INSERT INTO portal_apps (name, icon, recipes) VALUES ('{name}', '{icon}', '{{\"*\": {{\"packages\": [\"x\"]}}}}')")).execute(&mut c).unwrap();
        }
        let up = include_str!("../migrations/2026-11-11-000014_catalog_logos/up.sql");
        for stmt in up.lines().filter(|l| l.starts_with("UPDATE")) {
            diesel::sql_query(stmt).execute(&mut c).unwrap();
        }
    }
    let icons: std::collections::HashMap<String, String> = croustypatch_server::schema::portal_apps::table
        .select((croustypatch_server::schema::portal_apps::name, croustypatch_server::schema::portal_apps::icon))
        .load::<(String, String)>(&mut db.conn())
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(icons["Firefox"], "brand:firefox");
    assert_eq!(icons["git"], "brand:git", "the name matches whatever the case");
    assert_eq!(icons["GIMP"], "brand:gimp");
    assert_eq!(icons["Python 3"], "brand:python");
    assert_eq!(icons["ufw"], "app-window", "no logo for it: unchanged");
    assert_eq!(icons["Interne"], "code", "not a suggested application");
    assert_eq!(icons["Docker"], "brand:podman", "a logo chosen by hand is kept");
    assert!(icons["Krita"].starts_with("data:image/png"), "so is an uploaded image");
}

#[actix_web::test]
async fn a_new_app_with_the_default_icon_takes_the_logo_of_its_name() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let post = |b: Value| TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(b);
    let (_, a) = call(&app, post(json!({"name": "Firefox ESR", "packages": ["firefox-esr"]}))).await;
    assert_eq!(a["icon"], json!(""), "no logo is guessed for a name that is not one: the generic default stays");
    let (_, a) = call(&app, post(json!({"name": "vlcmediaplayer", "packages": ["vlc"], "icon": "app-window"}))).await;
    assert_eq!(a["icon"], json!("brand:vlcmediaplayer"));
    assert!(a["icon_url"].is_string());
    let (_, a) = call(&app, post(json!({"name": "Git", "packages": ["git"]}))).await;
    assert_eq!(a["icon"], json!("brand:git"), "a hand-written entry looks like a suggested one");
    let (_, a) = call(&app, post(json!({"name": "Docker", "packages": ["docker.io"], "icon": "cube"}))).await;
    assert_eq!(a["icon"], json!("cube"), "an icon chosen on purpose is respected");
    // editing keeps the choice, even back to the generic icon
    let (st, a2) = call(&app, TestRequest::put().uri(&format!("/api/v1/portal/apps/{}", a["id"].as_str().unwrap())).cookie(ck.clone()).set_json(json!({"name": "Docker", "packages": ["docker.io"], "icon": "app-window"}))).await;
    assert_eq!(st, 200);
    assert_eq!(a2["icon"], json!("app-window"));
}

// ------------------------------------------------------------------- system, Docker, or both

#[actix_web::test]
async fn updates_can_be_started_for_the_system_the_containers_or_both() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (both, sys_only, docker_only) = {
        let mut c = db.conn();
        let both = add_host(&mut c, "srv-both", json!({"docker": true}), json!({}));
        let sys_only = add_host(&mut c, "srv-sys", json!({"docker": true}), json!({}));
        let docker_only = add_host(&mut c, "srv-docker", json!({"docker": true}), json!({}));
        for h in [both, sys_only] {
            add_pkg(&mut c, h, "openssl", "system", "3.0", Some("3.1"), json!({}));
            add_pkg(&mut c, h, "curl", "system", "8.0", Some("8.1"), json!({}));
        }
        for h in [both, docker_only] {
            add_pkg(&mut c, h, "docker:cache", "docker", "redis:7 @aa", Some("redis:7 @bb"), json!({"image": "redis:7"}));
        }
        (both, sys_only, docker_only)
    };
    let post = |b: Value| TestRequest::post().uri("/api/v1/tasks").cookie(ck.clone()).set_json(b);
    let kinds = |h: Uuid| -> Vec<(String, Vec<String>)> {
        let mut v: Vec<(String, Value)> = tasks::table.filter(tasks::host_id.eq(h)).select((tasks::kind, tasks::packages)).load(&mut db.conn()).unwrap();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.into_iter().map(|(k, p)| (k, p.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect())).collect()
    };

    // the hosts list says how many of the pending updates are container images
    let (_, list) = call(&app, TestRequest::get().uri("/api/v1/hosts").cookie(ck.clone())).await;
    let row = |n: &str| list.as_array().unwrap().iter().find(|h| h["hostname"] == n).unwrap().clone();
    assert_eq!((row("srv-both")["updates"].clone(), row("srv-both")["docker_updates"].clone()), (json!(3), json!(1)));
    assert_eq!((row("srv-sys")["updates"].clone(), row("srv-sys")["docker_updates"].clone()), (json!(2), json!(0)));

    // the system only: no container task is created
    let (st, r) = call(&app, post(json!({"host_ids": [both], "scope": "system"}))).await;
    assert_eq!(st, 202, "{r}");
    assert_eq!(r["tasks"].as_array().unwrap().len(), 1);
    let k = kinds(both);
    assert_eq!(k.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["patch"]);
    assert_eq!(k[0].1.len(), 2, "both system updates, and not the container: {k:?}");
    assert!(!k[0].1.iter().any(|n| n.starts_with("docker:")));
    // Docker only: a container task, while the system patch is still open (each kind waits for its own)
    let (st, r) = call(&app, post(json!({"host_ids": [both], "scope": "docker"}))).await;
    assert_eq!(st, 202, "{r}");
    assert_eq!(kinds(both).iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["docker", "patch"]);
    assert_eq!(kinds(both)[0].1, vec!["docker:cache"]);
    // a second request of a kind that is open is refused, with the reason
    let (st, r) = call(&app, post(json!({"host_ids": [both], "scope": "system"}))).await;
    assert_eq!(st, 409, "{r}");
    assert!(r["message"].as_str().unwrap().contains("open patch task"), "{r}");
    let (st, r) = call(&app, post(json!({"host_ids": [both], "scope": "docker"}))).await;
    assert_eq!(st, 409, "{r}");
    assert!(r["message"].as_str().unwrap().contains("open container task"), "{r}");

    // the default is both, as before
    let (st, r) = call(&app, post(json!({"host_ids": [docker_only]}))).await;
    assert_eq!(st, 202, "{r}");
    let (st, _) = call(&app, post(json!({"host_ids": [sys_only], "scope": "all"}))).await;
    assert_eq!(st, 202);
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    let (_, r) = call(&app, post(json!({"host_ids": [both]}))).await;
    assert_eq!(r["tasks"].as_array().unwrap().len(), 2, "no scope: the system patch and the container update: {r}");

    // a scope with nothing to do says which
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    let (st, r) = call(&app, post(json!({"host_ids": [sys_only], "scope": "docker"}))).await;
    assert_eq!(st, 409);
    assert!(r["message"].as_str().unwrap().contains("no pending container updates"), "{r}");
    let (st, r) = call(&app, post(json!({"host_ids": [docker_only], "scope": "system"}))).await;
    assert_eq!(st, 409);
    assert!(r["message"].as_str().unwrap().contains("no pending system updates"), "{r}");
    // several hosts: the ones with something of that kind get a task, the others are skipped without failing the request
    let (st, r) = call(&app, post(json!({"host_ids": [sys_only, docker_only, both], "scope": "docker"}))).await;
    assert_eq!(st, 202, "{r}");
    let hosts: std::collections::HashSet<String> = r["tasks"].as_array().unwrap().iter().map(|t| t["host_id"].as_str().unwrap().to_string()).collect();
    assert_eq!(hosts, [docker_only.to_string(), both.to_string()].into_iter().collect(), "{r}");
    // an unknown scope is a bad request, and a named package still has to be of the scope
    let (st, _) = call(&app, post(json!({"host_ids": [both], "scope": "everything"}))).await;
    assert_eq!(st, 400);
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    let (st, r) = call(&app, post(json!({"host_ids": [both], "scope": "system", "packages": ["docker:cache"]}))).await;
    assert_eq!(st, 409, "a container named under the system scope is not taken: {r}");
    // audited with the scope
    let scopes: Vec<Value> = audit_log::table.filter(audit_log::action.eq("task.patch_requested")).select(audit_log::details).load(&mut db.conn()).unwrap();
    assert!(scopes.iter().any(|d| d["scope"] == json!("system")) && scopes.iter().any(|d| d["scope"] == json!("docker")) && scopes.iter().any(|d| d["scope"] == json!("all")), "{scopes:?}");
}

#[actix_web::test]
async fn the_editors_are_added_from_the_suggestions_only_by_someone_who_may_write_commands() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (st, sugg) = call(&app, TestRequest::get().uri("/api/v1/portal/suggestions").cookie(ck.clone())).await;
    assert_eq!(st, 200);
    let find = |n: &str| sugg.as_array().unwrap().iter().find(|s| s["name"] == n).unwrap_or_else(|| panic!("{n} is not suggested")).clone();
    let (code, zed) = (find("Visual Studio Code"), find("Zed"));
    assert!(zed["icon_url"].as_str().unwrap().starts_with("data:image/svg+xml;base64,"), "Zed shows its logo");
    assert_eq!(code["icon_url"], Value::Null, "VS Code keeps the generic glyph");
    let body = |s: &Value| json!({"name": s["name"], "category": s["category"], "description": s["description"], "icon": s["icon"], "recipes": s["recipes"], "group_ids": [], "enabled": true});

    // an administrator without portal:commands cannot add an application that installs with a command ...
    let admin_only = user_with_role(&app, &ck, "adm2", "admin").await;
    let (st, b) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(admin_only.clone()).set_json(body(&code))).await;
    assert_eq!(st, 403, "{b}");
    assert!(b["message"].as_str().unwrap().contains("portal:commands"), "{b}");
    // ... the full access account can, and the command is kept as written
    let (st, a) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(body(&code))).await;
    assert_eq!(st, 201, "{a}");
    assert_eq!(a["has_command"], json!(true));
    assert!(a["recipes"]["apt"]["command"]["install"].as_str().unwrap().contains("packages.microsoft.com"));
    assert!(a["recipes"]["pacman"]["command"]["install"].as_str().unwrap().contains("visual-studio-code-bin"), "Arch gets Microsoft's build from the AUR");
    let (st, z) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(ck.clone()).set_json(body(&zed))).await;
    assert_eq!(st, 201, "{z}");
    let oss = find("Code - OSS");
    let (st, o) = call(&app, TestRequest::post().uri("/api/v1/portal/apps").cookie(admin_only.clone()).set_json(body(&oss))).await;
    assert_eq!(st, 201, "a packages-only application needs no command permission: {o}");
    assert_eq!(z["icon"], json!("brand:zedindustries"));
    // they are then listed as in the catalog
    let (_, sugg) = call(&app, TestRequest::get().uri("/api/v1/portal/suggestions").cookie(ck.clone())).await;
    assert!(sugg.as_array().unwrap().iter().filter(|s| s["name"] == "Zed" || s["name"] == "Visual Studio Code").all(|s| s["in_catalog"] == json!(true)));

    // each system sees what it can install: Debian runs the command, Arch installs the package, FreeBSD is not offered either
    let (deb, arch, bsd) = {
        let mut c = db.conn();
        let mk = |c: &mut PgConnection, n: &str, pm: &str| diesel::insert_into(hosts::table).values((hosts::hostname.eq(n), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::package_manager.eq(pm), hosts::modules.eq(json!({"portal": true})), hosts::facts.eq(json!({"capabilities": croustypatch_common::protocol::capability::ALL})))).returning(hosts::id).get_result::<Uuid>(c).unwrap();
        (mk(&mut c, "deb", "apt"), mk(&mut c, "arch", "pacman"), mk(&mut c, "bsd", "pkg"))
    };
    let view = |h: Uuid| TestRequest::get().uri(&format!("/api/v1/portal/hosts/{h}")).cookie(ck.clone());
    let app_of = |v: &Value, n: &str| v["apps"].as_array().unwrap().iter().find(|a| a["name"] == n).cloned();
    let (_, v) = call(&app, view(deb)).await;
    assert_eq!(app_of(&v, "Visual Studio Code").unwrap()["kind"], json!("command"));
    assert_eq!(app_of(&v, "Zed").unwrap()["command"]["check"], json!("/usr/local/bin/zed"));
    let (_, v) = call(&app, view(arch)).await;
    let c = app_of(&v, "Visual Studio Code").unwrap();
    assert_eq!((c["kind"].clone(), c["command"]["check"].clone()), (json!("command"), json!("/usr/bin/code")), "Microsoft's build on Arch: built from the AUR by a command");
    assert!(c["command"]["install"].as_str().unwrap().contains("visual-studio-code-bin"));
    assert_eq!(app_of(&v, "Zed").unwrap()["packages"], json!(["zed"]));
    assert_eq!(app_of(&v, "Code - OSS").unwrap()["packages"], json!(["code"]), "the open-source build is the repository package");
    let (_, v) = call(&app, view(deb)).await;
    assert!(app_of(&v, "Code - OSS").is_none(), "and is not offered on Debian");
    let (_, v) = call(&app, view(bsd)).await;
    assert!(app_of(&v, "Zed").is_none() && app_of(&v, "Visual Studio Code").is_none(), "no recipe for pkg: not offered");
}

// ------------------------------------------------------------------ apply the suggested Docker version

#[actix_web::test]
async fn a_suggested_docker_version_is_applied_to_named_containers_from_what_the_agent_reported() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let caps = json!({"capabilities": ["settings", "app_command", "self_update", "docker_retag"]});
    let (host, old_agent) = {
        let mut c = db.conn();
        let host = add_host(&mut c, "docker-a", json!({"docker": true}), caps.clone());
        let old_agent = add_host(&mut c, "docker-old", json!({"docker": true}), json!({"capabilities": ["settings", "app_command", "self_update"]}));
        let latest = |tag: &str, major: bool, apply: Value| json!({"image": "x", "latest": {"tag": tag, "major": major, "same_major": null, "apply": apply}});
        let mk = |c: &mut PgConnection, h: Uuid, name: &str, image: &str, tag: &str, major: bool, apply: Value, candidate: Option<&str>| {
            let mut meta = latest(tag, major, apply);
            meta["image"] = json!(image);
            add_pkg(c, h, name, "docker", &format!("{image} @aaaa"), candidate, meta);
        };
        mk(&mut c, host, "docker:web", "nginx:1.27", "1.29.0", false, json!({"mode": "file", "file": "/opt/stacks/site/compose.yaml"}), Some("nginx:1.27 @bbbb"));
        mk(&mut c, host, "docker:db", "grafana/grafana:10.4", "11.0", true, json!({"mode": "file", "file": "/opt/stacks/site/compose.yaml"}), None);
        mk(&mut c, host, "docker:pg", "postgres:17-alpine", "18-alpine", true, json!({"mode": "file", "file": "/opt/stacks/site/compose.yaml"}), None);
        mk(&mut c, host, "docker:cache", "redis:7", "7.4", false, json!({"mode": "manual", "reason": "the tag comes from a variable"}), None);
        add_pkg(&mut c, host, "docker:plain", "docker", "busybox:1 @cc", None, json!({"image": "busybox:1"}));
        add_pkg(&mut c, host, "openssl", "system", "3.0", Some("3.1"), json!({}));
        mk(&mut c, old_agent, "docker:web", "nginx:1.27", "1.29.0", false, json!({"mode": "file", "file": "/x"}), None);
        (host, old_agent)
    };
    let post = |b: Value| TestRequest::post().uri("/api/v1/tasks").cookie(ck.clone()).set_json(b);
    let task_of = |h: Uuid| -> (String, Value, Value) {
        let mut v: Vec<(String, Option<Value>, Value)> = tasks::table.filter(tasks::host_id.eq(h)).order(tasks::created_at.desc()).select((tasks::kind, tasks::exec, tasks::packages)).load(&mut db.conn()).unwrap();
        let (k, e, p) = v.remove(0);
        (k, e.unwrap_or_default(), p)
    };

    // what a request must look like
    for bad in [
        json!({"host_ids": [host], "scope": "docker", "to_latest": true}),
        json!({"host_ids": [host], "scope": "system", "packages": ["docker:web"], "to_latest": true}),
        json!({"host_ids": [host], "scope": "all", "packages": ["docker:web"], "to_latest": true}),
        json!({"host_ids": [host], "scope": "docker", "packages": ["openssl"], "to_latest": true}),
    ] {
        let (st, b) = call(&app, post(bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    // an agent that cannot change a compose file would pull the same tag and report success: it is never sent one
    let (st, b) = call(&app, post(json!({"host_ids": [old_agent], "scope": "docker", "packages": ["docker:web"], "to_latest": true}))).await;
    assert_eq!(st, 409, "{b}");
    assert!(b["message"].as_str().unwrap().contains("update it first"), "{b}");
    // what the agent says cannot be done, or has no newer version, is refused with the reason
    let (st, b) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:cache"], "to_latest": true}))).await;
    assert_eq!(st, 409);
    assert!(b["message"].as_str().unwrap().contains("by hand") && b["message"].as_str().unwrap().contains("variable"), "{b}");
    let (st, b) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:plain"], "to_latest": true}))).await;
    assert_eq!(st, 409);
    assert!(b["message"].as_str().unwrap().contains("no newer version"), "{b}");
    // a major version is not applied by accident
    let (st, b) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:db"], "to_latest": true}))).await;
    assert_eq!(st, 409);
    assert!(b["message"].as_str().unwrap().contains("major version change") && b["message"].as_str().unwrap().contains("allow_major"), "{b}");
    let none: i64 = tasks::table.count().get_result(&mut db.conn()).unwrap();
    assert_eq!(none, 0, "nothing was created by the refusals");

    // the image comes from the inventory (the running image with the suggested tag), whatever the request says
    let (st, r) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:web"], "to_latest": true, "retag": {"docker:web": "evil/web:666"}, "image": "evil/web:666"}))).await;
    assert_eq!(st, 202, "{r}");
    let (kind, exec, pk) = task_of(host);
    assert_eq!(kind, "docker");
    assert_eq!(exec["retag"], json!({"docker:web": "nginx:1.29.0"}));
    assert!(exec.get("soak_minutes").is_some(), "the policy's options travel with it: {exec}");
    assert_eq!((pk[0]["name"].clone(), pk[0]["to"].clone()), (json!("docker:web"), json!("nginx:1.29.0")));
    let state = db.state();
    actix_web::rt::time::sleep(std::time::Duration::from_secs(2)).await; // a remote test database may run a moment ahead of this machine
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, host).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["type"], json!("apply_updates"));
    assert_eq!(payload["action"]["retag"], json!({"docker:web": "nginx:1.29.0"}), "{payload}");
    assert_eq!(payload["action"]["packages"], json!(["docker:web"]));
    assert!(payload["action"]["exec"].get("soak_minutes").is_some(), "{payload}");
    // one container task at a time, and the reason is given
    let (st, b) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:web"], "to_latest": true}))).await;
    assert_eq!(st, 409);
    assert!(b["message"].as_str().unwrap().contains("open container task"), "{b}");
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();

    // a database's major version is never applied this way, confirmed or not: the new image cannot open the old data
    for allow in [false, true] {
        let (st, b) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:pg"], "to_latest": true, "allow_major": allow}))).await;
        assert_eq!(st, 409, "{b}");
        assert!(b["message"].as_str().unwrap().contains("data migration"), "{b}");
    }
    // a major version needs the explicit confirmation, then it goes
    let (st, r) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:db"], "to_latest": true, "allow_major": true}))).await;
    assert_eq!(st, 202, "{r}");
    assert_eq!(task_of(host).1["retag"], json!({"docker:db": "grafana/grafana:11.0"}));
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();

    // an ordinary container update of the same container pulls the running tag and carries no new image
    let (st, r) = call(&app, post(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:web"]}))).await;
    assert_eq!(st, 202, "{r}");
    assert!(task_of(host).1.get("retag").is_none());
    actix_web::rt::time::sleep(std::time::Duration::from_secs(2)).await;
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, host).unwrap();
    let payload: Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert!(payload["action"].get("retag").is_none(), "no field at all for an agent that does not know it: {payload}");

    // audited with what was asked
    let details: Vec<Value> = audit_log::table.filter(audit_log::action.eq("task.patch_requested")).select(audit_log::details).load(&mut db.conn()).unwrap();
    assert!(details.iter().any(|d| d["retag"] == json!({"docker:web": "nginx:1.29.0"}) && d["allow_major"] == json!(false)), "{details:?}");
    assert!(details.iter().any(|d| d["retag"] == json!({"docker:db": "grafana/grafana:11.0"}) && d["allow_major"] == json!(true)));
    // read-only users cannot
    let auditor = user_with_role(&app, &ck, "audit2", "auditor").await;
    diesel::update(tasks::table).set(tasks::status.eq("succeeded")).execute(&mut db.conn()).unwrap();
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/tasks").cookie(auditor).set_json(json!({"host_ids": [host], "scope": "docker", "packages": ["docker:web"], "to_latest": true}))).await;
    assert_eq!(st, 403);
}

#[test]
fn the_new_image_is_the_running_one_with_the_suggested_tag() {
    use croustypatch_server::api::patching::retag_ref;
    assert_eq!(retag_ref("ghcr.io/immich-app/immich-server:v3.3.1", "v3.4.1").as_deref(), Some("ghcr.io/immich-app/immich-server:v3.4.1"));
    assert_eq!(retag_ref("localhost:5000/app:1.0", "2.0").as_deref(), Some("localhost:5000/app:2.0"));
    assert_eq!(retag_ref("postgres:17-alpine", "18-alpine").as_deref(), Some("postgres:18-alpine"));
    assert_eq!(retag_ref("localhost:5000/app", "2.0"), None, "a registry port is not a tag");
    assert_eq!(retag_ref("nginx", "2.0"), None);
    assert_eq!(retag_ref("nginx:1.27", "1.28; rm -rf /"), None);
    assert_eq!(retag_ref("nginx:1.27", "../x"), None);
    assert_eq!(retag_ref("nginx:1.27", ""), None);
    assert_eq!(retag_ref("nginx@sha256:abcd", "1.28"), None);
}

/// Visual Studio Code added to the catalog before 0.7.5 gets the source of its latest version, so its updates show.
#[actix_web::test]
async fn the_vscode_latest_migration_completes_older_catalog_entries_only() {
    let db = support::TestDb::new();
    {
        let mut c = db.conn();
        let old = json!({"pacman": {"command": {"install": "set -eu\ngit clone https://aur.archlinux.org/visual-studio-code-bin.git", "uninstall": "pacman -Rns --noconfirm visual-studio-code-bin", "check": "/usr/bin/code"}}, "apt": {"command": {"install": "x", "uninstall": "y", "check": "/usr/bin/code"}}});
        for (name, recipes) in [("Visual Studio Code", old.clone()), ("Other tool", json!({"pacman": {"command": {"install": "make install", "uninstall": "rm", "check": "/usr/bin/t"}}})), ("Code - OSS", json!({"pacman": {"packages": ["code"]}}))] {
            diesel::sql_query(format!("INSERT INTO portal_apps (name, icon, recipes) VALUES ('{name}', '', '{}')", recipes.to_string().replace('\'', "''"))).execute(&mut c).unwrap();
        }
        for stmt in include_str!("../migrations/2026-11-12-000015_vscode_latest/up.sql").split(';').filter(|s| s.contains("UPDATE")) {
            diesel::sql_query(stmt).execute(&mut c).unwrap();
        }
    }
    let rows: Vec<(String, Value)> = croustypatch_server::schema::portal_apps::table.select((croustypatch_server::schema::portal_apps::name, croustypatch_server::schema::portal_apps::recipes)).load(&mut db.conn()).unwrap();
    let get = |n: &str| rows.iter().find(|r| r.0 == n).unwrap().1.clone();
    assert_eq!(get("Visual Studio Code")["pacman"]["command"]["latest"], json!({"aur": "visual-studio-code-bin"}));
    assert_eq!(get("Visual Studio Code")["apt"]["command"].get("latest"), None, "Debian's recipe is updated by apt itself");
    assert_eq!(get("Other tool")["pacman"]["command"].get("latest"), None);
    assert_eq!(get("Code - OSS"), json!({"pacman": {"packages": ["code"]}}));
}

/// New system updates and new container updates are two separate alerts, raised once per new update.
#[actix_web::test]
async fn new_system_and_container_updates_raise_separate_alerts_once() {
    use croustypatch_common::protocol::{AgentMessage, HostFacts, Inventory, PackageEntry};
    let db = support::TestDb::new();
    let state = actix_web::web::Data::new(db.state());
    let host = add_host(&mut db.conn(), "srv-alerts", json!({"docker": true}), json!({}));
    let p = |name: &str, source: &str, cand: Option<&str>, sec: bool, meta: Value| PackageEntry { name: name.into(), version: "1".into(), candidate: cand.map(str::to_owned), is_security: sec, cves: vec![], source: source.into(), meta };
    let send = |pkgs: Vec<PackageEntry>| {
        let st = state.clone();
        async move { croustypatch_server::gateway::handle_messages(&st, host, vec![AgentMessage::Inventory(Inventory { facts: HostFacts { hostname: "srv-alerts".into(), ..Default::default() }, packages: pkgs, reboot_required: false, collected_at: chrono::Utc::now(), app_updates: vec![] })]).await }
    };
    let events = || -> Vec<(String, Value)> { audit_log::table.filter(audit_log::action.like("updates.%")).order(audit_log::seq.asc()).select((audit_log::action, audit_log::details)).load(&mut db.conn()).unwrap() };
    // the first inventory of a host is its starting point, not news
    send(vec![p("bash", "system", Some("2"), false, json!({}))]).await;
    assert!(events().is_empty());
    // then: a new security update, a new image and a newly suggested version
    send(vec![p("bash", "system", Some("2"), false, json!({})), p("openssl", "system", Some("3.1"), true, json!({})), p("docker:web", "docker", Some("nginx:1.27 @bb"), false, json!({"image": "nginx:1.27"})), p("docker:db", "docker", None, false, json!({"image": "redis:7", "latest": {"tag": "7.4"}}))]).await;
    let ev = events();
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert_eq!(ev[0].0, "updates.system_available");
    assert_eq!((ev[0].1["count"].clone(), ev[0].1["security"].clone(), ev[0].1["packages"].clone()), (json!(1), json!(1), json!(["openssl"])), "bash was already pending");
    assert_eq!(ev[1].0, "updates.docker_available");
    assert_eq!(ev[1].1["count"], json!(2));
    // the same inventory again: nothing new, no alert
    send(vec![p("bash", "system", Some("2"), false, json!({})), p("openssl", "system", Some("3.1"), true, json!({})), p("docker:web", "docker", Some("nginx:1.27 @bb"), false, json!({"image": "nginx:1.27"})), p("docker:db", "docker", None, false, json!({"image": "redis:7", "latest": {"tag": "7.4"}}))]).await;
    assert_eq!(events().len(), 2);
    // a newer candidate of a known package is news again
    send(vec![p("bash", "system", Some("3"), false, json!({}))]).await;
    assert_eq!(events().len(), 3);
    // they are notification events of their own
    let queued: Vec<String> = croustypatch_server::schema::notifications::table.select(croustypatch_server::schema::notifications::event).load(&mut db.conn()).unwrap_or_default();
    let _ = queued; // routing depends on rules; the catalog entries are what matter here
    assert!(croustypatch_server::notify::event_key("updates.system_available", &json!({})).is_some() && croustypatch_server::notify::event_key("updates.docker_available", &json!({})).is_some());
}

#[test]
fn the_digest_is_an_html_page_grouped_by_severity_worst_first() {
    let html = croustypatch_server::notify::digest_html("Infra <team>", &[
        ("info".into(), "web-01: Patch applied".into()),
        ("critical".into(), "db-01: Host quarantined".into()),
        ("medium".into(), "web-01: 3 new system update(s) <openssl>".into()),
        ("info".into(), "web-02: Patch applied".into()),
    ]);
    assert!(html.starts_with("<!doctype html>") && html.contains("Daily summary") && html.contains("4 event(s)"));
    let (c, m, i) = (html.find("critical · 1").unwrap(), html.find("medium · 1").unwrap(), html.find("info · 2").unwrap());
    assert!(c < m && m < i, "worst first");
    assert!(html.contains("Infra &lt;team&gt;") && html.contains("&lt;openssl&gt;") && !html.contains("<openssl>"), "everything is escaped");
    assert!(!html.contains("high ·"), "no empty section");
}

/// The air-gapped switch reaches the agents (who then skip registries and the AUR).
#[actix_web::test]
async fn the_air_gapped_mode_is_sent_to_the_agents() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host = add_host(&mut db.conn(), "srv-off", json!({}), json!({"capabilities": ["settings"]}));
    let offline = || croustypatch_server::modules::settings_for(&mut db.conn(), host).unwrap().unwrap().offline;
    assert!(!offline(), "connected by default");
    let (st, b) = call(&app, TestRequest::put().uri("/api/v1/settings/connectivity").cookie(ck.clone()).set_json(json!({"mode": "air_gapped"}))).await;
    assert!(st.is_success(), "{b}");
    assert!(offline());
    call(&app, TestRequest::put().uri("/api/v1/settings/connectivity").cookie(ck.clone()).set_json(json!({"mode": "connected"}))).await;
    assert!(!offline());
}

#[test]
fn a_fleet_default_profile_a_host_lacks_gives_way_to_its_recommended_one() {
    use croustypatch_server::compliance::fleet_default_for;
    let cis = "xccdf_org.ssgproject.content_profile_cis_level1_server";
    let ubuntu = json!({"profiles": [{"id": cis, "title": "CIS"}, {"id": "xccdf_org.ssgproject.content_profile_stig", "title": "STIG"}], "recommended": cis});
    let debian = json!({"profiles": [{"id": "xccdf_org.ssgproject.content_profile_standard", "title": "Standard"}]});
    assert_eq!(fleet_default_for(cis, Some(&ubuntu)), cis, "the host has it");
    assert_eq!(fleet_default_for(cis, Some(&debian)), "", "Debian's content has no CIS for Ubuntu: the agent uses its recommended one");
    assert_eq!(fleet_default_for(cis, None), cis, "content not known yet: the choice is sent as is");
    assert_eq!(fleet_default_for(cis, Some(&json!({"profiles": []}))), cis);
    assert_eq!(fleet_default_for("", Some(&ubuntu)), "", "automatic");
    let custom = "custom:6a0c3f9e-2f6c-4d55-9a3f-0b1f7d0f2a11";
    assert_eq!(fleet_default_for(custom, Some(&debian)), custom, "a custom profile is kept (the agent says when its base is missing)");
}
