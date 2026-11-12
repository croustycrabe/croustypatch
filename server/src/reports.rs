//! Reports: a summary of the fleet built from the database when it is sent (not from queued events), on a daily,
//! weekly or monthly schedule, by e-mail or any other channel. It is sent even when nothing happened, unless the
//! report says "skip when there is nothing to report". Every attempt is recorded, with the error when it fails.
use crate::api::compliance_api;
use crate::audit::{self, Event};
use crate::engine::policy::PolicyDef;
use crate::error::{ApiError, ApiResult};
use crate::notify::{self, Channel, Message};
use crate::schema::{host_group_members, host_packages, hosts, policies, policy_groups, report_runs, reports, tasks};
use crate::state::AppState;
use actix_web::web;
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub const FREQUENCIES: &[&str] = &["daily", "weekly", "monthly"];
const MAX_ATTEMPTS: i32 = 3;

#[derive(Queryable, Selectable, Serialize, Clone, Debug)]
#[diesel(table_name = reports)]
pub struct Report {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub frequency: String,
    pub weekday: i32,
    pub day_of_month: i32,
    pub send_at: String,
    pub timezone: String,
    pub language: String,
    pub sections: Value,
    pub group_ids: Value,
    pub emails: Value,
    pub channel_ids: Value,
    pub skip_if_empty: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_slot_at: Option<DateTime<Utc>>,
    pub attempts: i32,
    pub last_status: String,
    pub last_error: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What a report contains.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Sections {
    pub summary: bool,
    /// Hosts with updates waiting, and the packages most behind.
    pub pending: bool,
    /// Patches that were applied (or rolled back): the last N, or the last N days.
    pub history: bool,
    /// "count" or "days".
    pub history_mode: String,
    pub history_value: u32,
    pub failures: bool,
    pub reboots: bool,
    pub compliance: bool,
    /// The next patch day of each policy.
    pub patch_days: bool,
    /// Container images with a newer version.
    pub docker: bool,
}

impl Default for Sections {
    fn default() -> Self {
        Self { summary: true, pending: true, history: true, history_mode: "count".into(), history_value: 10, failures: true, reboots: true, compliance: false, patch_days: true, docker: false }
    }
}

impl Sections {
    pub fn from_value(v: &Value) -> Sections {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.history_mode.as_str(), "count" | "days") {
            return Err("history_mode must be 'count' or 'days'".into());
        }
        let max = if self.history_mode == "count" { 200 } else { 365 };
        if !(1..=max).contains(&self.history_value) {
            return Err(format!("history_value must be 1 to {max}"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------- schedule

fn parse_tz(z: &str) -> Option<Tz> {
    z.trim().parse::<Tz>().ok()
}

/// Local wall-clock time in a zone to UTC (a time skipped by daylight saving moves to just after the gap).
fn local_to_utc(tz: Tz, d: NaiveDate, t: NaiveTime) -> DateTime<Utc> {
    let naive = d.and_time(t);
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(x) => x.with_timezone(&Utc),
        LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        LocalResult::None => tz.from_local_datetime(&(naive + Duration::hours(1))).earliest().map(|x| x.with_timezone(&Utc)).unwrap_or_else(|| Utc.from_utc_datetime(&naive)),
    }
}

/// A repeating moment: daily, weekly (weekday 0 = Monday) or monthly (day of month), at a local time in a zone.
/// Shared by the reports and the planned compliance audits.
pub struct Recurrence<'a> {
    pub frequency: &'a str,
    pub weekday: i32,
    pub day_of_month: i32,
    /// "HH:MM"
    pub at: &'a str,
    pub timezone: &'a str,
}

impl Recurrence<'_> {
    fn slots(&self, now: DateTime<Utc>, forward: bool) -> Option<DateTime<Utc>> {
        let tz = parse_tz(self.timezone)?;
        let at = NaiveTime::parse_from_str(self.at, "%H:%M").ok()?;
        let today = now.with_timezone(&tz).date_naive();
        for step in 0..70 {
            let d = if forward { today + Duration::days(step) } else { today - Duration::days(step) };
            let hit = match self.frequency {
                "daily" => true,
                "weekly" => d.weekday().num_days_from_monday() as i32 == self.weekday,
                "monthly" => d.day() as i32 == self.day_of_month,
                _ => return None,
            };
            if hit {
                let slot = local_to_utc(tz, d, at);
                if (forward && slot > now) || (!forward && slot <= now) {
                    return Some(slot);
                }
            }
        }
        None
    }
    /// The most recent moment at or before `now` (None when the recurrence is off or invalid).
    pub fn latest(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.slots(now, false)
    }
    /// The next moment after `now`.
    pub fn next(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.slots(now, true)
    }
}

fn recurrence(r: &Report) -> Recurrence<'_> {
    Recurrence { frequency: &r.frequency, weekday: r.weekday, day_of_month: r.day_of_month, at: &r.send_at, timezone: &r.timezone }
}

/// The most recent scheduled moment at or before `now`.
pub fn latest_slot(r: &Report, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    recurrence(r).latest(now)
}

/// The next scheduled moment after `now` (for the UI).
pub fn next_slot(r: &Report, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    recurrence(r).next(now)
}

/// Should this report be sent now? A new scheduled moment, or a failed send that is due for another try.
pub fn is_due(r: &Report, now: DateTime<Utc>) -> bool {
    if !r.enabled {
        return false;
    }
    let Some(slot) = latest_slot(r, now) else { return false };
    // A report never fires for a moment before it existed: creating it at noon does not send last night's slot.
    if slot < r.created_at {
        return false;
    }
    match r.last_slot_at {
        Some(done) if slot <= done => r.last_status == "failed" && slot == done && r.attempts < MAX_ATTEMPTS && r.last_run_at.is_some_and(|l| now - l >= Duration::minutes(10 * r.attempts.max(1) as i64)),
        _ => true,
    }
}

// ------------------------------------------------------------------------------------------- the document

enum Block {
    Heading(String),
    Para(String),
    Stats(Vec<(String, String)>),
    Table { head: Vec<String>, rows: Vec<Vec<String>> },
}

pub struct Built {
    pub title: String,
    pub text: String,
    pub html: String,
    /// Nothing worth reading: no update waiting, no patch, no failure, no reboot needed.
    pub empty: bool,
}

struct Tx(bool);
impl Tx {
    fn t(&self, k: &str) -> String {
        let (fr, en) = match k {
            "summary" => ("Résumé", "Summary"),
            "machines" => ("Machines", "Machines"),
            "online" => ("En ligne", "Online"),
            "offline" => ("Hors ligne", "Offline"),
            "quarantined" => ("En quarantaine", "Quarantined"),
            "need_updates" => ("Machines avec des mises à jour", "Machines with updates"),
            "pending" => ("Mises à jour en attente", "Updates waiting"),
            "security" => ("dont sécurité", "of which security"),
            "reboot" => ("Redémarrage requis", "Reboot required"),
            "period_tasks" => ("Tâches sur la période", "Tasks in the period"),
            "pending_title" => ("Mises à jour en attente par machine", "Updates waiting, by machine"),
            "machine" => ("Machine", "Machine"),
            "total" => ("Total", "Total"),
            "sec" => ("Sécurité", "Security"),
            "examples" => ("Exemples", "Examples"),
            "behind" => ("Paquets les plus en retard", "Packages most behind"),
            "package" => ("Paquet", "Package"),
            "machines_n" => ("Machines", "Machines"),
            "history_title" => ("Historique des correctifs", "Patch history"),
            "when" => ("Quand", "When"),
            "kind" => ("Type", "Type"),
            "result" => ("Résultat", "Result"),
            "detail" => ("Détail", "Detail"),
            "failures_title" => ("Échecs et problèmes", "Failures and problems"),
            "reboots_title" => ("Machines à redémarrer", "Machines to reboot"),
            "compliance_title" => ("Conformité", "Compliance"),
            "score" => ("Score moyen du parc", "Fleet average score"),
            "comp_below" => ("Sous le seuil ({t} %)", "Below the threshold ({t} %)"),
            "comp_period" => ("Audits sur la période", "Audits in the period"),
            "comp_stale" => ("Dernier audit de plus de 30 jours", "Last audit over 30 days old"),
            "comp_never" => ("Jamais auditées", "Never audited"),
            "comp_profile" => ("Profil", "Profile"),
            "comp_score" => ("Score", "Score"),
            "comp_failed" => ("Règles en échec", "Failed rules"),
            "comp_high" => ("dont graves", "of which high"),
            "comp_na" => ("aucune règle applicable", "no rule applies"),
            "comp_next" => ("Prochain audit planifié", "Next planned audit"),
            "patch_days_title" => ("Prochains jours de patch", "Next patch days"),
            "policy" => ("Politique", "Policy"),
            "applies" => ("Application", "Applies"),
            "next" => ("Prochain", "Next"),
            "auto" => ("automatique", "automatic"),
            "approval" => ("après approbation", "after approval"),
            "anytime" => ("dès que possible", "as soon as possible"),
            "docker_title" => ("Images de conteneurs à mettre à jour", "Container images to update"),
            "container" => ("Conteneur", "Container"),
            "current" => ("Actuelle", "Current"),
            "available" => ("Disponible", "Available"),
            "nothing" => ("Rien à signaler.", "Nothing to report."),
            "all_current" => ("Tout est à jour.", "Everything is up to date."),
            "no_patches" => ("Aucun correctif sur la période.", "No patch in the period."),
            "none" => ("aucune", "none"),
            "more" => ("et {n} autre(s)", "and {n} more"),
            "scope" => ("Périmètre", "Scope"),
            "all_hosts" => ("toutes les machines", "all machines"),
            _ => ("?", "?"),
        };
        if self.0 { fr } else { en }.to_string()
    }
    fn kind(&self, k: &str) -> String {
        match (k, self.0) {
            ("docker", true) => "conteneurs",
            ("docker", false) => "containers",
            ("patch", true) => "correctifs",
            ("patch", false) => "patch",
            ("rollback", true) => "retour arrière",
            ("rollback", false) => "rollback",
            ("reboot", true) => "redémarrage",
            ("reboot", false) => "reboot",
            ("app", true) => "application",
            ("app", false) => "app",
            (o, _) => return o.to_string(),
        }
        .to_string()
    }
    fn status(&self, s: &str) -> String {
        match (s, self.0) {
            ("succeeded", true) => "réussi",
            ("succeeded", false) => "succeeded",
            ("failed", true) => "échec",
            ("failed", false) => "failed",
            ("rolled_back", true) => "annulé (rollback)",
            ("rolled_back", false) => "rolled back",
            ("quarantined", true) => "quarantaine",
            ("quarantined", false) => "quarantined",
            ("expired", true) => "manqué ou expiré",
            ("expired", false) => "missed or expired",
            ("cancelled", true) => "annulé",
            ("cancelled", false) => "cancelled",
            (o, _) => return o.to_string(),
        }
        .to_string()
    }
}

fn cut(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= n { s } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

fn fmt_time(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string()
}

/// Build the report as of `now`. `since` is the start of the period failures are listed for.
pub fn build(c: &mut PgConnection, r: &Report, now: DateTime<Utc>) -> ApiResult<Built> {
    let s = Sections::from_value(&r.sections);
    let tx = Tx(r.language != "en");
    let tz = parse_tz(&r.timezone).unwrap_or(chrono_tz::UTC);
    let period = match r.frequency.as_str() {
        "daily" => Duration::hours(24),
        "monthly" => Duration::days(31),
        _ => Duration::days(7),
    };
    let since = now - period;

    let scope: Vec<Uuid> = r.group_ids.as_array().map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| s.parse().ok())).collect()).unwrap_or_default();
    let all: Vec<(Uuid, String, String, bool)> = hosts::table.order(hosts::hostname.asc()).select((hosts::id, hosts::hostname, hosts::status, hosts::reboot_required)).load(c)?;
    let in_scope: HashSet<Uuid> = if scope.is_empty() {
        all.iter().map(|h| h.0).collect()
    } else {
        host_group_members::table.filter(host_group_members::group_id.eq_any(&scope)).select(host_group_members::host_id).load::<Uuid>(c)?.into_iter().collect()
    };
    let hs: Vec<&(Uuid, String, String, bool)> = all.iter().filter(|h| in_scope.contains(&h.0)).collect();
    let names: HashMap<Uuid, &str> = hs.iter().map(|h| (h.0, h.1.as_str())).collect();
    let ids: Vec<Uuid> = hs.iter().map(|h| h.0).collect();

    // pending updates
    let pend: Vec<(Uuid, String, bool, String, String)> = host_packages::table
        .filter(host_packages::host_id.eq_any(&ids))
        .filter(host_packages::candidate.is_not_null())
        .filter(host_packages::source.eq("system"))
        .select((host_packages::host_id, host_packages::name, host_packages::is_security, host_packages::version, host_packages::candidate.assume_not_null()))
        .load(c)?;
    let mut per_host: BTreeMap<Uuid, (usize, usize, Vec<String>)> = BTreeMap::new();
    let mut per_pkg: BTreeMap<String, (usize, bool)> = BTreeMap::new();
    for (h, name, sec, _, _) in &pend {
        let e = per_host.entry(*h).or_default();
        e.0 += 1;
        e.1 += *sec as usize;
        if e.2.len() < 4 {
            e.2.push(name.clone());
        }
        let p = per_pkg.entry(name.clone()).or_default();
        p.0 += 1;
        p.1 |= *sec;
    }
    let total_pending = pend.len();
    let sec_pending = pend.iter().filter(|p| p.2).count();

    // tasks in the period
    type T = (Uuid, String, String, String, String, Option<DateTime<Utc>>, Value);
    let finished: Vec<T> = tasks::table
        .filter(tasks::host_id.eq_any(&ids))
        .filter(tasks::finished_at.is_not_null())
        .filter(tasks::finished_at.ge(since))
        .order(tasks::finished_at.desc())
        .select((tasks::host_id, tasks::kind, tasks::status, tasks::message, tasks::requested_by, tasks::finished_at, tasks::packages))
        .limit(1000)
        .load(c)?;
    let count_status = |st: &str| finished.iter().filter(|t| t.2 == st).count();
    let problems: Vec<&T> = finished.iter().filter(|t| matches!(t.2.as_str(), "failed" | "rolled_back" | "quarantined" | "expired")).collect();

    // patch history (last N patches, or last N days), independent of the report period
    let hist: Vec<T> = if s.history {
        let q = tasks::table
            .filter(tasks::host_id.eq_any(&ids))
            .filter(tasks::kind.eq_any(["patch", "rollback", "docker"]))
            .filter(tasks::finished_at.is_not_null())
            .filter(tasks::status.ne_all(["cancelled"]))
            .order(tasks::finished_at.desc())
            .select((tasks::host_id, tasks::kind, tasks::status, tasks::message, tasks::requested_by, tasks::finished_at, tasks::packages))
            .into_boxed();
        if s.history_mode == "days" { q.filter(tasks::finished_at.ge(now - Duration::days(s.history_value as i64))).limit(200).load(c)? } else { q.limit(s.history_value as i64).load(c)? }
    } else {
        vec![]
    };

    let reboot: Vec<&&(Uuid, String, String, bool)> = hs.iter().filter(|h| h.3).collect();
    let mut blocks: Vec<Block> = vec![];
    let scope_label = if scope.is_empty() { tx.t("all_hosts") } else { format!("{} group(s)", scope.len()) };
    blocks.push(Block::Para(format!("{} : {} · {}", tx.t("scope"), scope_label, fmt_time(now, tz))));

    if s.summary {
        let n = |st: &[&str]| hs.iter().filter(|h| st.contains(&h.2.as_str())).count();
        blocks.push(Block::Heading(tx.t("summary")));
        blocks.push(Block::Stats(vec![
            (tx.t("machines"), hs.len().to_string()),
            (tx.t("online"), n(&["online", "eco", "maintenance"]).to_string()),
            (tx.t("offline"), n(&["offline", "alert"]).to_string()),
            (tx.t("quarantined"), n(&["quarantined"]).to_string()),
            (tx.t("need_updates"), per_host.len().to_string()),
            (tx.t("pending"), format!("{total_pending} ({} {sec_pending})", tx.t("security"))),
            (tx.t("reboot"), reboot.len().to_string()),
            (tx.t("period_tasks"), format!("{} ✔ · {} ✖ · {} ↺", count_status("succeeded"), count_status("failed"), count_status("rolled_back"))),
        ]));
    }
    if s.pending {
        blocks.push(Block::Heading(tx.t("pending_title")));
        if per_host.is_empty() {
            blocks.push(Block::Para(tx.t("all_current")));
        } else {
            let mut rows: Vec<(&Uuid, &(usize, usize, Vec<String>))> = per_host.iter().collect();
            rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(b.1 .0.cmp(&a.1 .0)));
            let shown: Vec<Vec<String>> = rows.iter().take(20).map(|(h, (t, sec, ex))| vec![names.get(*h).copied().unwrap_or("?").to_string(), t.to_string(), sec.to_string(), ex.join(", ")]).collect();
            blocks.push(Block::Table { head: vec![tx.t("machine"), tx.t("total"), tx.t("sec"), tx.t("examples")], rows: shown });
            if rows.len() > 20 {
                blocks.push(Block::Para(tx.t("more").replace("{n}", &(rows.len() - 20).to_string())));
            }
            let mut pk: Vec<(&String, &(usize, bool))> = per_pkg.iter().collect();
            pk.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(b.1 .1.cmp(&a.1 .1)).then(a.0.cmp(b.0)));
            blocks.push(Block::Heading(tx.t("behind")));
            blocks.push(Block::Table { head: vec![tx.t("package"), tx.t("machines_n"), tx.t("sec")], rows: pk.iter().take(10).map(|(n, (c, sec))| vec![n.to_string(), c.to_string(), if *sec { "●".into() } else { String::new() }]).collect() });
        }
    }
    if s.history {
        blocks.push(Block::Heading(tx.t("history_title")));
        if hist.is_empty() {
            blocks.push(Block::Para(tx.t("no_patches")));
        } else {
            blocks.push(Block::Table {
                head: vec![tx.t("when"), tx.t("machine"), tx.t("kind"), tx.t("result"), tx.t("detail")],
                rows: hist.iter().map(|t| vec![t.5.map(|x| fmt_time(x, tz)).unwrap_or_default(), names.get(&t.0).copied().unwrap_or("?").to_string(), tx.kind(&t.1), tx.status(&t.2), cut(&t.3, 90)]).collect(),
            });
        }
    }
    if s.failures {
        blocks.push(Block::Heading(tx.t("failures_title")));
        if problems.is_empty() {
            blocks.push(Block::Para(tx.t("none")));
        } else {
            blocks.push(Block::Table {
                head: vec![tx.t("when"), tx.t("machine"), tx.t("kind"), tx.t("result"), tx.t("detail")],
                rows: problems.iter().take(30).map(|t| vec![t.5.map(|x| fmt_time(x, tz)).unwrap_or_default(), names.get(&t.0).copied().unwrap_or("?").to_string(), tx.kind(&t.1), tx.status(&t.2), cut(&t.3, 110)]).collect(),
            });
        }
    }
    if s.reboots {
        blocks.push(Block::Heading(tx.t("reboots_title")));
        blocks.push(Block::Para(if reboot.is_empty() { tx.t("none") } else { reboot.iter().map(|h| h.1.clone()).collect::<Vec<_>>().join(", ") }));
    }
    if s.compliance {
        blocks.push(Block::Heading(tx.t("compliance_title")));
        let limit = crate::compliance::threshold(c);
        // the latest audit of each machine in scope, worst score first; an audit where no rule applied has no score
        let mut rows: Vec<compliance_api::Row> = compliance_api::latest(c)?.into_iter().filter(|r| in_scope.contains(&r.1)).collect();
        let scored = |r: &compliance_api::Row| r.3 + r.4 > 0;
        rows.sort_by(|a, b| scored(b).cmp(&scored(a)).then(a.6.partial_cmp(&b.6).unwrap_or(std::cmp::Ordering::Equal)));
        let scores: Vec<f64> = rows.iter().filter(|r| scored(r)).map(|r| r.6).collect();
        let below = rows.iter().filter(|r| scored(r) && r.6 < limit).count();
        let in_period = rows.iter().filter(|r| r.9 >= since).count();
        let stale = rows.iter().filter(|r| r.9 < now - Duration::days(30)).count();
        // machines that should be audited (OpenSCAP module on) and never were
        let audited: HashSet<Uuid> = rows.iter().map(|r| r.1).collect();
        let mut never = 0;
        for id in ids.iter().filter(|i| !audited.contains(i)) {
            never += crate::modules::for_host(c, *id).map(|m| m.openscap as usize).unwrap_or(0);
        }
        blocks.push(Block::Stats(vec![
            (tx.t("score"), if scores.is_empty() { tx.t("none") } else { format!("{:.1} %", scores.iter().sum::<f64>() / scores.len() as f64) }),
            (tx.t("comp_below").replace("{t}", &format!("{limit:.0}")), below.to_string()),
            (tx.t("comp_period"), in_period.to_string()),
            (tx.t("comp_stale"), stale.to_string()),
            (tx.t("comp_never"), never.to_string()),
        ]));
        if !rows.is_empty() {
            let high = |r: &compliance_api::Row| r.7.as_array().map(|a| a.iter().filter(|x| x.get("severity").and_then(|s| s.as_str()) == Some("high")).count()).unwrap_or(0);
            blocks.push(Block::Table {
                head: vec![tx.t("machine"), tx.t("comp_profile"), tx.t("comp_score"), tx.t("comp_failed"), tx.t("comp_high"), tx.t("when")],
                rows: rows
                    .iter()
                    .take(40)
                    .map(|r| {
                        let score = if !scored(r) { tx.t("comp_na") } else if r.6 < limit { format!("{:.1} % ▼", r.6) } else { format!("{:.1} %", r.6) };
                        vec![names.get(&r.1).copied().unwrap_or("?").to_string(), cut(&r.2.replace("xccdf_org.ssgproject.content_profile_", ""), 40), score, r.4.to_string(), high(r).to_string(), fmt_time(r.9, tz)]
                    })
                    .collect(),
            });
            if rows.len() > 40 {
                blocks.push(Block::Para(tx.t("more").replace("{n}", &(rows.len() - 40).to_string())));
            }
        }
        if let Some(next) = crate::api::admin::load_setting_plain(c, "compliance").and_then(|v| crate::compliance::planned_next(&v, now)) {
            blocks.push(Block::Para(format!("{} : {}", tx.t("comp_next"), fmt_time(next, tz))));
        }
    }
    if s.patch_days {
        let pols: Vec<(Uuid, String, Value)> = policies::table.filter(policies::enabled.eq(true)).order((policies::priority.asc(), policies::name.asc())).select((policies::id, policies::name, policies::definition)).load(c)?;
        let links: Vec<(Uuid, Uuid)> = policy_groups::table.select((policy_groups::policy_id, policy_groups::group_id)).load(c)?;
        let members: Vec<(Uuid, Uuid)> = host_group_members::table.select((host_group_members::group_id, host_group_members::host_id)).load(c)?;
        let mut rows = vec![];
        for (id, name, def) in pols {
            let Ok(def) = serde_json::from_value::<PolicyDef>(def) else { continue };
            let gs: HashSet<Uuid> = links.iter().filter(|(p, _)| *p == id).map(|(_, g)| *g).collect();
            let n = members.iter().filter(|(g, h)| gs.contains(g) && in_scope.contains(h)).map(|(_, h)| *h).collect::<HashSet<_>>().len();
            if n == 0 {
                continue;
            }
            let mode = match def.default_action {
                croustypatch_common::model::PolicyAction::AutoDeploy => tx.t("auto"),
                _ => tx.t("approval"),
            };
            let next = match &def.maintenance_window {
                Some(w) => w.upcoming(now, 1).first().map(|o| format!("{} → {}", fmt_time(o.start, w_tz(w, tz)), fmt_time(o.end, w_tz(w, tz)).split(' ').last().unwrap_or(""))).unwrap_or_else(|| "?".into()),
                None => tx.t("anytime"),
            };
            rows.push(vec![name, mode, next, n.to_string()]);
        }
        blocks.push(Block::Heading(tx.t("patch_days_title")));
        if rows.is_empty() {
            blocks.push(Block::Para(tx.t("none")));
        } else {
            blocks.push(Block::Table { head: vec![tx.t("policy"), tx.t("applies"), tx.t("next"), tx.t("machines_n")], rows });
        }
    }
    if s.docker {
        let d: Vec<(Uuid, String, String, Option<String>)> = host_packages::table
            .filter(host_packages::host_id.eq_any(&ids))
            .filter(host_packages::source.eq("docker"))
            .filter(host_packages::candidate.is_not_null())
            .order(host_packages::name.asc())
            .select((host_packages::host_id, host_packages::name, host_packages::version, host_packages::candidate))
            .load(c)?;
        blocks.push(Block::Heading(tx.t("docker_title")));
        if d.is_empty() {
            blocks.push(Block::Para(tx.t("all_current")));
        } else {
            blocks.push(Block::Table {
                head: vec![tx.t("machine"), tx.t("container"), tx.t("current"), tx.t("available")],
                rows: d.iter().take(40).map(|(h, n, v, cand)| vec![names.get(h).copied().unwrap_or("?").to_string(), n.trim_start_matches("docker:").to_string(), v.clone(), cand.clone().unwrap_or_default()]).collect(),
            });
        }
    }

    let empty = per_host.is_empty() && finished.is_empty() && hist.is_empty() && reboot.is_empty();
    let title = format!("[CroustyPatch] {} · {}", r.name, now.with_timezone(&tz).format("%Y-%m-%d"));
    Ok(Built { text: render_text(&title, &blocks), html: render_html(&title, &blocks), title, empty })
}

