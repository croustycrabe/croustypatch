//! Integration: the Docker menu says why a container has no suggested version.
mod support;

use actix_web::cookie::Cookie;
use actix_web::test::TestRequest;
use croustypatch_server::schema::{audit_log, host_packages, hosts, tasks};
use diesel::prelude::*;
use serde_json::json;
use support::http::{admin, app, call, login, session};
use uuid::Uuid;

#[actix_web::test]
async fn the_docker_menu_carries_what_the_registry_checks_found() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-x"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({"docker": true})))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    let add = |name: &str, meta: serde_json::Value| {
        diesel::insert_into(host_packages::table)
            .values((host_packages::host_id.eq(host), host_packages::name.eq(format!("docker:{name}")), host_packages::source.eq("docker"), host_packages::version.eq("ghcr.io/x/y:v1.0.0 @abc"), host_packages::meta.eq(meta)))
            .execute(&mut db.conn())
            .unwrap();
    };
    add("failed", json!({"image": "ghcr.io/x/y:v1.0.0", "check": {"digest": "ok", "tags": "error: registry answered 401"}}));
    add("probed", json!({"image": "ghcr.io/x/y:v1.0.0", "check": {"digest": "ok", "tags": "probed"}, "latest": {"tag": "v1.2.0", "major": false}}));
    add("old-agent", json!({"image": "ghcr.io/x/y:v1.0.0"}));
    let (st, v) = call(&app, TestRequest::get().uri("/api/v1/docker").cookie(ck)).await;
    assert_eq!(st, 200, "{v}");
    let by = |n: &str| v["containers"].as_array().unwrap().iter().find(|c| c["name"] == json!(n)).unwrap().clone();
    assert_eq!(by("failed")["check"], json!({"digest": "ok", "tags": "error: registry answered 401"}));
    assert_eq!(by("failed")["latest"], serde_json::Value::Null);
    assert_eq!((by("probed")["check"]["tags"].clone(), by("probed")["latest"]["tag"].clone()), (json!("probed"), json!("v1.2.0")));
    assert_eq!(by("old-agent")["check"], serde_json::Value::Null, "agents before this release send no check");
}

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

