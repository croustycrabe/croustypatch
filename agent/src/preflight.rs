//! Checks run before a patch touches the system.
//!
//! Package maintainer scripts expect a plain root environment: they write kernel modules, rebuild the initramfs,
//! switch users and restart services. If the agent itself runs inside a service sandbox (a private mount namespace
//! with read-only system paths, or with capabilities stripped below what root is allowed on this machine), those
//! scripts fail half-way, and the rollback fails the same way. So a sandboxed agent refuses to patch instead.
//!
//! A container is not a sandbox in this sense: its root shares the container's mount namespace and its bounding set,
//! so its limits apply to any administrator on that system and the agent does not second-guess them.

/// Capability numbers worth naming in the message.
const NAMED_CAPS: &[(u32, &str)] = &[(6, "CAP_SETGID"), (7, "CAP_SETUID"), (16, "CAP_SYS_MODULE"), (21, "CAP_SYS_ADMIN")];

fn cap_mask(status: &str, field: &str) -> Option<u64> {
    status.lines().find_map(|l| l.strip_prefix(field)).and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
}

/// Reasons this process is more restricted than root on this system; empty when patching is safe.
/// `self_mnt` / `init_mnt` are the targets of `/proc/self/ns/mnt` and `/proc/1/ns/mnt` (None when unreadable).
pub fn sandbox_problems(self_status: &str, self_mnt: Option<&str>, init_mnt: Option<&str>) -> Vec<String> {
    let mut out = vec![];
    if let (Some(a), Some(b)) = (self_mnt, init_mnt) {
        if a != b {
            out.push("the agent runs in a private mount namespace (service sandboxing such as ProtectKernelModules, ProtectSystem or PrivateTmp): system paths like /usr/lib/modules may be read-only or hidden".to_string());
        }
    }
    if let (Some(eff), Some(bnd)) = (cap_mask(self_status, "CapEff:"), cap_mask(self_status, "CapBnd:")) {
        let missing = bnd & !eff;
        if missing != 0 {
            let named: Vec<&str> = NAMED_CAPS.iter().filter(|(b, _)| missing & (1 << b) != 0).map(|(_, n)| *n).collect();
            let list = if named.is_empty() { format!("mask {missing:#x}") } else { named.join(", ") };
            out.push(format!("the agent lacks capabilities that root has on this system ({list})"));
        }
    }
    out
}

/// Same check against the running process. Linux only; elsewhere nothing is reported.
pub fn current_problems() -> Vec<String> {
    if !cfg!(target_os = "linux") {
        return vec![];
    }
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let link = |p: &str| std::fs::read_link(p).ok().map(|x| x.to_string_lossy().into_owned());
    sandbox_problems(&status, link("/proc/self/ns/mnt").as_deref(), link("/proc/1/ns/mnt").as_deref())
}