fn w_tz(w: &crate::engine::policy::MaintenanceWindow, fallback: Tz) -> Tz {
    w.timezone.as_deref().and_then(parse_tz).unwrap_or(fallback)
}

// ---------------------------------------------------------------------------------------------- rendering

fn render_text(title: &str, blocks: &[Block]) -> String {
    let mut out = format!("{title}\n{}\n", "=".repeat(title.chars().count().min(78)));
    for b in blocks {
        match b {
            Block::Heading(h) => out += &format!("\n{h}\n{}\n", "-".repeat(h.chars().count().min(78))),
            Block::Para(p) => out += &format!("{p}\n"),
            Block::Stats(s) => {
                let w = s.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
                for (k, v) in s {
                    out += &format!("{k:<w$}  {v}\n");
                }
            }
            Block::Table { head, rows } => {
                let mut widths: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
                for r in rows {
                    for (i, c) in r.iter().enumerate() {
                        widths[i] = widths[i].max(c.chars().count().min(48));
                    }
                }
                let line = |cells: &[String]| cells.iter().enumerate().map(|(i, c)| format!("{:<w$}", cut(c, 48), w = widths[i])).collect::<Vec<_>>().join("  ").trim_end().to_string();
                out += &format!("{}\n", line(head));
                for r in rows {
                    out += &format!("{}\n", line(r));
                }
            }
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn render_html(title: &str, blocks: &[Block]) -> String {
    let mut body = String::new();
    for b in blocks {
        match b {
            Block::Heading(h) => body += &format!("<h2 style=\"font:600 16px Arial,sans-serif;color:#a44334;margin:24px 0 8px\">{}</h2>", esc(h)),
            Block::Para(p) => body += &format!("<p style=\"margin:4px 0;color:#1b2125\">{}</p>", esc(p)),
            Block::Stats(s) => {
                body += "<table role=\"presentation\" style=\"border-collapse:collapse\">";
                for (k, v) in s {
                    body += &format!("<tr><td style=\"padding:3px 16px 3px 0;color:#4d6169\">{}</td><td style=\"padding:3px 0;font-weight:600\">{}</td></tr>", esc(k), esc(v));
                }
                body += "</table>";
            }
            Block::Table { head, rows } => {
                body += "<table style=\"border-collapse:collapse;width:100%;font-size:13px\"><tr>";
                for h in head {
                    body += &format!("<th align=\"left\" style=\"padding:6px 8px;background:#eeeae0;border-bottom:1px solid #d4ccb8\">{}</th>", esc(h));
                }
                body += "</tr>";
                for r in rows {
                    body += "<tr>";
                    for c in r {
                        body += &format!("<td style=\"padding:6px 8px;border-bottom:1px solid #eeeae0;vertical-align:top\">{}</td>", esc(c));
                    }
                    body += "</tr>";
                }
                body += "</table>";
            }
        }
    }
    format!("<!doctype html><html><body style=\"margin:0;background:#f5f2ea\"><div style=\"max-width:720px;margin:0 auto;padding:24px;background:#fcfbf8;font:14px/1.5 Arial,sans-serif;color:#1b2125\"><h1 style=\"font:700 20px Arial,sans-serif;margin:0 0 4px\">{}</h1>{}<p style=\"margin-top:28px;color:#8a979c;font-size:12px\">CroustyPatch</p></div></body></html>", esc(title), body)
}

// --------------------------------------------------------------------------------------------- sending

pub struct Outcome {
    pub status: &'static str,
    pub error: String,
    pub recipients: Vec<String>,
    pub title: String,
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// Build and send one report. `to` overrides the recipients (the "send me a test" button).
pub async fn send(state: &web::Data<AppState>, id: Uuid, trigger: &'static str, to: Option<Vec<String>>) -> ApiResult<Outcome> {
    let now = Utc::now();
    let (r, built) = {
        let st = id;
        state
            .db(move |c| {
                let r: Report = reports::table.find(st).select(Report::as_select()).first(c).map_err(|_| ApiError::NotFound("report"))?;
                let b = build(c, &r, now)?;
                Ok((r, b))
            })
            .await?
    };
    let recipients = to.clone().unwrap_or_else(|| strs(&r.emails));
    let skipped = trigger == "schedule" && r.skip_if_empty && built.empty;
    let channels: Vec<Uuid> = strs(&r.channel_ids).iter().filter_map(|s| s.parse().ok()).collect();
    let mut errors: Vec<String> = vec![];
    let mut sent_any = false;
    if !skipped {
        if channels.is_empty() {
            errors.push("no channel selected: choose an e-mail provider or another channel for this report".into());
        }
        for ch in channels {
            let st = state.clone();
            let loaded = state.db(move |c| Channel::load(c, &st.master, ch).map_err(|e| ApiError::bad(format!("{e:#}")))).await;
            let channel = match loaded {
                Ok(c) => c,
                Err(e) => {
                    errors.push(format!("channel unavailable: {e:?}"));
                    continue;
                }
            };
            if channel.kind == "smtp" && recipients.is_empty() && channel.config.get("to").and_then(|t| t.as_array()).is_none_or(|a| a.is_empty()) {
                errors.push("no recipient: add at least one e-mail address to the report".into());
                continue;
            }
            let m = Message { id: Uuid::new_v4(), event: "report".into(), severity: "info".into(), title: built.title.clone(), body: built.text.clone(), payload: json!({ "report": r.id, "name": r.name }), recipients: recipients.clone(), html: Some(built.html.clone()) };
            match notify::send(&channel, &m).await {
                Ok(()) => sent_any = true,
                Err(e) => errors.push(format!("{}: {e:#}", channel.kind)),
            }
        }
    }
    let status: &'static str = if skipped { "skipped" } else if errors.is_empty() && sent_any { "sent" } else { "failed" };
    let error = errors.join(" | ");
    let out = Outcome { status, error: error.clone(), recipients: recipients.clone(), title: built.title.clone() };
    let (title, rec) = (built.title, recipients);
    let slot = (trigger == "schedule").then(|| latest_slot(&r, now)).flatten();
    let prev_attempts = r.attempts;
    let prev_slot = r.last_slot_at;
    state
        .db(move |c| {
            c.transaction(|c| {
                diesel::insert_into(report_runs::table).values((report_runs::report_id.eq(id), report_runs::trigger.eq(trigger), report_runs::status.eq(status), report_runs::error.eq(&error), report_runs::recipients.eq(json!(rec)), report_runs::title.eq(&title))).execute(c)?;
                if let Some(slot) = slot {
                    let attempts = if prev_slot == Some(slot) { prev_attempts + 1 } else { 1 };
                    diesel::update(reports::table.find(id)).set((reports::last_slot_at.eq(Some(slot)), reports::attempts.eq(attempts))).execute(c)?;
                }
                diesel::update(reports::table.find(id)).set((reports::last_run_at.eq(Some(now)), reports::last_status.eq(status), reports::last_error.eq(&error))).execute(c)?;
                if status == "failed" {
                    audit::record(c, Event::new("system", "report.failed").target("report", &id.to_string()).details(json!({ "message": error, "trigger": trigger })))?;
                }
                Ok(())
            })
        })
        .await?;
    Ok(out)
}

/// Send every report that is due (called by the notification worker every few seconds).
pub async fn run_due(state: &web::Data<AppState>) -> anyhow::Result<usize> {
    let now = Utc::now();
    let due: Vec<Uuid> = state
        .db(move |c| {
            let all: Vec<Report> = reports::table.filter(reports::enabled.eq(true)).select(Report::as_select()).load(c)?;
            Ok(all.into_iter().filter(|r| is_due(r, now)).map(|r| r.id).collect())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut n = 0;
    for id in due {
        match send(state, id, "schedule", None).await {
            Ok(o) => {
                tracing::info!(report = %id, status = o.status, "report run");
                n += 1;
            }
            Err(e) => tracing::warn!(report = %id, error = ?e, "report failed"),
        }
    }
    Ok(n)
}
