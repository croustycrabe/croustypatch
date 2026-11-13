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

/// Everything known about one container for the migration pages.
struct Loaded {
    image: String,
    suggested: Option<String>,
    to: String,
    facts: Value,
    modules: Value,
    /// The templates' context, or why there is none.
    ctx: Result<Ctx, &'static str>,
}

fn load(c: &mut PgConnection, host: Uuid, name: &str, to: Option<String>) -> ApiResult<Loaded> {
    let (facts, modules): (Value, Value) = hosts::table.find(host).select((hosts::facts, hosts::modules)).first(c).map_err(|_| ApiError::NotFound("host"))?;
    let (version, meta): (String, Value) = host_packages::table
        .filter(host_packages::host_id.eq(host))
        .filter(host_packages::source.eq("docker"))
        .filter(host_packages::name.eq(format!("docker:{name}")))
        .select((host_packages::version, host_packages::meta))
        .first(c)
        .map_err(|_| ApiError::NotFound("container"))?;
    let image = meta.get("image").and_then(|v| v.as_str()).map(str::to_owned).unwrap_or_else(|| version.split(" @").next().unwrap_or("").to_string());
    let suggested = meta.pointer("/latest/tag").and_then(|t| t.as_str()).map(str::to_owned);
    let to = to.filter(|t| !t.trim().is_empty()).or(suggested.clone()).unwrap_or_default();
    let ctx = if !docker_migration::valid_image(&image) {
        Err("the image name has characters that cannot be put in a command: do the migration by hand")
    } else if to.is_empty() {
        Err("no new version is suggested for this image: enter the tag to move to")
    } else if !docker_migration::valid_tag(&to) {
        Err("not a valid tag")
    } else {
        let compose = meta.get("compose");
        Ok(Ctx {
            container: name.to_string(),
            image: image.clone(),
            to_tag: to.clone(),
            project: meta.get("project").and_then(|v| v.as_str()).map(str::to_owned),
            service: meta.get("service").and_then(|v| v.as_str()).map(str::to_owned),
            compose_dir: compose.and_then(|c| c.get("dir")).and_then(|v| v.as_str()).map(str::to_owned),
            compose_files: compose.and_then(|c| c.get("files")).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
        })
    };
    Ok(Loaded { image, suggested, to, facts, modules, ctx })
}

fn ai_config(c: &mut PgConnection, st: &AppState) -> Option<crate::ai::AiConfig> {
    crate::api::admin::load_setting(c, st, "ai").ok().and_then(|v| crate::ai::AiConfig::from_setting(&v))
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
            let l = load(c, q.host_id, &name, q.to)?;
            let ai = ai_config(c, &st).is_some();
            let mut out = json!({
                "container": name, "image": l.image, "suggested": l.suggested, "to": l.to, "family": docker_migration::family(&l.image),
                "can_run": can_run, "docker_module": docker_on(&l.modules), "agent_ready": crate::modules::has_capability(&l.facts, capability::DOCKER_SHELL),
                "ai": { "available": ai }, "templates": [],
            });
            match &l.ctx {
                Ok(ctx) => out["templates"] = serde_json::to_value(docker_migration::templates(ctx)).unwrap_or_default(),
                Err(why) => out["problem"] = json!(why),
            }
            Ok(out)
        })
        .await?;
    Ok(HttpResponse::Ok().json(out))
}

/// POST /docker/migration-plan — a draft of the steps written by the AI provider, from the container's names, tags and
/// compose paths (no environment values, no file contents). It is only a draft: nothing runs until a person has read it
/// and sent it to `POST /docker/shell`.
pub async fn plan(state: web::Data<AppState>, user: AuthUser, body: web::Json<MigrationQuery>) -> ApiResult<HttpResponse> {
    user.require(PERMISSION)?;
    let q = body.into_inner();
    let name = q.container.trim_start_matches("docker:").to_string();
    if !docker_migration::valid_container(&name) {
        return Err(ApiError::bad("not a container name"));
    }
    let st = state.clone();
    let (ctx, cfg) = state
        .db(move |c| {
            let l = load(c, q.host_id, &name, q.to)?;
            let cfg = ai_config(c, &st).ok_or_else(|| ApiError::Conflict("no AI provider is configured or it is switched off: Settings > AI".into()))?;
            let ctx = l.ctx.map_err(|why| ApiError::bad(why))?;
            audit::record(c, Event::new(&user.username, "docker.migration_plan_requested").target("container", &format!("docker:{name}")).host(q.host_id).ip(&user.ip).details(json!({ "image": ctx.image, "to": ctx.to_tag, "model": cfg.model, "sent": ["container, service and project names", "image and tags", "compose file paths", "the baseline steps"] })))?;
            Ok((ctx, cfg))
        })
        .await?;
    let baseline = docker_migration::templates(&ctx).into_iter().next().map(|t| t.steps).unwrap_or_default();
    let text = crate::ai::complete(&cfg, docker_migration::AI_SYSTEM, &docker_migration::ai_prompt(&ctx, &baseline)).await.map_err(|e| ApiError::Conflict(format!("the AI provider did not answer: {e:#}")))?;
    let plan = docker_migration::parse_plan(&text).map_err(|e| ApiError::Conflict(format!("the AI did not return a usable plan: {e}")))?;
    let steps: Vec<Value> = plan.steps.iter().map(|s| json!({ "title": s.title, "command": s.command, "warnings": docker_migration::warnings(&s.command) })).collect();
    Ok(HttpResponse::Ok().json(json!({ "summary": plan.summary, "risks": plan.risks, "steps": steps, "source": "ai", "model": cfg.model })))
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
