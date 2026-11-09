//! HTTP-level helpers shared by the API integration tests.
#![allow(dead_code)]
use super::TestDb;
use actix_web::cookie::Cookie;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::StatusCode;
use actix_web::test::{self, TestRequest};
use actix_web::{web, App};
use croustypatch_server::{api, auth};
use serde_json::{json, Value};

pub const ADMIN_PW: &str = "bootstrap-password-1";
pub const ADMIN_NEW_PW: &str = "an-admin-password-2";

pub async fn app(db: &TestDb) -> impl Service<actix_http::Request, Response = ServiceResponse, Error = actix_web::Error> {
    app_with(db, db.state()).await
}

pub async fn app_with(db: &TestDb, state: croustypatch_server::state::AppState) -> impl Service<actix_http::Request, Response = ServiceResponse, Error = actix_web::Error> {
    {
        let mut c = db.conn();
        auth::ensure_roles(&mut c).unwrap();
        auth::ensure_fallback_admin(&mut c, Some(ADMIN_PW.into())).unwrap();
    }
    test::init_service(App::new().app_data(web::Data::new(state)).app_data(web::JsonConfig::default().limit(512 * 1024)).configure(api::configure)).await
}

pub fn session(resp: &ServiceResponse) -> Cookie<'static> {
    resp.response().cookies().find(|c| c.name() == auth::COOKIE).expect("session cookie").into_owned()
}

pub async fn login<S>(app: &S, user: &str, pw: &str) -> ServiceResponse
where
    S: Service<actix_http::Request, Response = ServiceResponse, Error = actix_web::Error>,
{
    test::call_service(app, TestRequest::post().uri("/api/v1/auth/login").set_json(json!({"username": user, "password": pw})).to_request()).await
}

/// Log in as the bootstrap admin and complete the forced password change.
pub async fn admin<S>(app: &S) -> Cookie<'static>
where
    S: Service<actix_http::Request, Response = ServiceResponse, Error = actix_web::Error>,
{
    let first = session(&login(app, "admin", ADMIN_PW).await);
    let r = test::call_service(app, TestRequest::post().uri("/api/v1/auth/password").cookie(first).set_json(json!({"current_password": ADMIN_PW, "new_password": ADMIN_NEW_PW})).to_request()).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    session(&login(app, "admin", ADMIN_NEW_PW).await)
}

pub async fn call<S>(app: &S, req: TestRequest) -> (StatusCode, Value)
where
    S: Service<actix_http::Request, Response = ServiceResponse, Error = actix_web::Error>,
{
    let resp = test::call_service(app, req.to_request()).await;
    let status = resp.status();
    let body = test::read_body(resp).await;
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

