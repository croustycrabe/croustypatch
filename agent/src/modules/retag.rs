//! "Apply the suggested version": change the tag of a container's image in its compose file.
//!
//! CroustyPatch updates a compose service by pulling its image and recreating it. A *newer version tag* (`v3.3.1` ->
//! `v3.4.1`) is a different image, so the compose file itself has to say so, or the next `docker compose up` would
//! bring the old one back. This module makes that one change, and only when it can do it with no doubt:
//!
//! - the service has a **literal `image:` line** directly inside its block (not a variable, an anchor, a merge key, an
//!   `extends`, a `build:`-only service, a flow-style mapping);
//! - that line says exactly what the container runs now (so it is the definition the container was created from);
//! - the new reference is the same image with another tag.
//!
//! Anything else is refused with the reason, and the administrator changes the tag by hand. The edit replaces only the
//! tag characters: indentation, quotes, comments and line endings stay as they were, and the file's mode and owner are
//! kept. The original text goes into the task's restore point, so a failed update (or a manual rollback) puts the
//! file back, but only if nobody changed it since.
use anyhow::{anyhow, bail, Context, Result};
use croustypatch_common::crypto::sha256_hex;
use serde::{Deserialize, Serialize};

/// What a file said before an update, and a hash of what it says after: the rollback puts the original back only when
/// the file still is the one the update wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredEdit {
    pub path: String,
    pub original: String,
    pub edited_sha256: String,
}

/// A file the update changes.
#[derive(Debug, Clone, PartialEq)]
pub struct FileEdit {
    pub path: String,
    pub original: String,
    pub edited: String,
}

impl FileEdit {
    pub fn stored(&self) -> StoredEdit {
        StoredEdit { path: self.path.clone(), original: self.original.clone(), edited_sha256: sha256_hex(self.edited.as_bytes()) }
    }
}

/// What planning found for one container.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The tag in this file will change.
    Changed { path: String },
    /// The file already says the new reference (changed by hand, or by an earlier task): only the container needs
    /// recreating.
    AlreadyThere { path: String },
}

/// `ghcr.io/x/y:v3.3.1` -> (`ghcr.io/x/y`, `v3.3.1`). A registry port (`host:5000/y`) is not a tag.
pub fn split_tag(image: &str) -> Option<(&str, &str)> {
    let i = image.rfind(':')?;
    match image.rfind('/') {
        Some(s) if i < s => None,
        _ => Some((&image[..i], &image[i + 1..])),
    }
}

pub fn valid_tag(t: &str) -> bool {
    !t.is_empty() && t.len() <= 128 && t.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The new reference must be the running one with another tag: same registry and repository, no digest.
pub fn check_refs(running: &str, new: &str) -> Result<(), String> {
    if [running, new].iter().any(|r| r.is_empty() || r.len() > 300 || r.chars().any(|c| c.is_whitespace() || c.is_control() || c == '@' || c == '$')) {
        return Err("the image reference is not a plain name:tag".into());
    }
    let (Some((rb, rt)), Some((nb, nt))) = (split_tag(running), split_tag(new)) else {
        return Err(format!("{running} or {new} has no tag"));
    };
    if rb != nb {
        return Err(format!("{new} is not the same image as {running}: only the tag may change"));
    }
    if rt == nt {
        return Err(format!("{new} is the tag that runs already"));
    }
    if !valid_tag(nt) {
        return Err(format!("{nt} is not a valid tag"));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ reading a compose file

struct Line<'a> {
    text: &'a str,
    indent: usize,
    body: &'a str,
}

impl Line<'_> {
    /// A line that says something (not blank, not a comment).
    fn significant(&self) -> bool {
        !self.body.trim().is_empty() && !self.body.starts_with('#')
    }
}

fn lines(text: &str) -> Vec<Line<'_>> {
    text.split_inclusive('\n')
        .map(|l| {
            let t = l.trim_end_matches(['\n', '\r']);
            let body = t.trim_start_matches(' ');
            Line { text: t, indent: t.len() - body.len(), body }
        })
        .collect()
}

/// The key of a mapping line (`name:`, `"name":`) and what follows the colon. None for anything else.
fn key_of(body: &str) -> Option<(&str, &str)> {
    if let Some(q) = body.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = body[1..].find(q)? + 1;
        let rest = body[end + 1..].strip_prefix(':')?;
        return Some((&body[1..end], rest));
    }
    let mut it = body.char_indices();
    while let Some((i, c)) = it.next() {
        if c == ':' && body[i + 1..].chars().next().is_none_or(|n| n == ' ' || n == '\t') {
            return Some((body[..i].trim_end(), &body[i + 1..]));
        }
    }
    None
}

