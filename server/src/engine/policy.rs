//! Policy definition (stored as JSONB) and maintenance-window arithmetic.
use super::expr;
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc};
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

/// Weekly recurring window in a fixed UTC offset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    /// 0 = Monday … 6 = Sunday. Empty = every day.
    #[serde(default)]
    pub days: Vec<u32>,
    /// "HH:MM"
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub utc_offset_minutes: i32,
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

struct ParsedWindow {
    days: Vec<u32>,
    start: NaiveTime,
    end: NaiveTime,
    offset: Duration,
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
        Ok(ParsedWindow { days: self.days.clone(), start, end, offset: Duration::minutes(self.utc_offset_minutes as i64) })
    }

    /// Is `now` inside the window?
    pub fn contains(&self, now: DateTime<Utc>) -> bool {
        let Ok(w) = self.parse() else { return true };
        let local = now + w.offset;
        let day_ok = |d: u32| w.days.is_empty() || w.days.contains(&d);
        let t = local.time();
        let today = local.weekday().num_days_from_monday();
        if w.start < w.end {
            day_ok(today) && t >= w.start && t < w.end
        } else {
            // overnight: evening part belongs to today, morning part to yesterday's window
            (day_ok(today) && t >= w.start) || (day_ok((today + 6) % 7) && t < w.end)
        }
    }

    /// Start of the window to use: `now` if already inside, otherwise the next opening.
    pub fn next_start(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.contains(now) {
            return now;
        }
        let Ok(w) = self.parse() else { return now };
        let local = now + w.offset;
        for add in 0..8 {
            let day = local.date_naive() + Duration::days(add);
            if !(w.days.is_empty() || w.days.contains(&day.weekday().num_days_from_monday())) {
                continue;
            }
            let cand = day.and_time(w.start);
            let cand_utc = Utc.from_utc_datetime(&(cand - w.offset));
            if cand_utc > now {
                return cand_utc;
            }
        }
        now
    }
}
