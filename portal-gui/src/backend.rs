//! The agent's local socket: one request, one answer, the same line protocol as the terminal app center. The agent
//! identifies the user from the socket's peer credentials, never from anything this program says.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const DEFAULT_SOCKET: &str = "/run/croustypatch/portal.sock";

fn socket_path() -> String {
    std::env::var("CROUSTYPATCH_PORTAL_SOCKET").ok().filter(|s| s.starts_with('/')).unwrap_or_else(|| DEFAULT_SOCKET.into())
}

pub(crate) fn ask(req: Value) -> Result<Value, String> {
    let path = socket_path();
    let mut s = UnixStream::connect(&path).map_err(|e| format!("agent unreachable at {path} ({e})"))?;
    s.set_read_timeout(Some(Duration::from_secs(90))).map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    s.write_all(&line).map_err(|e| e.to_string())?;
    // At most 8 MiB, like the agent's own limits.
    let mut buf = String::new();
    BufReader::new(std::io::Read::take(s, 8 * 1024 * 1024)).read_line(&mut buf).map_err(|e| e.to_string())?;
    let r: Value = serde_json::from_str(&buf).map_err(|_| "unreadable answer from the agent".to_string())?;
    if r.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(r.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(r.get("error").and_then(Value::as_str).unwrap_or("the agent refused the request").to_string())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A package name as package managers take it: letters, digits and `+ . _ - : ~ @`, not starting with `-` or `.`.
pub fn valid_pkg(p: &str) -> bool {
    !p.is_empty() && p.len() <= 128 && !p.starts_with(['-', '.']) && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '_' | '-' | ':' | '~' | '@'))
}

/// The six operations the window can ask for (list, install, uninstall, updates, update, check); anything else, and
/// any bad identifier, is refused before it reaches the socket. No shell, file system or network access is exposed.
pub enum Op {
    List,
    Install(String),
    Uninstall(String),
    Updates,
    Update(Vec<String>),
    Check,
}

impl Op {
    pub fn request(&self) -> Result<Value, String> {
        Ok(match self {
            Op::List => json!({ "op": "list" }),
            Op::Install(id) | Op::Uninstall(id) if !valid_id(id) => return Err("bad application id".into()),
            Op::Install(id) => json!({ "op": "install", "app_id": id }),
            Op::Uninstall(id) => json!({ "op": "uninstall", "app_id": id }),
            Op::Updates => json!({ "op": "updates" }),
            Op::Update(p) if p.len() > 1000 || !p.iter().all(|x| valid_pkg(x)) => return Err("bad package list".into()),
            Op::Update(p) => json!({ "op": "update", "packages": p }),
            Op::Check => json!({ "op": "check" }),
        })
    }

    pub fn run(&self) -> Result<Value, String> {
        ask(self.request()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_package_names_are_checked_before_the_socket() {
        assert!(valid_id("3f2a-bc") && !valid_id("") && !valid_id("a b") && !valid_id("../x") && !valid_id(&"a".repeat(65)));
        assert!(valid_pkg("libstdc++6") && valid_pkg("python3.12") && valid_pkg("a:b~c@d"));
        assert!(!valid_pkg("-rf") && !valid_pkg(".hidden") && !valid_pkg("a b") && !valid_pkg("a;b") && !valid_pkg(""));
    }

    #[test]
    fn requests_have_the_agents_shape_and_refuse_bad_input() {
        assert_eq!(Op::Install("ab-1".into()).request().unwrap(), json!({ "op": "install", "app_id": "ab-1" }));
        assert_eq!(Op::Update(vec!["less".into()]).request().unwrap(), json!({ "op": "update", "packages": ["less"] }));
        assert!(Op::Uninstall("x y".into()).request().is_err());
        assert!(Op::Update(vec!["a;b".into()]).request().is_err());
        assert_eq!(Op::List.request().unwrap(), json!({ "op": "list" }));
    }
}
