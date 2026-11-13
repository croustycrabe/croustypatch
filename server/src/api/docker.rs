//! Docker menu: every container of every host (those whose Docker module is on), and whether a newer image exists.
//! CroustyPatch only updates images: it never starts, stops or removes containers.
use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::planner::OPEN;
use crate::schema::{host_packages, hosts, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct DockerQuery {
    host: Option<Uuid>,
    /// Only containers with a newer image.
    updates: Option<bool>,
    /// Only containers whose image has a newer version tag in its registry.
    newer: Option<bool>,
    q: Option<String>,
}

/// GET /docker
pub async fn list(state: web::Data<AppState>, user: AuthUser, q: web::Query<DockerQuery>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let q = q.into_inner();
    let out = state
        .db(move |c| {
            let mut query = host_packages::table.inner_join(hosts::table).filter(host_packages::source.eq("docker")).into_boxed();
            if let Some(h) = q.host {
                query = query.filter(hosts::id.eq(h));
            }
            if q.updates == Some(true) {
                query = query.filter(host_packages::candidate.is_not_null());
            }
            if let Some(t) = q.q.as_ref().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                // by container name, or by image (the version column starts with the image reference)
                let like = format!("%{}%", t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
                query = query.filter(host_packages::name.ilike(like.clone()).or(host_packages::version.ilike(like)));
            }
            type Row = (Uuid, String, String, String, Option<String>, Value);
            let rows: Vec<Row> = query
                .order((hosts::hostname.asc(), host_packages::name.asc()))
                .select((hosts::id, hosts::hostname, host_packages::name, host_packages::version, host_packages::candidate, host_packages::meta))
                .limit(2000)
                .load(c)?;
            // The containers a queued or running container task is about: its own packages, not every container of the host
            // (one task at a time runs per host, but only the containers in it are being updated).
            let open: Vec<(Uuid, Uuid, String, Value)> = tasks::table.filter(tasks::kind.eq("docker")).filter(tasks::status.eq_any(OPEN)).select((tasks::host_id, tasks::id, tasks::status, tasks::packages)).load(c)?;
            let mut busy: HashMap<(Uuid, String), (Uuid, String)> = HashMap::new();
            for (h, id, st, pk) in open {
                for name in pk.as_array().into_iter().flatten().filter_map(|p| p.get("name").and_then(|n| n.as_str())) {
                    busy.insert((h, name.to_string()), (id, st.clone()));
                }
            }
            let has_newer = |r: &Row| r.5.get("latest").is_some_and(|l| l.get("tag").is_some_and(|t| t.is_string()));
            let with_newer = rows.iter().filter(|r| has_newer(r)).count();
            let rows: Vec<Row> = if q.newer == Some(true) { rows.into_iter().filter(|r| has_newer(r)).collect() } else { rows };
            let with_updates = rows.iter().filter(|r| r.4.is_some()).count();
            let hosts_n = rows.iter().map(|r| r.0).collect::<std::collections::HashSet<_>>().len();
            let containers: Vec<Value> = rows
                .into_iter()
                .map(|(host_id, hostname, name, version, candidate, meta)| {
                    let image = meta.get("image").and_then(|v| v.as_str()).map(str::to_owned).unwrap_or_else(|| version.split(" @").next().unwrap_or("").to_string());
                    json!({
                        "host_id": host_id, "hostname": hostname, "name": name.trim_start_matches("docker:"), "package": name,
                        "image": image, "current": version.split(" @").nth(1).unwrap_or(""), "candidate": candidate.as_deref().map(|c| c.split(" @").nth(1).unwrap_or("")),
                        "update": candidate.is_some(),
                        // what the agent's registry checks found, and why there is no suggestion when there is none (agents 0.10+)
                        "check": meta.get("check").filter(|c| c.is_object()).cloned(),
                        "latest": meta.get("latest").filter(|l| l.get("tag").is_some_and(|t| t.is_string())).map(|l| json!({
                            "tag": l.get("tag"), "major": l.get("major").and_then(|m| m.as_bool()).unwrap_or(false), "same_major": l.get("same_major").filter(|x| x.is_string()),
                            // whether the agent can apply it, and by changing which file (agents 0.7.4 and later)
                            "apply": l.get("apply").filter(|a| a.get("mode").is_some_and(|m| m == "file" || m == "manual")),
                        })),
                        "project": meta.get("project").cloned().unwrap_or(Value::Null), "service": meta.get("service").cloned().unwrap_or(Value::Null),
                        "state": meta.get("state").cloned().unwrap_or(Value::Null), "health": meta.get("health").cloned().unwrap_or(Value::Null), "started_at": meta.get("started_at").cloned().unwrap_or(Value::Null),
                        "open_task": busy.get(&(host_id, name.clone())).map(|(id, st)| json!({ "id": id, "status": st })),
                    })
                })
                .collect();
            Ok(json!({ "summary": { "containers": containers.len(), "with_updates": with_updates, "with_newer_version": with_newer, "hosts": hosts_n }, "containers": containers }))
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}
