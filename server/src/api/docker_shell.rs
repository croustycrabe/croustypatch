//! Docker migration console: prebuilt command lines for a container (`GET /docker/migration`) and a run of the
//! steps a person reviewed (`POST /docker/shell`). The steps run as root on the host, so running them needs the
//! `docker:shell` permission, which no built-in role except super_admin has; the exact commands are recorded in the
//! audit log as sent.
use crate::audit::{self, Event};
use crate::auth::AuthUser;
use crate::docker_migration::{self, Ctx};
use crate::error::{ApiError, ApiResult};
use crate::planner::OPEN;
use crate::schema::{host_packages, hosts, task_events, tasks};
use crate::state::AppState;
use actix_web::{web, HttpResponse};
use croustypatch_common::protocol::{capability, check_shell_steps, ShellStep};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub const PERMISSION: &str = "docker:shell";

fn docker_on(modules: &Value) -> bool {
    modules.get("docker").and_then(|v| v.as_bool()).unwrap_or(false)
}

#[derive(Deserialize)]
pub struct MigrationQuery {
    host_id: Uuid,
    container: String,
    /// The tag to move to; default: the version the registry suggested.
    to: Option<String>,
}

/// GET /docker/migration — the templates for one container, and whether this user, this host and this server can run them.
pub async fn migration(state: web::Data<AppState>, user: AuthUser, q: web::Query<MigrationQuery>) -> ApiResult<HttpResponse> {
    user.require("hosts:read")?;
    let q = q.into_inner();
    let name = q.container.trim_start_matches("docker:").to_string();
    if !docker_migration::valid_container(&name) {
        return Err(ApiError::bad("not a container name"));
    }
    let can_run = user.can(PERMISSION);
    let st = state.clone();
    let out = state
        .db(move |c| {
            let (facts, modules): (Value, Value) = hosts::table.find(q.host_id).select((hosts::facts, hosts::modules)).first(c).map_err(|_| ApiError::NotFound("host"))?;
            let (version, meta): (String, Value) = host_packages::table
                .filter(host_packages::host_id.eq(q.host_id))
                .filter(host_packages::source.eq("docker"))
                .filter(host_packages::name.eq(format!("docker:{name}")))
                .select((host_packages::version, host_packages::meta))
                .first(c)
                .map_err(|_| ApiError::NotFound("container"))?;
            let image = meta.get("image").and_then(|v| v.as_str()).map(str::to_owned).unwrap_or_else(|| version.split(" @").next().unwrap_or("").to_string());
            let suggested = meta.pointer("/latest/tag").and_then(|t| t.as_str()).map(str::to_owned);
            let to = q.to.clone().filter(|t| !t.trim().is_empty()).or(suggested.clone()).unwrap_or_default();
            let ai = crate::api::admin::load_setting(c, &st, "ai").ok().and_then(|v| crate::ai::AiConfig::from_setting(&v)).is_some();
            let mut out = json!({
                "container": name, "image": image, "suggested": suggested, "to": to, "family": docker_migration::family(&image),
                "can_run": can_run, "docker_module": docker_on(&modules), "agent_ready": crate::modules::has_capability(&facts, capability::DOCKER_SHELL),
                "ai": { "available": ai }, "templates": [],
            });
            if !docker_migration::valid_image(&image) {
                out["problem"] = json!("the image name has characters that cannot be put in a command: do the migration by hand");
            } else if to.is_empty() {
                out["problem"] = json!("no new version is suggested for this image: enter the tag to move to");
            } else if !docker_migration::valid_tag(&to) {
                out["problem"] = json!("not a valid tag");
            } else {
                let compose = meta.get("compose");
                let ctx = Ctx {
                    container: name.clone(),
                    image: image.clone(),
                    to_tag: to,
                    project: meta.get("project").and_then(|v| v.as_str()).map(str::to_owned),
                    service: meta.get("service").and_then(|v| v.as_str()).map(str::to_owned),
                    compose_dir: compose.and_then(|c| c.get("dir")).and_then(|v| v.as_str()).map(str::to_owned),
                    compose_files: compose.and_then(|c| c.get("files")).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                };
                out["templates"] = serde_json::to_value(docker_migration::templates(&ctx)).unwrap_or_default();
            }
            Ok(out)
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Deserialize)]
pub struct ShellBody {
    host_id: Uuid,
    container: String,
    steps: Vec<ShellStep>,
    /// `manual` (templates and edits) or `ai` (a draft that was reviewed).
    #[serde(default)]
    source: String,
}

/// POST /docker/shell — queue a run of the steps on the host.
pub async fn run(state: web::Data<AppState>, user: AuthUser, body: web::Json<ShellBody>) -> ApiResult<HttpResponse> {
    user.require(PERMISSION)?;
    let b = body.into_inner();
    let name = b.container.trim_start_matches("docker:").to_string();
    if !docker_migration::valid_container(&name) {
        return Err(ApiError::bad("not a container name"));
    }
    if !matches!(b.source.as_str(), "" | "manual" | "ai") {
        return Err(ApiError::bad("source must be manual or ai"));
    }
    check_shell_steps(&b.steps).map_err(ApiError::bad)?;
    let (actor, ip) = (user.username.clone(), user.ip.clone());
    let host = b.host_id;
    let id: Uuid = state
        .db(move |c| {
            c.transaction::<_, ApiError, _>(|c| {
                let (status, facts, modules): (String, Value, Value) = hosts::table.find(host).select((hosts::status, hosts::facts, hosts::modules)).first(c).map_err(|_| ApiError::NotFound("host"))?;
                if matches!(status.as_str(), "quarantined" | "pending") {
                    return Err(ApiError::Conflict(format!("the host is {status}")));
                }
                if !docker_on(&modules) {
                    return Err(ApiError::Conflict("the Docker module is switched off for this host".into()));
                }
                if !crate::modules::has_capability(&facts, capability::DOCKER_SHELL) {
                    return Err(ApiError::Conflict("this host's agent is too old for the migration console: update the agent first".into()));
                }
                let known: i64 = host_packages::table.filter(host_packages::host_id.eq(host)).filter(host_packages::source.eq("docker")).filter(host_packages::name.eq(format!("docker:{name}"))).count().get_result(c)?;
                if known == 0 {
                    return Err(ApiError::NotFound("container"));
                }
                let open: i64 = tasks::table.filter(tasks::host_id.eq(host)).filter(tasks::kind.eq_any(["patch", "docker", "docker_shell", "rollback"])).filter(tasks::status.eq_any(OPEN)).count().get_result(c)?;
                if open > 0 {
                    return Err(ApiError::Conflict("an update, patch or migration is already running on this host".into()));
                }
                let exec = json!({ "container": format!("docker:{name}"), "steps": b.steps, "source": if b.source.is_empty() { "manual" } else { b.source.as_str() } });
                let msg = format!("migration console for {name}: {} step(s), requested by {actor}", b.steps.len());
                let id: Uuid = diesel::insert_into(tasks::table)
                    .values((tasks::host_id.eq(host), tasks::kind.eq("docker_shell"), tasks::status.eq("queued"), tasks::requested_by.eq(&actor), tasks::message.eq(&msg), tasks::exec.eq(Some(exec.clone()))))
                    .returning(tasks::id)
                    .get_result(c)?;
                diesel::insert_into(task_events::table).values((task_events::task_id.eq(id), task_events::status.eq("queued"), task_events::message.eq(&msg))).execute(c)?;
                // the exact commands, as sent: root commands are always traceable
                audit::record(c, Event::new(&actor, "docker.shell_requested").target("task", &id.to_string()).host(host).ip(&ip).details(exec))?;
                Ok(id)
            })
        })
        .await?;
    crate::gateway::push_pending(&state, host).await;
    Ok(HttpResponse::Accepted().json(json!({ "task_id": id })))
}
