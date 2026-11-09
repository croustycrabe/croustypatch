//! OpenSCAP module (spec openscap): runs `oscap xccdf eval` with low CPU and I/O priority, summarises the results and
//! reports them with the SHA-256 of the raw result file.
use crate::exec::{Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{bail, Result};
use croustypatch_common::protocol::{ComplianceReport, FailedRule};
use std::path::{Path, PathBuf};

pub fn available() -> bool {
    ["/usr/bin/oscap", "/usr/local/bin/oscap"].iter().any(|p| Path::new(p).exists())
}

/// The SCAP Security Guide datastream for this system, if installed (scap-security-guide / ssg-* packages).
pub fn find_datastream(dir: &Path, distro_hint: &str) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("ssg-") && n.ends_with("-ds.xml"))).collect();
    files.sort();
    let hint = distro_hint.to_ascii_lowercase();
    files.iter().find(|p| p.to_string_lossy().contains(&hint)).cloned().or_else(|| files.into_iter().next())
}

/// Summarise an XCCDF result file: counts per result and the failed rules with their severity.
pub fn summarize(xml: &str) -> (u32, u32, u32, u32, Vec<FailedRule>) {
    let (mut pass, mut fail, mut na, mut other) = (0u32, 0u32, 0u32, 0u32);
    let mut failed = vec![];
    let attr = |tag: &str, name: &str| -> Option<String> {
        let k = format!("{name}=\"");
        let i = tag.find(&k)? + k.len();
        Some(tag[i..].split('"').next()?.to_string())
    };
    let mut rest = xml;
    while let Some(start) = rest.find("rule-result ") {
        let tag_end = rest[start..].find('>').map(|i| start + i).unwrap_or(rest.len());
        let tag = &rest[start..tag_end];
        let body_end = rest[tag_end..].find("rule-result>").map(|i| tag_end + i).unwrap_or(rest.len());
        let body = &rest[tag_end..body_end];
        let result = body.find("result>").map(|i| body[i + 7..].split('<').next().unwrap_or("").trim().to_string()).unwrap_or_default();
        match result.as_str() {
            "pass" | "fixed" => pass += 1,
            "fail" => {
                fail += 1;
                if failed.len() < 200 {
                    failed.push(FailedRule { rule_id: attr(tag, "idref").unwrap_or_default(), severity: attr(tag, "severity").unwrap_or_else(|| "unknown".into()) });
                }
            }
            "notapplicable" | "notselected" => na += 1,
            _ => other += 1,
        }
        rest = &rest[body_end.min(rest.len())..];
        if body_end == rest.len() && body_end == 0 {
            break;
        }
    }
    (pass, fail, na, other, failed)
}

/// Compliance score: passed rules over evaluated rules (pass + fail), in percent.
pub fn score(pass: u32, fail: u32) -> f64 {
    if pass + fail == 0 { 0.0 } else { (pass as f64 * 10000.0 / (pass + fail) as f64).round() / 100.0 }
}

pub async fn scan(r: &Runner, state_dir: &Path, profile: &str, distro_hint: &str, log: &TaskLog) -> Result<ComplianceReport> {
    if !available() {
        bail!("oscap is not installed (apt install openscap-scanner ssg-debderived, dnf install openscap-scanner scap-security-guide)");
    }
    let ds = find_datastream(Path::new("/usr/share/xml/scap/ssg/content"), distro_hint).ok_or_else(|| anyhow::anyhow!("no SCAP Security Guide content in /usr/share/xml/scap/ssg/content (install scap-security-guide / ssg-*)"))?;
    let profile = if profile.trim().is_empty() { "xccdf_org.ssgproject.content_profile_standard".to_string() } else { profile.trim().to_string() };
    if !profile.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
        bail!("invalid profile id");
    }
    let results = state_dir.join("oscap-results.xml");
    let _ = std::fs::remove_file(&results);
    let res = results.to_string_lossy().to_string();
    let dss = ds.to_string_lossy().to_string();
    log.line(format!("evaluating {profile} from {dss}"));
    // Lowest CPU and I/O priority: a compliance audit must never slow production down.
    let out = r.read(Cmd::new("nice", &["-n", "19", "ionice", "-c", "3", "oscap", "xccdf", "eval", "--profile", &profile, "--results", &res, &dss]).timeout(3600)).await?;
    // oscap exits 2 when some rules fail: that is a result, not an error.
    if out.code != 0 && out.code != 2 {
        bail!("oscap failed ({}): {}", out.code, out.stderr.lines().last().unwrap_or("").trim());
    }
    let xml = std::fs::read_to_string(&results)?;
    let (pass, fail, na, other, failed_rules) = summarize(&xml);
    Ok(ComplianceReport {
        profile,
        datastream: dss,
        passed: pass,
        failed: fail,
        not_applicable: na,
        other,
        score: score(pass, fail),
        failed_rules,
        results_sha256: croustypatch_common::crypto::sha256_hex(xml.as_bytes()),
        collected_at: chrono::Utc::now(),
    })
}
