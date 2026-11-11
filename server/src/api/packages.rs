//! Fleet-wide package view: which packages are installed where, and which are out of date.
use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::schema::{host_packages, hosts};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(QueryableByName, Serialize)]
struct PackageRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = BigInt)]
    hosts: i64,
    #[diesel(sql_type = BigInt)]
    outdated: i64,
    #[diesel(sql_type = Bool)]
    security: bool,
    #[diesel(sql_type = Nullable<Text>)]
    latest: Option<String>,
}

#[derive(Deserialize)]
pub struct PackageQuery {
    q: Option<String>,
    /// Only packages with at least one host needing an update.
    updates: Option<bool>,
}

pub async fn list_packages(state: web::Data<AppState>, user: AuthUser, q: web::Query<PackageQuery>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let q = q.into_inner();
    let pattern = format!("%{}%", q.q.as_deref().unwrap_or("").trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let only = q.updates.unwrap_or(false);
    let rows: Vec<PackageRow> = state
        .db(move |c| {
            Ok(diesel::sql_query(
                "SELECT name, source, COUNT(*)::bigint AS hosts, \
                        (COUNT(*) FILTER (WHERE candidate IS NOT NULL))::bigint AS outdated, \
                        COALESCE(bool_or(is_security) FILTER (WHERE candidate IS NOT NULL), false) AS security, \
                        MAX(candidate) AS latest \
                 FROM host_packages WHERE name ILIKE $1 \
                 GROUP BY name, source \
                 HAVING ($2 = false OR COUNT(*) FILTER (WHERE candidate IS NOT NULL) > 0) \
                 ORDER BY outdated DESC, security DESC, hosts DESC, name ASC LIMIT 1000",
            )
            .bind::<Text, _>(pattern)
            .bind::<Bool, _>(only)
            .load(c)?)
        })
        .await?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(Serialize)]
struct PackageHost {
    host_id: Uuid,
    hostname: String,
    version: String,
    candidate: Option<String>,
    is_security: bool,
    cves: Value,
}

/// Every host carrying this package, with its installed and available version.
pub async fn package_hosts(state: web::Data<AppState>, user: AuthUser, path: web::Path<String>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let name = path.into_inner();
    let rows: Vec<PackageHost> = state
        .db(move |c| {
            let r: Vec<(Uuid, String, String, Option<String>, bool, Value)> = host_packages::table
                .inner_join(hosts::table)
                .filter(host_packages::name.eq(&name))
                .order((host_packages::candidate.is_null().asc(), hosts::hostname.asc()))
                .select((hosts::id, hosts::hostname, host_packages::version, host_packages::candidate, host_packages::is_security, host_packages::cves))
                .load(c)?;
            Ok(r.into_iter().map(|(host_id, hostname, version, candidate, is_security, cves)| PackageHost { host_id, hostname, version, candidate, is_security, cves }).collect())
        })
        .await?;
    Ok(HttpResponse::Ok().json(rows))
}
