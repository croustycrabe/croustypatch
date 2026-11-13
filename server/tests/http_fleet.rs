//! Integration: "Add agent" distribution endpoints, the fleet-wide package view and dashboard aggregates.
mod support;

use actix_web::http::StatusCode;
use actix_web::test::{self, TestRequest};
use croustypatch_common::crypto::sha256_hex;
use croustypatch_server::schema::{host_packages, hosts};
use diesel::prelude::*;
use serde_json::json;
use support::http::*;
use uuid::Uuid;

fn add_host(c: &mut PgConnection, name: &str, distro: &str) -> Uuid {
    diesel::insert_into(hosts::table).values((hosts::hostname.eq(name), hosts::distro.eq(distro), hosts::public_key_pem.eq("pem"), hosts::status.eq("online"))).returning(hosts::id).get_result(c).unwrap()
}

fn add_pkg(c: &mut PgConnection, host: Uuid, name: &str, version: &str, candidate: Option<&str>, security: bool) {
    diesel::insert_into(host_packages::table)
        .values((host_packages::host_id.eq(host), host_packages::name.eq(name), host_packages::version.eq(version), host_packages::candidate.eq(candidate), host_packages::is_security.eq(security), host_packages::cves.eq(json!(if security { vec!["CVE-2026-0001"] } else { vec![] }))))
        .execute(c)
        .unwrap();
}

#[actix_web::test]
async fn the_installer_script_targets_the_servers_own_public_address() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let get = |host: &str| TestRequest::get().uri("/api/v1/agent/install.sh").insert_header(("host", host.to_string()));

    let resp = test::call_service(&app, get("patch.example.com").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("content-type").unwrap().to_str().unwrap().starts_with("text/x-shellscript"));
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
    let body = String::from_utf8(test::read_body(resp).await.to_vec()).unwrap();
    assert!(body.starts_with("#!/bin/sh"));
    assert!(body.contains("SERVER=\"https://patch.example.com\""), "tls_direct in tests => https");
    assert!(!body.contains("__SERVER__"), "every placeholder replaced");
    assert!(body.contains("CROUSTYPATCH_ENROLL_TOKEN") && body.contains("sha256"), "reads the token from the environment and verifies the binary");
    // only the obvious placeholder `cpt_xxxxxxxx` in the usage comment may appear: never a real token
    assert_eq!(body.matches("cpt_").count(), body.matches("cpt_xxxxxxxx").count(), "the script never embeds a token");

    // HTTP/2 style request: authority only, no Host header
    let h2 = test::call_service(&app, TestRequest::get().uri("https://patch.example.com:8443/api/v1/agent/install.sh").to_request()).await;
    let b2 = String::from_utf8(test::read_body(h2).await.to_vec()).unwrap();
    assert!(b2.contains("SERVER=\"https://patch.example.com:8443\""), "{}", b2.lines().find(|l| l.starts_with("SERVER")).unwrap_or(""));

    // the script is piped into a root shell: a hostile Host header must never reach it
    for evil in ["a.com\";rm -rf /;\"", "x$(id)", "a b", "a`id`", "a;b", "a'b", ""] {
        let r = test::call_service(&app, TestRequest::get().uri("/api/v1/agent/install.sh").insert_header(("host", evil.to_string())).to_request()).await;
        assert!(r.status().is_client_error(), "host {evil:?} must be refused, got {}", r.status());
    }
}

