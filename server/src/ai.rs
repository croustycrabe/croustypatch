//! AI-assisted contextual risk (spec patch_core 4.1, option A), through any OpenAI-compatible chat API (Ollama,
//! vLLM, LocalAI, OpenAI…).
//!
//! The model only ever *advises* one value: the contextual risk of a patch batch, with its reasons. The stability
//! risk, the decision table, waivers, windows and every safety net stay deterministic. Any problem (provider down,
//! timeout, malformed or out-of-range answer) falls back to the deterministic engine and says so in the rationale.
use crate::engine::policy::PolicyDef;
use crate::engine::risk::{self, AssessInput, Assessment};
use anyhow::{anyhow, bail, Context, Result};
use croustypatch_common::model::RiskLevel;
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct AiConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Zeroizing<String>,
    pub timeout_secs: u64,
}

impl AiConfig {
    /// From the `ai` setting; None when AI is switched off or not configured.
    pub fn from_setting(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
        if v.get("enabled").and_then(|x| x.as_bool()) != Some(true) || s("base_url").is_empty() || s("model").is_empty() {
            return None;
        }
        Some(Self { base_url: s("base_url"), model: s("model"), api_key: Zeroizing::new(s("api_key")), timeout_secs: v.get("timeout_secs").and_then(|x| x.as_u64()).unwrap_or(90).clamp(5, 600) })
    }
}

/// What the model is asked to return.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub contextual_risk: RiskLevel,
    pub rationale: Vec<String>,
    pub confidence: f64,
}

pub const SYSTEM_PROMPT: &str = "You are the risk analyst of a patch management system. You receive one batch of pending \
package updates for one machine, with what is known about the machine and the vulnerabilities fixed. Judge the \
CONTEXTUAL risk of leaving the machine unpatched: how likely and how harmful exploitation is on THIS machine \
(exposure, environment, criticality, exploitation in the wild, CVSS). Do not judge the risk of the update itself. \
Answer with ONE JSON object and nothing else: {\"contextual_risk\": \"Low\"|\"Medium\"|\"High\"|\"Critical\", \
\"rationale\": [up to 4 short sentences], \"confidence\": number between 0 and 1}. The deterministic baseline is \
given; depart from it only for a concrete reason you state.";

/// Facts given to the model. No secrets: host attributes, package names and versions, CVE metadata.
pub fn context(input: &AssessInput, packages: &[Value], baseline: &Assessment) -> Value {
    json!({
        "machine": {
            "os": format!("{} ({})", input.distro, input.os_family),
            "type": input.asset_type,
            "environment": input.asset_environment,
            "criticality": input.criticality,
            "exposed_to_internet": input.internet_exposed,
            "has_staging_copy": input.has_staging,
            "tags": input.tags,
        },
        "updates": {
            "count": input.package_count,
            "security_count": input.security_count,
            "highest_cvss": input.max_cvss,
            "exploited_in_the_wild": input.any_exploited,
            "packages": packages,
        },
        "deterministic_baseline": {
            "contextual_risk": baseline.contextual_risk,
            "matched_rule": baseline.matched_rule,
        }
    })
}

#[derive(Deserialize)]
struct Raw {
    contextual_risk: String,
    #[serde(default)]
    rationale: Value,
    #[serde(default)]
    confidence: Option<f64>,
}

/// Parse and validate the model's answer. Tolerates a ```json fence or text around the object; nothing else.
pub fn parse_verdict(text: &str) -> Result<Verdict> {
    let start = text.find('{').ok_or_else(|| anyhow!("no JSON object in the answer"))?;
    let end = text.rfind('}').ok_or_else(|| anyhow!("no JSON object in the answer"))?;
    let raw: Raw = serde_json::from_str(&text[start..=end]).context("answer is not the expected JSON")?;
    let risk = match raw.contextual_risk.trim().to_ascii_lowercase().as_str() {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "critical" => RiskLevel::Critical,
        other => bail!("contextual_risk {other:?} is not Low, Medium, High or Critical"),
    };
    let rationale: Vec<String> = match raw.rationale {
        Value::Array(a) => a.into_iter().filter_map(|v| v.as_str().map(clean)).filter(|s| !s.is_empty()).take(4).collect(),
        Value::String(s) => vec![clean(&s)],
        _ => vec![],
    };
    let confidence = raw.confidence.filter(|c| c.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
    Ok(Verdict { contextual_risk: risk, rationale, confidence })
}

/// Model text goes into the UI and the audit trail: single line, bounded, no control characters.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect()
}

