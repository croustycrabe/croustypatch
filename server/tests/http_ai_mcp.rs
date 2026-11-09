//! Integration: AI-assisted risk (parsing, fallbacks, planner, provider round trip) and the MCP server with
//! personal API tokens.
mod support;

use actix_web::test::TestRequest;
use croustypatch_common::model::{PolicyAction, RiskLevel};
use croustypatch_server::ai::{self, Advisor, Verdict};
use croustypatch_server::engine::policy::PolicyDef;
use croustypatch_server::engine::risk::Assessment;
use croustypatch_server::planner;
use croustypatch_server::schema::{host_packages, hosts, policies, tasks};
use diesel::prelude::*;
use serde_json::{json, Value};
use support::http::{admin, app, call};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[test]
fn model_answers_are_parsed_strictly() {
    let v = ai::parse_verdict("```json\n{\"contextual_risk\": \"high\", \"rationale\": [\"exposed\\nto internet\", \"KEV\"], \"confidence\": 1.7}\n```").unwrap();
    assert_eq!(v.contextual_risk, RiskLevel::High);
    assert_eq!(v.rationale, vec!["exposed to internet".to_string(), "KEV".into()], "control characters flattened");
    assert_eq!(v.confidence, 1.0, "confidence clamped");
    assert!(ai::parse_verdict("{\"contextual_risk\": \"apocalyptic\"}").is_err(), "out-of-range levels are refused");
    assert!(ai::parse_verdict("I think it is high").is_err());
    assert!(ai::parse_verdict("{\"rationale\": []}").is_err());
}

struct Fake(Result<Verdict, String>);
impl Advisor for Fake {
    fn contextual(&self, _: &Value) -> anyhow::Result<Verdict> {
        self.0.clone().map_err(|e| anyhow::anyhow!(e))
    }
    fn label(&self) -> String {
        "fake-model".into()
    }
}

fn baseline() -> Assessment {
    Assessment { engine: "deterministic".into(), contextual_risk: RiskLevel::Low, stability_risk: RiskLevel::Low, matched_rule: None, rationale: vec!["base".into()], action: PolicyAction::RequireApproval, matched_decision: None }
}

#[test]
fn the_ai_only_changes_the_contextual_risk_and_failures_fall_back() {
    let def: PolicyDef = serde_json::from_value(json!({ "ai_mode_enabled": true, "decisions": [{ "conditions": { "contextual_risk": "Critical" }, "action": "AUTO_DEPLOY" }], "default_action": "REQUIRE_APPROVAL" })).unwrap();
    let ok = Fake(Ok(Verdict { contextual_risk: RiskLevel::Critical, rationale: vec!["exploited in the wild on an exposed host".into()], confidence: 0.8 }));
    let a = ai::apply(&def, baseline(), Some(&ok), &json!({}));
    assert_eq!((a.engine.as_str(), a.contextual_risk, a.stability_risk, a.action), ("ai", RiskLevel::Critical, RiskLevel::Low, PolicyAction::AutoDeploy), "the decision table still decides, on the AI's risk");
    assert!(a.rationale[0].contains("fake-model") && a.rationale[0].contains("baseline Low"), "{:?}", a.rationale);

    // lowering is limited to one level below the rules
    let mut high = baseline();
    high.contextual_risk = RiskLevel::Critical;
    let optimistic = Fake(Ok(Verdict { contextual_risk: RiskLevel::Low, rationale: vec!["not exposed".into()], confidence: 0.6 }));
    let c = ai::apply(&def, high, Some(&optimistic), &json!({}));
    assert_eq!(c.contextual_risk, RiskLevel::High, "Critical -> Low is refused; High at most");
    assert!(c.rationale.iter().any(|r| r.contains("one level at most")), "{:?}", c.rationale);

    let down = Fake(Err("connection refused".into()));
    let b = ai::apply(&def, baseline(), Some(&down), &json!({}));
    assert_eq!((b.engine.as_str(), b.contextual_risk, b.action), ("deterministic", RiskLevel::Low, PolicyAction::RequireApproval));
    assert!(b.rationale[0].contains("AI analysis failed") && b.rationale[0].contains("connection refused"));

    let none = ai::apply(&def, baseline(), None, &json!({}));
    assert!(none.rationale[0].contains("no AI provider is configured"));
}

#[test]
fn a_policy_in_ai_mode_plans_with_the_advisor() {
    let db = support::TestDb::new();
    let mut c = db.conn();
    let host: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("web-01"), hosts::status.eq("online"), hosts::public_key_pem.eq("pem"))).returning(hosts::id).get_result(&mut c).unwrap();
    diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(host), host_packages::name.eq("openssl"), host_packages::version.eq("3.0.1"), host_packages::candidate.eq(Some("3.0.2")), host_packages::is_security.eq(true))).execute(&mut c).unwrap();
    let def = json!({ "ai_mode_enabled": true, "decisions": [{ "conditions": { "contextual_risk": "Critical" }, "action": "AUTO_DEPLOY" }] });
    diesel::insert_into(policies::table).values((policies::name.eq("ai"), policies::definition.eq(def))).execute(&mut c).unwrap();
    // A policy without groups applies to nothing; attach it to every host through a group.
    let g: Uuid = diesel::insert_into(croustypatch_server::schema::host_groups::table).values(croustypatch_server::schema::host_groups::name.eq("all")).returning(croustypatch_server::schema::host_groups::id).get_result(&mut c).unwrap();
    diesel::insert_into(croustypatch_server::schema::host_group_members::table).values((croustypatch_server::schema::host_group_members::host_id.eq(host), croustypatch_server::schema::host_group_members::group_id.eq(g))).execute(&mut c).unwrap();
    let pid: Uuid = policies::table.select(policies::id).first(&mut c).unwrap();
    diesel::insert_into(croustypatch_server::schema::policy_groups::table).values((croustypatch_server::schema::policy_groups::policy_id.eq(pid), croustypatch_server::schema::policy_groups::group_id.eq(g))).execute(&mut c).unwrap();

    let adv = Fake(Ok(Verdict { contextual_risk: RiskLevel::Critical, rationale: vec!["r".into()], confidence: 0.9 }));
    let id = planner::plan_for_host_with(&mut c, host, chrono::Utc::now(), Some(&adv)).unwrap().expect("a task");
    let (status, decision): (String, Option<Value>) = tasks::table.find(id).select((tasks::status, tasks::decision)).first(&mut c).unwrap();
    assert_eq!(status, "queued", "Critical per the AI -> AUTO_DEPLOY per the table");
    assert_eq!(decision.unwrap()["assessment"]["engine"], json!("ai"));
}