#[actix_web::test]
async fn the_migration_console_offers_templates_and_runs_reviewed_steps_only_for_those_allowed() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let caps = json!({"capabilities": croustypatch_common::protocol::capability::ALL});
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-m"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({"docker": true})), hosts::facts.eq(caps))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    let old: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-old"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({"docker": true})))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    for h in [host, old] {
        diesel::insert_into(host_packages::table)
            .values((host_packages::host_id.eq(h), host_packages::name.eq("docker:hoppscotch-db"), host_packages::source.eq("docker"), host_packages::version.eq("postgres:17-alpine @abc"), host_packages::meta.eq(json!({
                "image": "postgres:17-alpine", "project": "hoppscotch", "service": "db", "compose": {"dir": "/opt/stacks/hoppscotch", "files": ["/opt/stacks/hoppscotch/compose.yaml"]},
                "latest": {"tag": "18-alpine", "major": true, "apply": {"mode": "manual", "reason": "database"}}
            }))))
            .execute(&mut db.conn())
            .unwrap();
    }
    let get = |q: &str, c: &Cookie<'static>| TestRequest::get().uri(&format!("/api/v1/docker/migration?{q}")).cookie(c.clone());
    let (st, m) = call(&app, get(&format!("host_id={host}&container=docker:hoppscotch-db"), &ck)).await;
    assert_eq!(st, 200, "{m}");
    assert_eq!((m["family"].clone(), m["to"].clone(), m["can_run"].clone(), m["agent_ready"].clone(), m["ai"]["available"].clone()), (json!("postgres"), json!("18-alpine"), json!(true), json!(true), json!(false)));
    assert_eq!(m["templates"][0]["id"], json!("db-migrate"));
    assert!(m["templates"][0]["steps"][1]["command"].as_str().unwrap().contains("pg_dumpall"));
    // another tag, a tag that is not one, and no tag at all
    let (_, m2) = call(&app, get(&format!("host_id={host}&container=hoppscotch-db&to=19-alpine"), &ck)).await;
    assert!(m2["templates"][0]["description"].as_str().unwrap().contains("postgres:19-alpine"));
    let (_, bad) = call(&app, get(&format!("host_id={host}&container=hoppscotch-db&to=1%3Bid"), &ck)).await;
    assert_eq!((bad["templates"].clone(), bad["problem"].clone()), (json!([]), json!("not a valid tag")));
    let (st, _) = call(&app, get(&format!("host_id={host}&container=a%3Brm"), &ck)).await;
    assert_eq!(st, 400);
    let (st, _) = call(&app, get(&format!("host_id={host}&container=nope"), &ck)).await;
    assert_eq!(st, 404);

    // running needs docker:shell: super_admin has it, the admin role does not
    let steps = json!([{"title": "Back up", "command": "echo backup"}, {"title": "Stop", "command": "docker stop hoppscotch-db"}]);
    let admin_role = user_with_role(&app, &ck, "adm", "admin").await;
    let post = |c: &Cookie<'static>, body: serde_json::Value| TestRequest::post().uri("/api/v1/docker/shell").cookie(c.clone()).set_json(body);
    let (st, b) = call(&app, post(&admin_role, json!({"host_id": host, "container": "hoppscotch-db", "steps": steps}))).await;
    assert_eq!(st, 403, "an admin cannot run root commands by default: {b}");
    let (_, view) = call(&app, get(&format!("host_id={host}&container=hoppscotch-db"), &admin_role)).await;
    assert_eq!(view["can_run"], json!(false), "but sees the templates");
    // validation
    for bad in [json!({"host_id": host, "container": "hoppscotch-db", "steps": []}), json!({"host_id": host, "container": "hoppscotch-db", "steps": [{"title": "", "command": "x"}]}), json!({"host_id": host, "container": "a b", "steps": steps}), json!({"host_id": host, "container": "hoppscotch-db", "steps": steps, "source": "robot"})] {
        let (st, b) = call(&app, post(&ck, bad.clone())).await;
        assert_eq!(st, 400, "{bad} -> {b}");
    }
    let (st, _) = call(&app, post(&ck, json!({"host_id": host, "container": "unknown-one", "steps": steps}))).await;
    assert_eq!(st, 404);
    let (st, b) = call(&app, post(&ck, json!({"host_id": old, "container": "hoppscotch-db", "steps": steps}))).await;
    assert_eq!(st, 409, "an agent without the capability is told to update: {b}");
    assert!(b["message"].as_str().unwrap().contains("too old"));
    // the run: a task, the commands in the audit log, one at a time
    let (st, r) = call(&app, post(&ck, json!({"host_id": host, "container": "hoppscotch-db", "steps": steps, "source": "ai"}))).await;
    assert_eq!(st, 202, "{r}");
    let (kind, exec): (String, Option<serde_json::Value>) = tasks::table.find(r["task_id"].as_str().unwrap().parse::<Uuid>().unwrap()).select((tasks::kind, tasks::exec)).first(&mut db.conn()).unwrap();
    assert_eq!((kind.as_str(), exec.as_ref().unwrap()["steps"].clone(), exec.unwrap()["source"].clone()), ("docker_shell", steps.clone(), json!("ai")));
    let ev: serde_json::Value = audit_log::table.filter(audit_log::action.eq("docker.shell_requested")).select(audit_log::details).first(&mut db.conn()).unwrap();
    assert_eq!(ev["steps"], steps, "the exact commands are in the audit log");
    let (st, _) = call(&app, post(&ck, json!({"host_id": host, "container": "hoppscotch-db", "steps": steps}))).await;
    assert_eq!(st, 409, "one migration at a time per host");
    // dispatched to the agent as a signed command
    let state = db.state();
    actix_web::rt::time::sleep(std::time::Duration::from_secs(2)).await;
    let cmds = croustypatch_server::gateway::claim_due_commands(&mut db.conn(), &state, host).unwrap();
    let payload: serde_json::Value = serde_json::from_str(&cmds[0].payload).unwrap();
    assert_eq!(payload["action"]["steps"], steps, "{payload}");
    assert_eq!(payload["action"]["type"], json!("docker_shell"));
}

/// Minimal OpenAI-compatible endpoint answering `content` to every chat completion; returns the request bodies.
async fn fake_llm(content: String) -> (String, tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let (tx, content) = (tx.clone(), content.clone());
            tokio::spawn(async move {
                let mut buf = vec![];
                let mut tmp = [0u8; 8192];
                loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 { return; }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= p + 4 + len {
                            let _ = tx.send(serde_json::from_slice(&buf[p + 4..p + 4 + len]).unwrap_or(serde_json::Value::Null));
                            break;
                        }
                    }
                }
                let body = json!({ "choices": [{ "message": { "role": "assistant", "content": content } }] }).to_string();
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            });
        }
    });
    (url, rx)
}