/// What follows a colon is nothing, or a comment: the value is the block on the next lines.
fn only_comment(rest: &str) -> bool {
    let r = rest.trim();
    r.is_empty() || r.starts_with('#')
}

#[derive(Debug, PartialEq)]
struct ImageLine {
    line: usize,
    value: String,
    /// Where the value (inside its quotes) starts and ends in the line.
    from: usize,
    to: usize,
}

/// The `image:` line of `service`. `Ok(None)`: this file does not define it. `Err`: it does, in a way this module will
/// not edit.
fn find_image_line(text: &str, service: &str) -> Result<Option<ImageLine>, String> {
    let ls = lines(text);
    if ls.iter().any(|l| l.significant() && l.text.starts_with('\t')) {
        return Err("the file is indented with tabs".into());
    }
    // `services:` at the top level
    let Some(si) = ls.iter().position(|l| l.significant() && l.indent == 0 && key_of(l.body).is_some_and(|(k, _)| k == "services")) else { return Ok(None) };
    if !only_comment(key_of(ls[si].body).unwrap().1) {
        return Err("`services:` is written inline".into());
    }
    // the block of services: up to the next top-level key
    let end = ls.iter().enumerate().skip(si + 1).find(|(_, l)| l.significant() && l.indent == 0).map_or(ls.len(), |(i, _)| i);
    let Some(first) = (si + 1..end).find(|i| ls[*i].significant()) else { return Ok(None) };
    let svc_indent = ls[first].indent;
    let Some(start) = (si + 1..end).find(|i| ls[*i].significant() && ls[*i].indent == svc_indent && key_of(ls[*i].body).is_some_and(|(k, _)| k == service)) else { return Ok(None) };
    if !only_comment(key_of(ls[start].body).unwrap().1) {
        return Err(format!("service {service} is written inline"));
    }
    // the service's own block: its lines are indented more than its name
    let block_end = (start + 1..end).find(|i| ls[*i].significant() && ls[*i].indent <= svc_indent).unwrap_or(end);
    let Some(child) = (start + 1..block_end).find(|i| ls[*i].significant()) else { return Ok(None) };
    let child_indent = ls[child].indent;
    let direct = |i: &usize| ls[*i].significant() && ls[*i].indent == child_indent;
    let image = (start + 1..block_end).filter(direct).find_map(|i| key_of(ls[i].body).filter(|(k, _)| *k == "image").map(|(_, rest)| (i, rest)));
    let Some((i, rest)) = image else {
        if (start + 1..block_end).filter(direct).any(|i| ls[i].body.starts_with("<<")) {
            return Err(format!("service {service} gets its settings from a merge key (<<), the image may come from there"));
        }
        if (start + 1..block_end).filter(direct).any(|i| key_of(ls[i].body).is_some_and(|(k, _)| k == "extends")) {
            return Err(format!("service {service} extends another service"));
        }
        return Err(format!("service {service} has no image: line (it is built, or the image comes from elsewhere)"));
    };
    // its value: plain, or in quotes, then maybe a comment
    let line = &ls[i];
    let after_key = line.text.len() - rest.len();
    let lead = rest.len() - rest.trim_start().len();
    let v = rest.trim_start();
    let (inner, from) = match v.chars().next() {
        None | Some('#') => return Err(format!("the image of {service} is on the next line")),
        Some(q @ ('"' | '\'')) => {
            let close = v[1..].find(q).map(|c| c + 1).ok_or("an unterminated quote")?;
            if v[1..close].contains('\\') || !only_comment(&v[close + 1..]) {
                return Err(format!("the image line of {service} is not a plain string"));
            }
            (&v[1..close], after_key + lead + 1)
        }
        Some('*' | '&' | '!' | '|' | '>' | '{' | '[') => return Err(format!("the image of {service} is an anchor, an alias or a structure")),
        _ => {
            let cut = v.find(" #").unwrap_or(v.len());
            (v[..cut].trim_end(), after_key + lead)
        }
    };
    Ok(Some(ImageLine { line: i, value: inner.to_string(), from, to: from + inner.len() }))
}