#[actix_web::test]
async fn agent_binaries_are_listed_served_and_checksummed_without_exposing_other_files() {
    let db = support::TestDb::new();
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap(); // searched first, holds nothing: later directories still count
    let binary = b"\x7fELF-pretend-agent-binary".to_vec();
    std::fs::write(dir.path().join("croustypatch-agent-linux-x86_64"), &binary).unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"root:x:0:0").unwrap();
    let state = db.state_with(|c| c.agent_dir = vec![empty.path().to_path_buf(), dir.path().to_path_buf()]);
    let app = app_with(&db, state).await;

    // public, no session needed
    let (st, list) = call(&app, TestRequest::get().uri("/api/v1/agent/platforms")).await;
    assert_eq!(st, StatusCode::OK);
    let by_id = |id: &str| list.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap().clone();
    assert_eq!(by_id("linux-x86_64")["available"], json!(true));
    assert_eq!(by_id("linux-x86_64")["sha256"], json!(sha256_hex(&binary)));
    assert_eq!(by_id("linux-x86_64")["size"], json!(binary.len()));
    assert_eq!(by_id("linux-aarch64")["sha256"], json!(null));

    let resp = test::call_service(&app, TestRequest::get().uri("/api/v1/agent/download/linux-x86_64").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("content-disposition").unwrap().to_str().unwrap().contains("attachment"));
    assert_eq!(test::read_body(resp).await.to_vec(), binary);

    let sum = test::call_service(&app, TestRequest::get().uri("/api/v1/agent/download/linux-x86_64.sha256").to_request()).await;
    let text = String::from_utf8(test::read_body(sum).await.to_vec()).unwrap();
    assert!(text.starts_with(&sha256_hex(&binary)), "{text}");

    // a known platform with no binary, an unknown one, and attempts to leave the directory
    for uri in [
        "/api/v1/agent/download/linux-aarch64",
        "/api/v1/agent/download/linux-aarch64.sha256",
        "/api/v1/agent/download/windows-x86_64",
        "/api/v1/agent/download/secret.txt",
        "/api/v1/agent/download/..%2Fsecret.txt",
        "/api/v1/agent/download/..%2F..%2F..%2Fetc%2Fpasswd",
        "/api/v1/agent/download/linux-x86_64%2F..%2Fsecret.txt",
    ] {
        let r = test::call_service(&app, TestRequest::get().uri(uri).to_request()).await;
        let status = r.status();
        let body = test::read_body(r).await;
        assert!(status.is_client_error(), "{uri} -> {status}");
        assert!(!String::from_utf8_lossy(&body).contains("root:"), "{uri} leaked a file");
    }
}

