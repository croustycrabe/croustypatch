//! Risk assessment (contextual + stability) and the policy decision.
use super::expr::{self, Context, Value};
use super::policy::PolicyDef;
use croustypatch_common::model::{PolicyAction, RiskLevel};
use serde::{Deserialize, Serialize};

/// Facts about the asset and the candidate update batch the engine reasons on.
#[derive(Debug, Clone, Default)]
pub struct AssessInput {
    pub max_cvss: f64,
    pub any_exploited: bool,
    pub security_count: usize,
    pub package_count: usize,
    pub includes_kernel: bool,
    pub critical_packages: Vec<String>,
    pub asset_environment: String,
    pub asset_type: String,
    pub criticality: String,
    pub os_family: String,
    pub distro: String,
    pub hostname: String,
    pub internet_exposed: bool,
    pub has_staging: bool,
    pub ring: i32,
    pub tags: Vec<String>,
    pub modules: Vec<String>,
}

impl AssessInput {
    /// Variables visible to rule and scope expressions.
    pub fn context(&self) -> Context {
        let mut c = Context::new();
        let mut set = |k: &str, v: Value| {
            c.insert(k.to_string(), v);
        };
        set("cvss_score", self.max_cvss.into());
        set("exploited", self.any_exploited.into());
        set("security_count", (self.security_count as i64).into());
        set("package_count", (self.package_count as i64).into());
        set("includes_kernel", self.includes_kernel.into());
        set("asset_environment", self.asset_environment.clone().into());
        set("environment", self.asset_environment.clone().into());
        set("asset_type", self.asset_type.clone().into());
        set("criticality", self.criticality.clone().into());
        set("os_family", self.os_family.clone().into());
        set("distro", self.distro.clone().into());
        set("hostname", self.hostname.clone().into());
        set("port_exposed_to_internet", self.internet_exposed.into());
        set("has_staging", self.has_staging.into());
        set("ring", (self.ring as i64).into());
        set("tags", self.tags.clone().into());
        set("has_docker_module", self.modules.iter().any(|m| m == "docker").into());
        set("has_helper_script_module", self.modules.iter().any(|m| m == "helper-script").into());
        c
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assessment {
    pub engine: String, // "deterministic" | "ai"
    pub contextual_risk: RiskLevel,
    pub stability_risk: RiskLevel,
    pub matched_rule: Option<String>,
    pub rationale: Vec<String>,
    pub action: PolicyAction,
    pub matched_decision: Option<usize>,
}

/// Packages whose update is more likely to disturb a running system.
pub fn is_sensitive_package(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const PREFIXES: &[&str] = &["linux", "kernel", "systemd", "glibc", "libc6", "grub", "openssl", "libssl", "postgresql", "mysql", "mariadb", "docker", "containerd", "nginx", "haproxy", "zfs", "openzfs", "freebsd-"];
    PREFIXES.iter().any(|p| n == *p || n.starts_with(p))
}

pub fn is_kernel_package(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("linux-image") || n.starts_with("kernel") || n == "linux" || n.starts_with("linux-lts") || n.starts_with("linux-zen") || n == "freebsd-kernel" || n == "freebsd-kernel-generic"
}

/// Contextual risk, deterministic path.
pub fn contextual_deterministic(def: &PolicyDef, i: &AssessInput) -> (RiskLevel, Option<String>, Vec<String>) {
    let ctx = i.context();
    let mut why = vec![];
    for r in &def.rules {
        match expr::eval_str(&r.condition, &ctx) {
            Ok(true) => {
                why.push(format!("rule '{}' matched: {}", r.rule_id, r.condition));
                return (r.contextual_risk, Some(r.rule_id.clone()), why);
            }
            Ok(false) => {}
            Err(e) => why.push(format!("rule '{}' skipped: {e}", r.rule_id)),
        }
    }
    // No rule matched: derive from CVSS, then adjust for exploitation and exposure.
    let mut level = RiskLevel::from_cvss(i.max_cvss);
    why.push(format!("no rule matched; base risk {} from CVSS {:.1}", level, i.max_cvss));
    if i.any_exploited && level.rank() < RiskLevel::High.rank() {
        level = RiskLevel::High;
        why.push("known exploited vulnerability: raised to High".into());
    }
    if i.internet_exposed && level.rank() >= RiskLevel::Medium.rank() && level != RiskLevel::Critical {
        level = bump(level);
        why.push("host exposed to the internet: risk raised one level".into());
    }
    (level, None, why)
}

fn bump(l: RiskLevel) -> RiskLevel {
    match l {
        RiskLevel::Low => RiskLevel::Medium,
        RiskLevel::Medium => RiskLevel::High,
        _ => RiskLevel::Critical,
    }
}

/// Risk that applying the batch breaks the machine.
pub fn stability_risk(i: &AssessInput) -> (RiskLevel, Vec<String>) {
    let mut pts = 0i32;
    let mut why = vec![];
    let mut add = |n: i32, msg: String| {
        pts += n;
        why.push(format!("{n:+} {msg}"));
    };
    if i.includes_kernel {
        add(2, "kernel update (reboot and driver risk)".into());
    }
    let sensitive: Vec<_> = i.critical_packages.iter().filter(|p| !is_kernel_package(p)).collect();
    if !sensitive.is_empty() {
        add(sensitive.len().min(3) as i32, format!("sensitive packages: {}", sensitive.iter().take(5).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    match i.criticality.as_str() {
        "high" => add(2, "high business criticality".into()),
        "medium" => add(1, "medium business criticality".into()),
        _ => {}
    }
    if i.asset_environment == "production" {
        add(1, "production environment".into());
    }
    if i.has_staging {
        add(-1, "a staging twin validates the change".into());
    } else if i.asset_environment == "production" {
        add(1, "no staging environment to test on".into());
    }
    if i.package_count > 20 {
        add(1, format!("large batch ({} packages)", i.package_count));
    }
    let level = match pts {
        i32::MIN..=1 => RiskLevel::Low,
        2..=3 => RiskLevel::Medium,
        4..=5 => RiskLevel::High,
        _ => RiskLevel::Critical,
    };
    (level, why)
}

/// Pick the action for a (contextual, stability) pair.
pub fn decide(def: &PolicyDef, contextual: RiskLevel, stability: RiskLevel) -> (PolicyAction, Option<usize>) {
    for (idx, d) in def.decisions.iter().enumerate() {
        if d.conditions.matches(contextual, stability) {
            return (d.action, Some(idx));
        }
    }
    (def.default_action, None)
}

/// Full deterministic evaluation.
pub fn assess_deterministic(def: &PolicyDef, i: &AssessInput) -> Assessment {
    let (contextual, rule, mut why) = contextual_deterministic(def, i);
    let (stability, swhy) = stability_risk(i);
    why.extend(swhy.into_iter().map(|s| format!("stability {s}")));
    let (action, decision) = decide(def, contextual, stability);
    Assessment {
        engine: "deterministic".into(),
        contextual_risk: contextual,
        stability_risk: stability,
        matched_rule: rule,
        rationale: why,
        action,
        matched_decision: decision,
    }
}