/// Minimal OpenAI-compatible endpoint answering `content` to every chat completion; returns the request bodies.
async fn fake_llm(content: &'static str) -> (String, tokio::sync::mpsc::UnboundedReceiver<Value>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let tx = tx.clone();
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
                            let _ = tx.send(serde_json::from_slice(&buf[p + 4..p + 4 + len]).unwrap_or(Value::Null));
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
async fn the_provider_test_makes_a_real_round_trip() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let (url, mut rx) = fake_llm("{\"contextual_risk\": \"Critical\", \"rationale\": [\"remote code execution, exploited\"], \"confidence\": 0.9}").await;
    let (st, _) = call(&app, TestRequest::put().uri("/api/v1/settings/ai").cookie(ck.clone()).set_json(json!({ "enabled": true, "provider": "openai_compatible", "base_url": url, "model": "qwen2.5:3b", "api_key": "" }))).await;
    assert!(st.is_success(), "{st}");
    let (st, r) = call(&app, TestRequest::post().uri("/api/v1/ai/test").cookie(ck.clone())).await;
    assert_eq!(st, 200);
    assert_eq!((r["ok"].clone(), r["contextual_risk"].clone()), (json!(true), json!("Critical")), "{r}");
    let sent = rx.recv().await.unwrap();
    assert_eq!(sent["model"], json!("qwen2.5:3b"));
    assert_eq!(sent["temperature"], json!(0));
    assert_eq!(sent["messages"][0]["role"], json!("system"));
}

#[actix_web::test]
async fn mcp_runs_as_the_token_owner_and_patch_requests_wait_for_approval() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let ck = admin(&app).await;
    let host: Uuid = {
        let mut c = db.conn();
        let h: Uuid = diesel::insert_into(hosts::table).values((hosts::hostname.eq("web-01"), hosts::status.eq("online"), hosts::public_key_pem.eq("pem"))).returning(hosts::id).get_result(&mut c).unwrap();
        diesel::insert_into(host_packages::table).values((host_packages::host_id.eq(h), host_packages::name.eq("openssl"), host_packages::version.eq("3.0.1"), host_packages::candidate.eq(Some("3.0.2")), host_packages::is_security.eq(true))).execute(&mut c).unwrap();
        h
    };
    let (st, tok) = call(&app, TestRequest::post().uri("/api/v1/auth/tokens").cookie(ck.clone()).set_json(json!({ "name": "claude desktop", "days": 30 }))).await;
    assert_eq!(st, 201);
    let token = tok["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("cpat_"));
    let (_, list) = call(&app, TestRequest::get().uri("/api/v1/auth/tokens").cookie(ck.clone())).await;
    assert!(!list.to_string().contains(&token), "the token is shown once, never listed");

    let rpc = |body: Value| TestRequest::post().uri("/api/v1/mcp").insert_header(("authorization", format!("Bearer {token}"))).set_json(body);
    let (st, init) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }))).await;
    assert_eq!(st, 200);
    assert_eq!(init["result"]["serverInfo"]["name"], json!("croustypatch"));
    let (st, _) = call(&app, rpc(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))).await;
    assert_eq!(st, 202, "notifications get no body");
    let (_, tl) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))).await;
    let names: Vec<&str> = tl["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"list_hosts") && names.contains(&"request_patch"));

    let (_, hl) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_hosts", "arguments": {} } }))).await;
    assert_eq!(hl["result"]["structuredContent"][0]["security_updates"], json!(1), "{hl}");

    let (_, pr) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "request_patch", "arguments": { "host": "web-01", "reason": "CVE fix" } } }))).await;
    assert_eq!(pr["result"]["isError"], json!(false), "{pr}");
    let (status, by): (String, String) = tasks::table.filter(tasks::host_id.eq(host)).select((tasks::status, tasks::requested_by)).first(&mut db.conn()).unwrap();
    assert_eq!((status.as_str(), by.as_str()), ("pending_approval", "mcp:admin"), "an assistant never installs anything by itself");

    let (_, again) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "request_patch", "arguments": { "host": "web-01", "reason": "again" } } }))).await;
    assert_eq!(again["result"]["isError"], json!(true), "errors come back as tool errors: {again}");
    let (_, unknown) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 6, "method": "nope" }))).await;
    assert_eq!(unknown["error"]["code"], json!(-32601));

    // revoked: rejected
    let id = list[0]["id"].as_str().unwrap();
    let (st, _) = call(&app, TestRequest::delete().uri(&format!("/api/v1/auth/tokens/{id}")).cookie(ck.clone())).await;
    assert_eq!(st, 204);
    let (st, _) = call(&app, rpc(json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }))).await;
    assert_eq!(st, 401);
    let (st, _) = call(&app, TestRequest::post().uri("/api/v1/mcp").insert_header(("authorization", "Bearer cpat_forged")).set_json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))).await;
    assert_eq!(st, 401);
}
