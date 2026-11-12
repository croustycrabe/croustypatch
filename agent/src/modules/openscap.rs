//! OpenSCAP module (spec openscap): runs `oscap xccdf eval` with low CPU and I/O priority, summarises the results and
//! reports them with the SHA-256 of the raw result file.
use crate::exec::{self, Cmd, Runner};
use crate::runner::TaskLog;
use anyhow::{bail, Result};
use croustypatch_common::model::PackageManager;
use croustypatch_common::protocol::{ComplianceReport, FailedRule, OpenScapInfo, ScapProfile, Tailoring};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where the SCAP Security Guide installs its datastreams.
pub const CONTENT_DIR: &str = "/usr/share/xml/scap/ssg/content";

pub fn available() -> bool {
    ["/usr/bin/oscap", "/usr/local/bin/oscap"].iter().any(|p| Path::new(p).exists())
}

/// The SCAP Security Guide datastream for this system, if installed (scap-security-guide / ssg-* packages).
pub fn find_datastream(dir: &Path, distro_hint: &str) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("ssg-") && n.ends_with("-ds.xml"))).collect();
    files.sort();
    let hint = distro_hint.to_ascii_lowercase();
    if let Some(p) = files.iter().find(|p| !hint.is_empty() && p.to_string_lossy().contains(&hint)) {
        return Some(p.clone());
    }
    // No content for this exact release (Ubuntu 26.04 against ssg for 22.04 and 24.04): the newest one of the same distribution.
    let family = hint.trim_end_matches(|c: char| c.is_ascii_digit());
    let release = |p: &PathBuf| -> u32 {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        n.trim_start_matches("ssg-").trim_start_matches(family).chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
    };
    let name = |p: &&PathBuf| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("ssg-{family}")));
    files.iter().filter(name).max_by_key(|p| release(p)).cloned().or_else(|| files.into_iter().next())
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

/// Ids of profiles and rules: letters, digits, `_ . -` only (they end up in a command line and in an XML file).
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 300 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Profiles from `oscap info --profiles <datastream>`: one `id:title` per line.
pub fn parse_profiles(text: &str) -> Vec<ScapProfile> {
    text.lines()
        .filter_map(|l| {
            let (id, title) = l.trim().split_once(':')?;
            (id.starts_with("xccdf_") && valid_id(id)).then(|| ScapProfile { id: id.to_string(), title: title.trim().to_string() })
        })
        .collect()
}

/// `OpenSCAP Library 1.4.2` / `OpenSCAP command line tool (oscap) 1.4.2` -> "1.4.2"
pub fn parse_version(text: &str) -> String {
    text.lines().find(|l| l.contains("OpenSCAP command line tool")).and_then(|l| l.split_whitespace().last()).unwrap_or("").to_string()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The id of the tailored profile inside the tailoring file.
pub fn tailoring_profile_id(t: &Tailoring) -> String {
    format!("xccdf_org.croustypatch_profile_{}", t.id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect::<String>())
}

/// An XCCDF 1.2 tailoring file: the base profile of the datastream with the listed rules deselected.
pub fn tailoring_xml(t: &Tailoring, datastream: &str, time: &str) -> String {
    let pid = tailoring_profile_id(t);
    let mut x = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xccdf-1.2:Tailoring xmlns:xccdf-1.2=\"http://checklists.nist.gov/xccdf/1.2\" id=\"xccdf_org.croustypatch_tailoring_{}\">\n  <xccdf-1.2:benchmark href=\"{}\"/>\n  <xccdf-1.2:version time=\"{}\">1</xccdf-1.2:version>\n  <xccdf-1.2:Profile id=\"{}\" extends=\"{}\">\n    <xccdf-1.2:title>{}</xccdf-1.2:title>\n",
        &pid["xccdf_org.croustypatch_profile_".len()..], xml_escape(datastream), xml_escape(time), pid, xml_escape(&t.base), xml_escape(&t.name)
    );
    for r in &t.disabled_rules {
        x += &format!("    <xccdf-1.2:select idref=\"{}\" selected=\"false\"/>\n", xml_escape(r));
    }
    x + "  </xccdf-1.2:Profile>\n</xccdf-1.2:Tailoring>\n"
}

