//! Policy definition (stored as JSONB) and maintenance-window arithmetic.
use super::expr;
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use croustypatch_common::model::{HealthProbe, PolicyAction, RiskLevel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDef {
    /// When true the contextual risk comes from the AI provider (falls back to the rules on error).
    #[serde(default)]
    pub ai_mode_enabled: bool,
    /// Boolean expression over host attributes; empty = every host of the linked groups.
    #[serde(default)]
    pub target_scope: String,
    /// Only consider security updates (default) or every available update.
    #[serde(default = "yes")]
    pub security_only: bool,
    /// Ordered deterministic rules; first match decides the contextual risk.
    #[serde(default)]
    pub rules: Vec<RiskRule>,
    /// Ordered decisions on (contextual risk, stability risk); first match wins.
    #[serde(default)]
    pub decisions: Vec<DecisionRule>,
    #[serde(default = "default_action")]
    pub default_action: PolicyAction,
    #[serde(default)]
    pub maintenance_window: Option<MaintenanceWindow>,
    #[serde(default)]
    pub rollback: RollbackCfg,
    #[serde(default)]
    pub reboot: RebootCfg,
    #[serde(default)]
    pub rings: RingCfg,
}

fn yes() -> bool {
    true
}
fn default_action() -> PolicyAction {
    PolicyAction::RequireApproval
}

impl Default for PolicyDef {
    fn default() -> Self {
        serde_json::from_str("{}").expect("empty policy is valid")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskRule {
    pub rule_id: String,
    pub condition: String,
    #[serde(alias = "fallback_contextual_risk")]
    pub contextual_risk: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionConditions {
    /// Exact match, if set.
    pub contextual_risk: Option<RiskLevel>,
    pub stability_risk: Option<RiskLevel>,
    /// Inclusive bounds, for rules such as "High or above".
    pub contextual_risk_min: Option<RiskLevel>,
    pub stability_risk_max: Option<RiskLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRule {
    #[serde(default)]
    pub conditions: DecisionConditions,
    pub action: PolicyAction,
}

impl DecisionConditions {
    pub fn matches(&self, contextual: RiskLevel, stability: RiskLevel) -> bool {
        self.contextual_risk.is_none_or(|r| r == contextual)
            && self.stability_risk.is_none_or(|r| r == stability)
            && self.contextual_risk_min.is_none_or(|r| contextual.rank() >= r.rank())
            && self.stability_risk_max.is_none_or(|r| stability.rank() <= r.rank())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackCfg {
    #[serde(default = "default_soak")]
    pub soak_minutes: u32,
    #[serde(default)]
    pub probes: Vec<HealthProbe>,
    #[serde(default = "yes")]
    pub auto_rollback: bool,
    #[serde(default = "yes")]
    pub snapshot_required: bool,
}
fn default_soak() -> u32 {
    30
}
impl Default for RollbackCfg {
    fn default() -> Self {
        Self { soak_minutes: default_soak(), probes: vec![], auto_rollback: true, snapshot_required: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RebootCfg {
    /// "manual" | "auto" | "user_prompt". Executed by the reboot module (milestone 2).
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RingCfg {
    /// Ring N is scheduled `N * delay_minutes` after ring 0.
    #[serde(default)]
    pub delay_minutes: u32,
}

/// How a patch day repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    /// On the listed weekdays (every day when none are listed), every week.
    #[default]
    Weekly,
    /// The Nth listed weekday of each month ("first Monday"): `nth` 1..=4, or 5 for the last one.
    MonthlyWeekday,
    /// The same day of each month (`day_of_month` 1..=28).
    MonthlyDay,
}

/// A recurring patch window: when updates may run. Existing weekly windows (days, start, end, UTC offset) load
/// unchanged; `repeat`, `nth`, `day_of_month` and `timezone` are additions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    /// 0 = Monday … 6 = Sunday. Empty = every day (weekly repeat only).
    #[serde(default)]
    pub days: Vec<u32>,
    /// "HH:MM", local time in the window's zone. `end` earlier than `start` means the window ends the next day.
    pub start: String,
    pub end: String,
    /// Fixed offset from UTC; ignored when `timezone` is set.
    #[serde(default)]
    pub utc_offset_minutes: i32,
    /// IANA zone such as "Europe/Paris": daylight saving time is followed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default)]
    pub repeat: Repeat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_of_month: Option<u32>,
}

impl Default for MaintenanceWindow {
    fn default() -> Self {
        Self { days: vec![], start: "02:00".into(), end: "04:00".into(), utc_offset_minutes: 0, timezone: None, repeat: Repeat::Weekly, nth: None, day_of_month: None }
    }
}

/// One concrete occurrence of a window, in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Occurrence {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

enum Zone {
    Fixed(Duration),
    Named(chrono_tz::Tz),
}

struct ParsedWindow {
    start: NaiveTime,
    end: NaiveTime,
    zone: Zone,
}

impl ParsedWindow {
    fn local(&self, utc: DateTime<Utc>) -> NaiveDateTime {
        match &self.zone {
            Zone::Fixed(o) => (utc + *o).naive_utc(),
            Zone::Named(tz) => utc.with_timezone(tz).naive_local(),
        }
    }

    /// Local wall-clock time to UTC. An ambiguous time (clocks go back) takes its first occurrence; a time that does
    /// not exist (clocks go forward) moves to the first moment after the gap.
    fn utc(&self, local: NaiveDateTime) -> DateTime<Utc> {
        match &self.zone {
            Zone::Fixed(o) => Utc.from_utc_datetime(&(local - *o)),
            Zone::Named(tz) => match tz.from_local_datetime(&local) {
                LocalResult::Single(t) => t.with_timezone(&Utc),
                LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
                LocalResult::None => tz.from_local_datetime(&(local + Duration::hours(1))).earliest().map(|t| t.with_timezone(&Utc)).unwrap_or_else(|| Utc.from_utc_datetime(&local)),
            },
        }
    }
}

impl PolicyDef {
    /// Validate expressions and the window; used at save time so bad policies never reach the scheduler.
    pub fn validate(&self) -> Result<(), String> {
        if !self.target_scope.trim().is_empty() {
            expr::parse(&self.target_scope).map_err(|e| format!("target_scope: {e}"))?;
        }
        for r in &self.rules {
            expr::parse(&r.condition).map_err(|e| format!("rule {}: {e}", r.rule_id))?;
        }
        if let Some(w) = &self.maintenance_window {
            w.parse().map_err(|e| format!("maintenance_window: {e}"))?;
        }
        if self.rollback.soak_minutes > 24 * 60 {
            return Err("rollback.soak_minutes must be <= 1440".into());
        }
        Ok(())
    }
}

impl MaintenanceWindow {
    fn parse(&self) -> Result<ParsedWindow, String> {
        let t = |s: &str| NaiveTime::parse_from_str(s, "%H:%M").map_err(|_| format!("bad time '{s}', expected HH:MM"));
        if self.days.iter().any(|d| *d > 6) {
            return Err("days must be 0..=6".into());
        }
        let (start, end) = (t(&self.start)?, t(&self.end)?);
        if start == end {
            return Err("start and end must differ".into());
        }
        let zone = match self.timezone.as_deref().map(str::trim).filter(|z| !z.is_empty()) {
            Some(z) => Zone::Named(z.parse::<chrono_tz::Tz>().map_err(|_| format!("unknown timezone '{z}' (e.g. Europe/Paris)"))?),
            None => Zone::Fixed(Duration::minutes(self.utc_offset_minutes as i64)),
        };
        match self.repeat {
            Repeat::Weekly => {}
            Repeat::MonthlyWeekday => {
                if self.days.is_empty() {
                    return Err("choose the weekday of the patch day".into());
                }
                if !self.nth.is_some_and(|n| (1..=5).contains(&n)) {
                    return Err("nth must be 1 to 4, or 5 for the last".into());
                }
            }
            Repeat::MonthlyDay => {
                if !self.day_of_month.is_some_and(|d| (1..=28).contains(&d)) {
                    return Err("day_of_month must be 1 to 28".into());
                }
            }
        }
        Ok(ParsedWindow { start, end, zone })
    }

    /// Does the window open on this calendar date (local to its zone)?
    fn opens_on(&self, d: NaiveDate) -> bool {
        match self.repeat {
            Repeat::Weekly => self.days.is_empty() || self.days.contains(&d.weekday().num_days_from_monday()),
            Repeat::MonthlyWeekday => {
                if !self.days.contains(&d.weekday().num_days_from_monday()) {
                    return false;
                }
                match self.nth {
                    Some(5) => (d + Duration::days(7)).month() != d.month(),
                    Some(n) => (d.day() - 1) / 7 + 1 == n,
                    None => false,
                }
            }
            Repeat::MonthlyDay => self.day_of_month == Some(d.day()),
        }
    }

    fn occurrence_on(&self, w: &ParsedWindow, d: NaiveDate) -> Occurrence {
        let start = w.utc(d.and_time(w.start));
        let end_day = if w.start < w.end { d } else { d + Duration::days(1) };
        Occurrence { start, end: w.utc(end_day.and_time(w.end)) }
    }

    /// The occurrence `now` is inside, if any.
    pub fn current(&self, now: DateTime<Utc>) -> Option<Occurrence> {
        let w = self.parse().ok()?;
        let today = w.local(now).date();
        // A window is shorter than a day: only yesterday's or today's occurrence can contain `now`.
        [today - Duration::days(1), today].into_iter().filter(|d| self.opens_on(*d)).map(|d| self.occurrence_on(&w, d)).find(|o| o.start <= now && now < o.end)
    }

    /// Is `now` inside the window? An invalid window never blocks anything.
    pub fn contains(&self, now: DateTime<Utc>) -> bool {
        self.parse().is_err() || self.current(now).is_some()
    }

    /// The next `n` occurrences that start after `now` (the current one is not included).
    pub fn upcoming(&self, now: DateTime<Utc>, n: usize) -> Vec<Occurrence> {
        let Ok(w) = self.parse() else { return vec![] };
        let today = w.local(now).date();
        let mut out = vec![];
        // The sparsest pattern (the last weekday of a month) repeats within 5 weeks; scan well beyond that.
        for add in 0..(n as i64 * 35 + 70) {
            let d = today + Duration::days(add);
            if self.opens_on(d) {
                let o = self.occurrence_on(&w, d);
                if o.start > now {
                    out.push(o);
                    if out.len() == n {
                        break;
                    }
                }
            }
        }
        out
    }

    /// The window to use for a task: the current occurrence when inside one, else the next.
    pub fn occurrence(&self, now: DateTime<Utc>) -> Option<Occurrence> {
        self.current(now).or_else(|| self.upcoming(now, 1).into_iter().next())
    }

    /// Start of the window to use: `now` if already inside, otherwise the next opening.
    pub fn next_start(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.contains(now) {
            return now;
        }
        self.upcoming(now, 1).first().map(|o| o.start).unwrap_or(now)
    }
}
