//! The only way the agent runs external programs.
//!
//! No shell is ever involved, programs are looked up in fixed system directories (never `$PATH`),
//! the environment is wiped, stdin is closed unless data is supplied, and every call has a timeout.
//! Callers pass arguments as separate strings, so package names can never be interpreted as options
//! or shell syntax (they are also validated before they get here).
use anyhow::{anyhow, bail, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const SEARCH_DIRS: &[&str] = &["/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin"];
const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

pub fn find_program(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return None; // only bare names from the allow-listed directories
    }
    SEARCH_DIRS.iter().map(|d| PathBuf::from(d).join(name)).find(|p| p.is_file())
}

pub fn has_program(name: &str) -> bool {
    find_program(name).is_some()
}

#[derive(Debug, Clone)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

#[derive(Debug, Clone)]
pub struct Cmd {
    pub program: &'static str,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, String)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
}

impl Cmd {
    pub fn new(program: &'static str, args: &[&str]) -> Self {
        Self { program, args: args.iter().map(|s| s.to_string()).collect(), env: vec![], stdin: None, timeout: Duration::from_secs(120), cwd: None }
    }
    pub fn args<I: IntoIterator<Item = String>>(mut self, a: I) -> Self {
        self.args.extend(a);
        self
    }
    pub fn env(mut self, k: &'static str, v: &str) -> Self {
        self.env.push((k, v.to_string()));
        self
    }
    pub fn stdin(mut self, data: Vec<u8>) -> Self {
        self.stdin = Some(data);
        self
    }
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }
    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout = Duration::from_secs(secs);
        self
    }
    /// Human-readable form for logs.
    pub fn display(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
    }
}

fn clip(mut b: Vec<u8>) -> String {
    b.truncate(MAX_OUTPUT);
    String::from_utf8_lossy(&b).into_owned()
}

/// Run a command to completion.
pub async fn run(cmd: &Cmd) -> Result<Output> {
    let path = find_program(cmd.program).ok_or_else(|| anyhow!("{} is not installed", cmd.program))?;
    let mut c = Command::new(path);
    c.args(&cmd.args).env_clear().env("PATH", SAFE_PATH).env("LC_ALL", "C").env("LANG", "C").kill_on_drop(true).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in &cmd.env {
        c.env(k, v);
    }
    if let Some(d) = &cmd.cwd {
        c.current_dir(d);
    }
    c.stdin(if cmd.stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = c.spawn().map_err(|e| anyhow!("cannot start {}: {e}", cmd.program))?;
    if let (Some(data), Some(mut si)) = (cmd.stdin.clone(), child.stdin.take()) {
        si.write_all(&data).await?;
        drop(si);
    }
    match tokio::time::timeout(cmd.timeout, child.wait_with_output()).await {
        Ok(out) => {
            let out = out?;
            Ok(Output { code: out.status.code().unwrap_or(-1), stdout: clip(out.stdout), stderr: clip(out.stderr) })
        }
        Err(_) => bail!("{} timed out after {}s", cmd.program, cmd.timeout.as_secs()),
    }
}

/// Executes commands, or only logs them in dry-run mode.
#[derive(Clone)]
pub struct Runner {
    pub dry_run: bool,
}

impl Runner {
    /// Read-only commands always run.
    pub async fn read(&self, cmd: Cmd) -> Result<Output> {
        run(&cmd).await
    }

    /// Commands that modify the system. In dry-run they are logged and reported as successful.
    pub async fn mutate(&self, cmd: Cmd, log: &crate::runner::TaskLog) -> Result<Output> {
        log.line(format!("$ {}", cmd.display()));
        if self.dry_run {
            log.line("[dry-run] not executed".into());
            return Ok(Output { code: 0, stdout: String::new(), stderr: String::new() });
        }
        let out = run(&cmd).await?;
        for l in out.stdout.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
            log.line(l.to_string());
        }
        if !out.ok() {
            for l in out.stderr.lines().take(40) {
                log.line(format!("stderr: {l}"));
            }
        }
        Ok(out)
    }
}