/// Packages that give a machine OpenSCAP and the SCAP Security Guide content, and a note when the content cannot come
/// from the repositories. None: nothing to offer for this system.
pub fn packages_for(kind: PackageManager, distro: &str) -> Option<(Vec<&'static str>, Option<&'static str>)> {
    let d = distro.to_ascii_lowercase();
    match kind {
        // Debian ships its own content (ssg-debian); Ubuntu and the other derivatives use ssg-debderived.
        PackageManager::Apt => Some((vec!["openscap-scanner", if d.starts_with("debian") { "ssg-debian" } else { "ssg-debderived" }], None)),
        // Arch has openscap but no SCAP content: the SCAP Security Guide has no benchmark for Arch Linux at all (not in
        // the repositories, and not in the AUR package either), so there is nothing useful to install.
        PackageManager::Pacman => None,
        _ => None,
    }
}

/// What to do before an audit when OpenSCAP or its SCAP content may be missing.
#[derive(Debug, PartialEq, Eq)]
pub enum InstallPlan {
    /// Everything is there, the administrator switched installation off, or this system has no packages for it:
    /// the audit runs (or fails with the instructions of [`install_hint`]).
    Nothing,
    /// Install these packages first; `note` says what the system's repositories cannot provide.
    Install { packages: Vec<&'static str>, note: Option<&'static str> },
}

pub fn install_plan(have_oscap: bool, have_content: bool, auto_install: bool, kind: PackageManager, distro: &str) -> InstallPlan {
    if (have_oscap && have_content) || !auto_install {
        return InstallPlan::Nothing;
    }
    match packages_for(kind, distro) {
        Some((packages, note)) => InstallPlan::Install { packages, note },
        None => InstallPlan::Nothing,
    }
}

/// The profile asked for when the content could not be listed and nobody chose one.
pub const DEFAULT_PROFILE: &str = "xccdf_org.ssgproject.content_profile_standard";
/// The recommended profile when none is chosen, by kind of machine: the first of these that the content has. CIS level 1
/// is the common baseline (Ubuntu: cis_level1_*, Red Hat family: cis_*_l1); `standard` (Debian) and ANSSI minimal come
/// next; STIG and level 2 are strict and only used when the content has nothing else.
const SERVER: &[&str] = &["cis_level1_server", "cis_server_l1", "standard", "anssi_bp28_minimal", "cis", "cis_level2_server", "cis_server_l2", "stig"];
const WORKSTATION: &[&str] = &["cis_level1_workstation", "cis_workstation_l1", "standard", "anssi_bp28_minimal", "cis_level1_server", "cis_server_l1", "cis", "stig"];

fn list_some(ids: &[String]) -> String {
    let short = |i: &String| i.replace("xccdf_org.ssgproject.content_profile_", "");
    let mut v: Vec<String> = ids.iter().take(12).map(short).collect();
    if ids.len() > 12 {
        v.push(format!("… ({} in all)", ids.len()));
    }
    v.join(", ")
}

/// The profile to audit with: the one asked for if the content has it (an error naming the ones it has otherwise), or,
/// when none was asked for, the one recommended for this kind of machine (laptops and desktops are workstations).
/// `available` empty means it could not be listed: trust the request.
pub fn pick_profile(requested: &str, available: &[String], workstation: bool) -> Result<String, String> {
    let requested = requested.trim();
    if available.is_empty() {
        return Ok(if requested.is_empty() { DEFAULT_PROFILE.to_string() } else { requested.to_string() });
    }
    if !requested.is_empty() {
        return if available.iter().any(|a| a == requested) { Ok(requested.to_string()) } else { Err(format!("profile {requested} does not exist in this SCAP content; it has: {}", list_some(available))) };
    }
    let has = |short: &str| available.iter().find(|a| a.as_str() == format!("xccdf_org.ssgproject.content_profile_{short}"));
    let order = if workstation { WORKSTATION } else { SERVER };
    Ok(order.iter().find_map(|p| has(p)).unwrap_or(&available[0]).clone())
}