/// One chat completion; returns the assistant's text.
pub async fn complete(cfg: &AiConfig, system: &str, user: &str) -> Result<String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(cfg.timeout_secs)).build()?;
    let mut req = http.post(&url).json(&json!({
        "model": cfg.model,
        "temperature": 0,
        "response_format": { "type": "json_object" },
        "messages": [ { "role": "system", "content": system }, { "role": "user", "content": user } ],
    }));
    if !cfg.api_key.is_empty() {
        req = req.bearer_auth(cfg.api_key.as_str());
    }
    let r = req.send().await.with_context(|| format!("contacting {url}"))?;
    let status = r.status();
    let body: Value = r.json().await.context("reading the provider's answer")?;
    if !status.is_success() {
        bail!("provider answered {status}: {}", body.get("error").map(|e| e.to_string()).unwrap_or_default().chars().take(200).collect::<String>());
    }
    body.pointer("/choices/0/message/content").and_then(|v| v.as_str()).map(str::to_owned).ok_or_else(|| anyhow!("no message in the provider's answer"))
}

pub async fn verdict(cfg: &AiConfig, ctx: &Value) -> Result<Verdict> {
    let text = complete(cfg, SYSTEM_PROMPT, &serde_json::to_string_pretty(ctx)?).await?;
    parse_verdict(&text)
}

/// Something that can advise on the contextual risk; the planner is synchronous, so this is too.
pub trait Advisor {
    fn contextual(&self, ctx: &Value) -> Result<Verdict>;
    fn label(&self) -> String;
}

/// The real provider, called from the planner's blocking database thread.
pub struct Provider(pub AiConfig);

impl Advisor for Provider {
    fn contextual(&self, ctx: &Value) -> Result<Verdict> {
        let h = tokio::runtime::Handle::try_current().map_err(|_| anyhow!("no async runtime"))?;
        h.block_on(verdict(&self.0, ctx))
    }
    fn label(&self) -> String {
        self.0.model.clone()
    }
}

/// Apply the AI's contextual risk to a deterministic assessment (or record why it could not be used).
pub fn apply(def: &PolicyDef, mut a: Assessment, advisor: Option<&dyn Advisor>, ctx: &Value) -> Assessment {
    let Some(adv) = advisor else {
        a.rationale.insert(0, "AI mode requested but no AI provider is configured (Settings > AI); deterministic engine used".into());
        return a;
    };
    match adv.contextual(ctx) {
        Ok(v) => {
            let base = a.contextual_risk;
            a.engine = "ai".into();
            // Guardrail: a model may raise the risk freely, but lowering it is where a mistake (a hallucinated "not
            // exposed", say) leaves a vulnerable machine waiting; so it may go at most one level below the rules.
            let floor = lower(base);
            let clamped = v.contextual_risk.rank() < floor.rank();
            a.contextual_risk = if clamped { floor } else { v.contextual_risk };
            let (action, decision) = risk::decide(def, a.contextual_risk, a.stability_risk);
            a.action = action;
            a.matched_decision = decision;
            let mut why = vec![format!("AI ({}) contextual risk {} (deterministic baseline {}), confidence {:.0}%", adv.label(), v.contextual_risk.as_str(), base.as_str(), v.confidence * 100.0)];
            if clamped {
                why.push(format!("AI may lower the risk by one level at most: kept at {}", floor.as_str()));
            }
            why.extend(v.rationale.into_iter().map(|r| format!("AI: {r}")));
            a.rationale.splice(0..0, why);
        }
        Err(e) => {
            a.rationale.insert(0, format!("AI analysis failed ({e:#}); deterministic engine used"));
        }
    }
    a
}

/// One level down (Low stays Low).
pub fn lower(l: RiskLevel) -> RiskLevel {
    match l {
        RiskLevel::Critical => RiskLevel::High,
        RiskLevel::High => RiskLevel::Medium,
        _ => RiskLevel::Low,
    }
}