/// Change the tag of `service`'s image from `running` to `new` in the compose files (`(path, text)`, in the order
/// compose reads them: the last file that defines the service decides). The new text goes back into `files`.
pub fn plan_one(service: &str, running: &str, new: &str, files: &mut [(String, String)]) -> Result<Outcome, String> {
    check_refs(running, new)?;
    // the definition in force: the last file that has an image line for this service
    let mut found = None;
    for (n, (path, text)) in files.iter().enumerate() {
        if let Some(l) = find_image_line(text, service).map_err(|e| format!("{path}: {e}"))? {
            found = Some((n, l));
        }
    }
    let Some((n, loc)) = found else {
        let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
        return Err(format!("no `image:` line for service {service} in {}", names.join(", ")));
    };
    let path = files[n].0.clone();
    if loc.value.contains('$') {
        return Err(format!("{path}: the image of {service} is `{}`: the tag comes from a variable, change it where the variable is set", loc.value));
    }
    if loc.value == new {
        return Ok(Outcome::AlreadyThere { path });
    }
    if loc.value != running {
        return Err(format!("{path} says `{}` for {service}, but the container runs `{running}`: it was not created from this line", loc.value));
    }
    let text = &files[n].1;
    // rebuild the file with that one value replaced
    let mut out = String::with_capacity(text.len() + 16);
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        if i == loc.line {
            let t = raw.trim_end_matches(['\n', '\r']);
            out.push_str(&t[..loc.from]);
            out.push_str(new);
            out.push_str(&t[loc.to..]);
            out.push_str(&raw[t.len()..]);
        } else {
            out.push_str(raw);
        }
    }
    files[n].1 = out;
    Ok(Outcome::Changed { path })
}

// ------------------------------------------------------------------------------------------------ files on the host

/// The text of the compose files, from the host. A file that cannot be read as text stops the plan.
pub fn read_files(paths: &[String]) -> Result<Vec<(String, String)>, String> {
    paths.iter().map(|p| std::fs::read_to_string(p).map(|t| (p.clone(), t)).map_err(|e| format!("cannot read {p}: {e}"))).collect()
}

/// Write `content` to `path` through a temporary file in the same directory (so a crash never leaves half a file),
/// keeping the file's mode and owner. A file that cannot be replaced (a single-file bind mount) is overwritten in place.
pub fn write_file(path: &str, content: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::metadata(path).with_context(|| format!("reading the mode of {path}"))?;
    let dir = std::path::Path::new(path).parent().ok_or_else(|| anyhow!("{path} has no directory"))?;
    let tmp = dir.join(format!(".{}.croustypatch-{}", std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("compose"), std::process::id()));
    let replaced = (|| -> Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(meta.mode() & 0o7777))?;
        let _ = std::os::unix::fs::chown(&tmp, Some(meta.uid()), Some(meta.gid()));
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if replaced.is_err() {
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(path, content).with_context(|| format!("writing {path}"))?;
    }
    Ok(())
}

/// Put the original text back, if the file still is what the update wrote (or already is the original).
pub fn revert(edit: &StoredEdit) -> Result<()> {
    let now = std::fs::read_to_string(&edit.path).with_context(|| format!("reading {}", edit.path))?;
    if now == edit.original {
        return Ok(());
    }
    if sha256_hex(now.as_bytes()) != edit.edited_sha256 {
        bail!("{} was changed after the update: it is left as it is, and the old tag has to be put back by hand", edit.path);
    }
    write_file(&edit.path, &edit.original)
}

/// Plan the change of several containers (`(service, running image, new image, compose files)`), in order. Containers
/// of one file are planned one after the other on the evolving text, so the file is edited once. All or nothing: any
/// refusal returns before anything was written.
pub fn plan_all(containers: &[(String, String, String, Vec<String>)], read: &dyn Fn(&[String]) -> Result<Vec<(String, String)>, String>) -> Result<(Vec<FileEdit>, Vec<Outcome>), String> {
    let mut texts: Vec<(String, String, String)> = vec![]; // (path, original, current)
    let mut outcomes = vec![];
    for (service, running, new, paths) in containers {
        let mut files: Vec<(String, String)> = read(paths)?;
        for f in files.iter_mut() {
            match texts.iter().find(|t| t.0 == f.0) {
                // continue from what earlier containers of the same file already changed
                Some((_, _, cur)) => f.1 = cur.clone(),
                None => texts.push((f.0.clone(), f.1.clone(), f.1.clone())),
            }
        }
        let outcome = plan_one(service, running, new, &mut files).map_err(|e| format!("{service}: {e}"))?;
        for (path, text) in files {
            if let Some(t) = texts.iter_mut().find(|t| t.0 == path) {
                t.2 = text;
            }
        }
        outcomes.push(outcome);
    }
    let edits = texts.into_iter().filter(|(_, original, current)| original != current).map(|(path, original, edited)| FileEdit { path, original, edited }).collect();
    Ok((edits, outcomes))
}