/// The release codename of this machine (`resolute` for Ubuntu 26.04): DISTRIB_CODENAME of /etc/lsb-release, else
/// VERSION_CODENAME of os-release. Lower-case letters only, or empty.
pub fn parse_codename(lsb_release: &str, os_release: &str) -> String {
    let get = |text: &str, key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(|v| v.trim().trim_matches('"').to_string());
    let ok = |v: &String| !v.is_empty() && v.len() <= 32 && v.chars().all(|c| c.is_ascii_lowercase());
    get(lsb_release, "DISTRIB_CODENAME").filter(ok).or_else(|| get(os_release, "VERSION_CODENAME").filter(ok)).unwrap_or_default()
}

pub fn host_codename() -> String {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    parse_codename(&read("/etc/lsb-release"), &read("/etc/os-release"))
}

const CODENAME_CHECK: &str = "^DISTRIB_CODENAME=";

/// The release codenames a datastream's applicability checks expect: Ubuntu content only applies where
/// `/etc/lsb-release` matches `^DISTRIB_CODENAME=noble$` (24.04), so on any other release every rule is "not applicable".
pub fn content_codenames(xml: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for (i, _) in xml.match_indices(CODENAME_CHECK) {
        let rest = &xml[i + CODENAME_CHECK.len()..];
        let name: String = rest.chars().take_while(|c| c.is_ascii_lowercase()).collect();
        if !name.is_empty() && rest[name.len()..].starts_with('$') && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The datastream with its release checks pointed at `codename` (the rules themselves are unchanged), or None when it
/// has no such check or already targets that release. This is how content written for the previous LTS audits a newer
/// one (Ubuntu 26.04 with the 24.04 benchmark) until the SCAP Security Guide ships content for it.
pub fn adapt_codename(xml: &str, codename: &str) -> Option<String> {
    let found = content_codenames(xml);
    if codename.is_empty() || found.is_empty() || found.iter().any(|c| c == codename) {
        return None;
    }
    let mut out = xml.to_string();
    for old in &found {
        out = out.replace(&format!("{CODENAME_CHECK}{old}$"), &format!("{CODENAME_CHECK}{codename}$"));
    }
    Some(out)
}

/// What [`adapt_codename`] would do to this datastream on this machine, said as "noble → resolute" (empty: nothing).
fn adaptation_note(ds: &Path, codename: &str) -> String {
    let Ok(xml) = std::fs::read_to_string(ds) else { return String::new() };
    let found = content_codenames(&xml);
    if codename.is_empty() || found.is_empty() || found.iter().any(|c| c == codename) {
        return String::new();
    }
    format!("{} → {codename}", found.join(", "))
}

/// The adapted copy of `ds` for this machine's release, kept in the agent's state directory and remade when the content
/// changes. None when no adaptation is needed or the copy does not validate (the original is then used).
async fn adapted_datastream(r: &Runner, ds: &Path, state_dir: &Path, codename: &str, log: &TaskLog) -> Option<PathBuf> {
    let stem = ds.file_stem()?.to_str()?;
    let dir = state_dir.join("oscap-content");
    let copy = dir.join(format!("{stem}-{codename}.xml"));
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if copy.is_file() && modified(&copy) >= modified(ds) {
        return Some(copy);
    }
    let xml = std::fs::read_to_string(ds).ok()?;
    let adapted = adapt_codename(&xml, codename)?;
    std::fs::create_dir_all(&dir).ok()?;
    if let Err(e) = crate::config::write_private(&copy, adapted.as_bytes()) {
        log.line(format!("could not write the adapted SCAP content: {e}"));
        return None;
    }
    let cp = copy.to_string_lossy().to_string();
    match r.read(Cmd::new("oscap", &["ds", "sds-validate", &cp]).timeout(300)).await {
        Ok(o) if o.code == 0 => Some(copy),
        Ok(o) => {
            log.line(format!("the adapted SCAP content does not validate ({}), using the original", o.stderr.lines().last().unwrap_or("").trim()));
            let _ = std::fs::remove_file(&copy);
            None
        }
        Err(e) => {
            log.line(format!("could not validate the adapted SCAP content ({e}), using the original"));
            let _ = std::fs::remove_file(&copy);
            None
        }
    }
}

/// Said on Arch, where an audit has nothing to run: the SCAP Security Guide has no benchmark for Arch Linux.
pub const ARCH_NO_CONTENT: &str = "the SCAP Security Guide has no content for Arch Linux (its benchmarks cover Debian, Ubuntu, Red Hat and similar systems), so audits are not available on this system unless you put a datastream of your own in /usr/share/xml/scap/ssg/content";

pub fn install_hint(kind: PackageManager, distro: &str) -> String {
    match (packages_for(kind, distro), kind) {
        (Some((p, _)), _) => format!("install {} with the system's package manager ({})", p.join(" and "), kind.as_str()),
        (None, PackageManager::Pacman) => ARCH_NO_CONTENT.to_string(),
        (None, _) => "OpenSCAP is not available for this system".to_string(),
    }
}

type Cache = Mutex<Option<(PathBuf, u64, OpenScapInfo)>>;
static CACHE: Cache = Mutex::new(None);

/// The machine an audit runs on, as the facts describe it.
pub struct Target<'a> {
    /// "ubuntu26", "debian12"...: which content to look for.
    pub hint: &'a str,
    pub kind: PackageManager,
    pub distro: &'a str,
    /// A laptop or a desktop: the workstation profiles are recommended rather than the server ones.
    pub workstation: bool,
}

/// The content used on this machine, and whether it is meant for another release of it.
fn content_for(hint: &str) -> Option<(PathBuf, bool)> {
    let ds = find_datastream(Path::new(CONTENT_DIR), hint)?;
    let other_release = !hint.is_empty() && !ds.to_string_lossy().contains(&hint.to_ascii_lowercase());
    Some((ds, other_release))
}

/// What OpenSCAP offers on this machine, or None when it is not installed. The profiles are read once per content file.
pub async fn info(distro_hint: &str, workstation: bool) -> Option<OpenScapInfo> {
    if !available() {
        return None;
    }
    let version = exec::run(&Cmd::new("oscap", &["--version"]).timeout(20)).await.ok().map(|o| parse_version(&o.stdout)).unwrap_or_default();
    let Some((ds, other_release)) = content_for(distro_hint) else {
        return Some(OpenScapInfo { version, ..Default::default() });
    };
    let stamp = std::fs::metadata(&ds).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let cached = CACHE.lock().unwrap().as_ref().filter(|(p, s, i)| *p == ds && *s == stamp && i.version == version).map(|c| c.2.clone());
    let mut out = match cached {
        Some(i) => i,
        None => {
            let profiles = exec::run(&Cmd::new("oscap", &["info", "--profiles", &ds.to_string_lossy()]).timeout(60)).await.ok().filter(|o| o.ok()).map(|o| parse_profiles(&o.stdout)).unwrap_or_default();
            let adapted = if other_release { adaptation_note(&ds, &host_codename()) } else { String::new() };
            let out = OpenScapInfo { version, datastream: ds.to_string_lossy().into_owned(), profiles, recommended: String::new(), adapted };
            *CACHE.lock().unwrap() = Some((ds, stamp, out.clone()));
            out
        }
    };
    let ids: Vec<String> = out.profiles.iter().map(|p| p.id.clone()).collect();
    out.recommended = if ids.is_empty() { String::new() } else { pick_profile("", &ids, workstation).unwrap_or_default() };
    Some(out)
}

pub async fn scan(r: &Runner, state_dir: &Path, profile: &str, tailoring: Option<&Tailoring>, host: &Target<'_>, log: &TaskLog) -> Result<ComplianceReport> {
    let (distro_hint, kind, distro) = (host.hint, host.kind, host.distro);
    let content = content_for(distro_hint);
    if kind == PackageManager::Pacman && content.is_none() {
        bail!("{ARCH_NO_CONTENT}");
    }
    if !available() {
        bail!("oscap is not installed: {}", install_hint(kind, distro));
    }
    let (ds, other_release) = content.ok_or_else(|| anyhow::anyhow!("no SCAP Security Guide content in {CONTENT_DIR} ({})", install_hint(kind, distro)))?;
    let name = ds.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let mut dss = ds.to_string_lossy().to_string();
    if other_release {
        let codename = host_codename();
        match adapted_datastream(r, &ds, state_dir, &codename, log).await {
            Some(copy) => {
                log.line(format!("there is no SCAP content for {distro_hint} yet: using {name} (the closest release) with its release checks pointed at {codename}; the rules are those of that release"));
                dss = copy.to_string_lossy().to_string();
            }
            None => log.line(format!("note: there is no SCAP content for {distro_hint} in {CONTENT_DIR}; using {name} (the closest release). Rules written for another release often report \"not applicable\".")),
        }
    }
    // The profiles this content really has: `standard` exists in Debian's, not in Ubuntu's (CIS, STIG…).
    let available: Vec<String> = info(distro_hint, host.workstation).await.map(|i| i.profiles.into_iter().map(|p| p.id).collect()).unwrap_or_default();
    let wanted = profile.trim();
    let mut profile = if wanted.is_empty() { DEFAULT_PROFILE.to_string() } else { wanted.to_string() };
    if !valid_id(&profile) {
        bail!("invalid profile id");
    }
    if tailoring.is_none() {
        let chosen = pick_profile(wanted, &available, host.workstation).map_err(|e| anyhow::anyhow!(e))?;
        if wanted.is_empty() {
            log.line(format!("no profile chosen: auditing with the one recommended for this {} ({})", if host.workstation { "workstation" } else { "server" }, chosen.replace("xccdf_org.ssgproject.content_profile_", "")));
        }
        profile = chosen;
    } else if let Some(t) = tailoring.filter(|t| !available.is_empty() && !available.contains(&t.base)) {
        bail!("the base profile {} of the custom profile does not exist in this content; available: {}", t.base, list_some(&available));
    }
    let mut label = profile.clone();
    let mut tailoring_file: Option<String> = None;
    if let Some(t) = tailoring {
        if !valid_id(&t.base) || t.disabled_rules.len() > 3000 || !t.disabled_rules.iter().all(|x| valid_id(x)) || t.name.len() > 120 || t.id.len() > 64 {
            bail!("invalid custom profile");
        }
        let path = state_dir.join("oscap-tailoring.xml");
        crate::config::write_private(&path, tailoring_xml(t, &dss, &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string()).as_bytes())?;
        profile = tailoring_profile_id(t);
        label = format!("custom:{}", t.name);
        tailoring_file = Some(path.to_string_lossy().to_string());
        log.line(format!("custom profile '{}' based on {} ({} rule(s) switched off)", t.name, t.base, t.disabled_rules.len()));
    }
    let results = state_dir.join("oscap-results.xml");
    let _ = std::fs::remove_file(&results);
    let res = results.to_string_lossy().to_string();
    log.line(format!("evaluating {profile} from {dss}"));
    // Lowest CPU and I/O priority: a compliance audit must never slow production down.
    let mut args: Vec<&str> = vec!["-n", "19", "ionice", "-c", "3", "oscap", "xccdf", "eval", "--profile", &profile];
    if let Some(t) = &tailoring_file {
        args.extend(["--tailoring-file", t]);
    }
    args.extend(["--results", &res, &dss]);
    let out = r.read(Cmd::new("nice", &args).timeout(3600)).await?;
    // oscap exits 2 when some rules fail: that is a result, not an error.
    if out.code != 0 && out.code != 2 {
        bail!("oscap failed ({}): {}", out.code, out.stderr.lines().last().unwrap_or("").trim());
    }
    let xml = std::fs::read_to_string(&results)?;
    let (pass, fail, na, other, failed_rules) = summarize(&xml);
    Ok(ComplianceReport {
        profile: label,
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