#[actix_web::test]
async fn the_ai_drafts_a_migration_that_a_person_reviews_before_anything_runs() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-ai"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({"docker": true})), hosts::facts.eq(json!({"capabilities": croustypatch_common::protocol::capability::ALL})))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    diesel::insert_into(host_packages::table)
        .values((host_packages::host_id.eq(host), host_packages::name.eq("docker:db"), host_packages::source.eq("docker"), host_packages::version.eq("postgres:17-alpine @abc"), host_packages::meta.eq(json!({"image": "postgres:17-alpine", "project": "app", "service": "db", "compose": {"dir": "/opt/app", "files": ["/opt/app/compose.yaml"]}, "latest": {"tag": "18-alpine", "major": true}, "env": {"POSTGRES_PASSWORD": "hunter2"}}))))
        .execute(&mut db.conn())
        .unwrap();
    let plan = |body: serde_json::Value| TestRequest::post().uri("/api/v1/docker/migration-plan").cookie(ck.clone()).set_json(body);
    let q = json!({"host_id": host, "container": "docker:db"});
    let (st, b) = call(&app, plan(q.clone())).await;
    assert_eq!(st, 409, "without a provider: {b}");
    assert!(b["message"].as_str().unwrap().contains("Settings > AI"));

    let answer = "{\"summary\": \"Dump, move the data aside, restore.\", \"risks\": [\"Extensions must exist in the new image\"], \"steps\": [{\"title\": \"Back up\", \"command\": \"mkdir -p /var/backups/croustypatch && docker exec db pg_dumpall -U postgres > /var/backups/croustypatch/db.sql\"}, {\"title\": \"Remove the old data\", \"command\": \"rm -rf /opt/app/data\"}]}".to_string();
    let (url, mut rx) = fake_llm(answer).await;
    let (st, _) = call(&app, TestRequest::put().uri("/api/v1/settings/ai").cookie(ck.clone()).set_json(json!({"enabled": true, "provider": "openai_compatible", "base_url": url, "model": "test-model", "api_key": ""}))).await;
    assert!(st.is_success());
    let (_, m) = call(&app, TestRequest::get().uri(&format!("/api/v1/docker/migration?host_id={host}&container=db")).cookie(ck.clone())).await;
    assert_eq!(m["ai"]["available"], json!(true), "the page can offer 'do with AI'");
    let (st, p) = call(&app, plan(q.clone())).await;
    assert_eq!(st, 200, "{p}");
    assert_eq!((p["source"].clone(), p["model"].clone(), p["steps"].as_array().unwrap().len()), (json!("ai"), json!("test-model"), 2));
    assert_eq!(p["steps"][0]["warnings"], json!([]));
    assert_eq!(p["steps"][1]["warnings"], json!(["deletes files"]), "a risky command is flagged for the reviewer");
    assert_eq!(p["risks"][0], json!("Extensions must exist in the new image"));
    // what the provider received: the container's names and tags, never environment values
    let sent = rx.recv().await.unwrap().to_string();
    assert!(sent.contains("postgres:18-alpine") && sent.contains("/opt/app/compose.yaml") && sent.contains("baseline_steps"), "{sent}");
    assert!(!sent.contains("hunter2"), "no secret leaves the server");
    // it is a draft: nothing was queued, and the request is in the audit log (without the content)
    let queued: i64 = tasks::table.filter(tasks::kind.eq("docker_shell")).count().get_result(&mut db.conn()).unwrap();
    assert_eq!(queued, 0);
    let ev: serde_json::Value = audit_log::table.filter(audit_log::action.eq("docker.migration_plan_requested")).select(audit_log::details).first(&mut db.conn()).unwrap();
    assert_eq!((ev["model"].clone(), ev["to"].clone()), (json!("test-model"), json!("18-alpine")));
    // without the permission
    let op = user_with_role(&app, &ck, "op", "operator").await;
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/docker/migration-plan").cookie(op).set_json(q)).await;
    assert_eq!(st, 403);
}

#[actix_web::test]
async fn an_unusable_ai_answer_is_reported_not_run() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("docker-ai2"), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"), hosts::modules.eq(json!({"docker": true})))).returning(hosts::id).get_result(&mut db.conn()).unwrap();
    diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(host), host_packages::name.eq("docker:db"), host_packages::source.eq("docker"), host_packages::version.eq("postgres:17 @abc"), host_packages::meta.eq(json!({"image": "postgres:17", "latest": {"tag": "18"}})))).execute(&mut db.conn()).unwrap();
    let (url, _rx) = fake_llm("I cannot help with that.".to_string()).await;
    call(&app, TestRequest::put().uri("/api/v1/settings/ai").cookie(ck.clone()).set_json(json!({"enabled": true, "provider": "openai_compatible", "base_url": url, "model": "m", "api_key": ""}))).await;
    let (st, b) = call(&app, TestRequest::post().uri("/api/v1/docker/migration-plan").cookie(ck.clone()).set_json(json!({"host_id": host, "container": "db"}))).await;
    assert_eq!(st, 409, "{b}");
    assert!(b["message"].as_str().unwrap().contains("did not return a usable plan"), "{b}");
    let (st, b) = call(&app, TestRequest::post().uri("/api/v1/docker/migration-plan").cookie(ck).set_json(json!({"host_id": host, "container": "db", "to": "1;id"}))).await;
    assert_eq!(st, 400, "{b}");
}