#[actix_web::test]
async fn packages_are_aggregated_across_the_fleet() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    {
        let mut c = db.conn();
        let (a, b, d) = (add_host(&mut c, "web-01", "Debian"), add_host(&mut c, "web-02", "Debian"), add_host(&mut c, "db-01", "Ubuntu"));
        add_pkg(&mut c, a, "libssl3", "3.0.1", Some("3.0.2"), true);
        add_pkg(&mut c, b, "libssl3", "3.0.1", Some("3.0.2"), true);
        add_pkg(&mut c, d, "libssl3", "3.0.2", None, false);
        add_pkg(&mut c, a, "bash", "5.2", None, false);
        add_pkg(&mut c, d, "curl", "8.0", Some("8.1"), false);
        add_pkg(&mut c, a, "my_pkg%x", "1", None, false);
    }
    let (st, rows) = call(&app, TestRequest::get().uri("/api/v1/packages").cookie(admin.clone())).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!((rows["total"].clone(), rows["page"].clone(), rows["per_page"].clone()), (json!(4), json!(1), json!(50)), "{rows}");
    let rows = rows["items"].as_array().unwrap();
    let find = |n: &str| rows.iter().find(|r| r["name"] == n).unwrap().clone();
    let ssl = find("libssl3");
    assert_eq!((ssl["hosts"].clone(), ssl["outdated"].clone(), ssl["security"].clone(), ssl["latest"].clone()), (json!(3), json!(2), json!(true), json!("3.0.2")));
    let curl = find("curl");
    assert_eq!((curl["hosts"].clone(), curl["outdated"].clone(), curl["security"].clone()), (json!(1), json!(1), json!(false)));
    assert_eq!(find("bash")["outdated"], json!(0));
    assert_eq!(rows[0]["name"], "libssl3", "most outdated, security first");

    let names = |uri: &str| {
        let req = TestRequest::get().uri(uri).cookie(admin.clone());
        let app = &app;
        async move { call(app, req).await.1["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect::<Vec<_>>() }
    };
    assert_eq!(names("/api/v1/packages?updates=true").await, ["libssl3", "curl"], "only packages that need an update");
    assert_eq!(names("/api/v1/packages?q=SSL").await, ["libssl3"], "search is case-insensitive");
    assert_eq!(names("/api/v1/packages?q=%25").await, ["my_pkg%x"], "a literal % matches literally, not as a wildcard");
    assert!(names("/api/v1/packages?q=_").await.len() == 1, "and so does _");

    let (st, hosts) = call(&app, TestRequest::get().uri("/api/v1/packages/libssl3/hosts").cookie(admin.clone())).await;
    assert_eq!(st, StatusCode::OK);
    let hosts = hosts.as_array().unwrap();
    assert_eq!(hosts.len(), 3);
    assert_eq!(hosts[2]["hostname"], "db-01", "hosts that need the update are listed first");
    assert_eq!(hosts[0]["candidate"], "3.0.2");

    let (st, _) = call(&app, TestRequest::get().uri("/api/v1/packages")).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
async fn the_dashboard_summarises_updates_operating_systems_and_top_outdated_packages() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    {
        let mut c = db.conn();
        let (a, b, d) = (add_host(&mut c, "a", "Debian"), add_host(&mut c, "b", "Debian"), add_host(&mut c, "c", "Ubuntu"));
        add_pkg(&mut c, a, "libssl3", "1", Some("2"), true);
        add_pkg(&mut c, b, "libssl3", "1", Some("2"), true);
        add_pkg(&mut c, b, "tzdata", "1", Some("2"), false);
        add_pkg(&mut c, d, "bash", "5", None, false);
    }
    let (st, d) = call(&app, TestRequest::get().uri("/api/v1/dashboard").cookie(admin)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(d["fleet"]["total"], 3);
    assert_eq!(d["fleet"]["hosts_needing_updates"], 2);
    assert_eq!(d["fleet"]["os"], json!([{"name": "Debian", "count": 2}, {"name": "Ubuntu", "count": 1}]));
    assert_eq!(d["vulnerabilities"]["outdated_packages"], 2);
    assert_eq!(d["vulnerabilities"]["hosts_with_security_updates"], 2);
    assert_eq!(d["top_outdated"][0], json!({"name": "libssl3", "hosts": 2, "security": true}));
    assert_eq!(d["top_outdated"][1]["name"], "tzdata");
}

#[actix_web::test]
async fn groups_carry_a_validated_colour_that_can_be_changed() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    let post = |body: serde_json::Value| TestRequest::post().uri("/api/v1/groups").cookie(admin.clone()).set_json(body);

    let (st, g) = call(&app, post(json!({"name": "prod", "color": "teal"}))).await;
    assert_eq!((st, g["color"].clone()), (StatusCode::CREATED, json!("teal")));
    let (st, plain) = call(&app, post(json!({"name": "dev"}))).await;
    assert_eq!((st, plain["color"].clone()), (StatusCode::CREATED, json!("")), "no colour = automatic, decided by the UI from the name");

    // a palette name or a strict #rrggbb: no arbitrary CSS or markup can reach the page through a group
    for bad in ["hotpink", "red; background:url(x)", "<script>", "TEAL", "#ff00", "#gggggg", "#ff0000ff", "ff0000", "#ff0000; x:y", "#ff000é"] {
        let (st, _) = call(&app, post(json!({"name": format!("x-{}", bad.len()), "color": bad}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad:?}");
    }

    let (st, custom) = call(&app, post(json!({"name": "lab", "color": "#A1B2C3"}))).await;
    assert_eq!((st, custom["color"].clone()), (StatusCode::CREATED, json!("#a1b2c3")), "a custom colour is accepted and stored in lower case");

    let id = g["id"].as_str().unwrap();
    let put = |body: serde_json::Value| TestRequest::put().uri(&format!("/api/v1/groups/{id}")).cookie(admin.clone()).set_json(body);
    let (st, g2) = call(&app, put(json!({"name": "prod", "color": "plum"}))).await;
    assert_eq!((st, g2["color"].clone()), (StatusCode::OK, json!("plum")));
    let (st, _) = call(&app, put(json!({"name": "prod", "color": "nope"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, g2b) = call(&app, put(json!({"name": "prod", "color": "#00ff7f"}))).await;
    assert_eq!((st, g2b["color"].clone()), (StatusCode::OK, json!("#00ff7f")));
    let (_, g3) = call(&app, put(json!({"name": "prod", "color": ""}))).await;
    assert_eq!(g3["color"], json!(""), "an empty colour resets to automatic");

    let (_, list) = call(&app, TestRequest::get().uri("/api/v1/groups").cookie(admin.clone())).await;
    assert!(list.as_array().unwrap().iter().all(|g| g.get("color").is_some()), "the list carries colours");
}

#[actix_web::test]
async fn the_packages_list_is_paged_across_the_whole_fleet() {
    let db = support::TestDb::new();
    let app = app(&db).await;
    let admin = admin(&app).await;
    {
        let mut c = db.conn();
        let a = add_host(&mut c, "web-01", "Debian");
        // 130 packages, more than the old 1000-row ceiling would ever have shown per page, in a known order
        for i in 0..130 {
            add_pkg(&mut c, a, &format!("pkg-{i:03}"), "1.0", if i < 3 { Some("2.0") } else { None }, false);
        }
    }
    let get = |q: &str| {
        let req = TestRequest::get().uri(&format!("/api/v1/packages{q}")).cookie(admin.clone());
        let app = &app;
        async move { call(app, req).await }
    };
    let (_, p1) = get("?per_page=25").await;
    assert_eq!((p1["total"].clone(), p1["page"].clone(), p1["per_page"].clone(), p1["items"].as_array().unwrap().len()), (json!(130), json!(1), json!(25), 25));
    let first: Vec<&str> = p1["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(&first[..4], ["pkg-000", "pkg-001", "pkg-002", "pkg-003"], "the outdated ones first, then by name");
    let (_, p2) = get("?per_page=25&page=2").await;
    assert_eq!(p2["items"][0]["name"], json!("pkg-025"), "page 2 continues where page 1 stopped");
    let (_, p6) = get("?per_page=25&page=6").await;
    assert_eq!((p6["items"].as_array().unwrap().len(), p6["total"].clone()), (5, json!(130)), "the last page holds the rest");
    let (_, past) = get("?per_page=25&page=99").await;
    assert_eq!((past["items"].as_array().unwrap().len(), past["total"].clone()), (0, json!(130)), "past the end: no rows, but the total is still known");
    // every package is reachable by paging (no ceiling)
    let mut seen = std::collections::HashSet::new();
    for page in 1..=3 {
        let (_, p) = get(&format!("?per_page=50&page={page}")).await;
        seen.extend(p["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()));
    }
    assert_eq!(seen.len(), 130);
    // sizes are clamped, odd pages do not fail
    let (_, big) = get("?per_page=5000").await;
    assert_eq!((big["per_page"].clone(), big["items"].as_array().unwrap().len()), (json!(100), 100));
    let (st, small) = get("?per_page=0&page=0").await;
    assert!(st.is_success());
    assert_eq!((small["per_page"].clone(), small["page"].clone()), (json!(1), json!(1)));
    // filters count their own total
    let (_, upd) = get("?updates=true&per_page=2").await;
    assert_eq!((upd["total"].clone(), upd["items"].as_array().unwrap().len()), (json!(3), 2));
    let (_, q) = get("?q=pkg-12&per_page=25").await;
    assert_eq!(q["total"], json!(10), "pkg-120 to pkg-129");
}
