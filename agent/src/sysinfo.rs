//! What the machine is made of and how busy it is, read from /proc and /sys (Linux) without running anything but
//! `df`. No serial numbers or identifiers are collected: only what helps to know the machine (CPU, memory, model).
use crate::exec::{self, Cmd};
use croustypatch_common::protocol::{DiskUsage, Hardware, Metrics};
use std::sync::Mutex;

/// (model name of the first processor, number of logical processors) from /proc/cpuinfo.
pub fn parse_cpuinfo(text: &str) -> (String, u32) {
    let mut model = String::new();
    let mut cores = 0;
    for l in text.lines() {
        let Some((k, v)) = l.split_once(':') else { continue };
        let k = k.trim();
        if k == "processor" {
            cores += 1;
        } else if model.is_empty() && (k == "model name" || k == "Hardware" || k == "Model") {
            model = v.trim().to_string();
        }
    }
    (model, cores)
}

/// All values in kB: (MemTotal, MemAvailable, SwapTotal, SwapFree) from /proc/meminfo.
pub fn parse_meminfo(text: &str) -> (u64, u64, u64, u64) {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':')).and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok()).unwrap_or(0);
    (get("MemTotal"), get("MemAvailable"), get("SwapTotal"), get("SwapFree"))
}

/// (idle, total) jiffies from the aggregate "cpu" line of /proc/stat.
pub fn parse_cpu_stat(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    // idle + iowait count as idle; guest time is already inside user/nice
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    let total: u64 = v.iter().take(8).sum();
    Some((idle, total))
}

/// CPU use between two samples, 0-100.
pub fn cpu_pct(prev: (u64, u64), now: (u64, u64)) -> Option<f32> {
    let total = now.1.checked_sub(prev.1)?;
    if total == 0 {
        return None;
    }
    let idle = now.0.saturating_sub(prev.0).min(total);
    Some(((total - idle) as f64 / total as f64 * 100.0) as f32)
}

/// Mounted real filesystems from `df -P -k -T` output (1 KiB blocks). A device mounted several times (bind mounts,
/// btrfs subvolumes) is the same space: it is listed once, under its first mount.
pub fn parse_df(text: &str) -> Vec<DiskUsage> {
    let mut out: Vec<DiskUsage> = vec![];
    let mut devices: Vec<&str> = vec![];
    for l in text.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        // device type blocks used avail capacity mount
        if f.len() < 7 {
            continue;
        }
        let (Ok(total), Ok(used)) = (f[2].parse::<u64>(), f[3].parse::<u64>()) else { continue };
        let mount = f[6..].join(" ");
        if total == 0 || devices.contains(&f[0]) {
            continue;
        }
        devices.push(f[0]);
        out.push(DiskUsage { mount, fs: f[1].to_string(), total_mb: total / 1024, used_mb: used / 1024 });
    }
    out.truncate(12);
    out
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn dmi(name: &str) -> String {
    // "To Be Filled By O.E.M." and friends are noise
    let v = read(&format!("/sys/class/dmi/id/{name}")).trim().to_string();
    if v.eq_ignore_ascii_case("to be filled by o.e.m.") || v.eq_ignore_ascii_case("default string") || v.eq_ignore_ascii_case("system product name") || v.eq_ignore_ascii_case("none") { String::new() } else { v }
}

/// "lxc", "docker", "kvm/qemu", "vmware"…: what the machine runs on, from what the system says about itself.
pub fn virtualization_from(container: &str, vendor: &str, product: &str, cpu_flags_have_hypervisor: bool) -> Option<String> {
    let c = container.trim();
    if !c.is_empty() && c != "none" {
        return Some(c.to_string());
    }
    let id = format!("{vendor} {product}").to_ascii_lowercase();
    let known = [("qemu", "kvm/qemu"), ("kvm", "kvm"), ("vmware", "vmware"), ("virtualbox", "virtualbox"), ("innotek", "virtualbox"), ("xen", "xen"), ("virtual machine", "hyper-v"), ("bochs", "bochs"), ("amazon ec2", "aws"), ("google compute", "gcp")];
    if let Some((_, name)) = known.iter().find(|(k, _)| id.contains(k)) {
        return Some((*name).to_string());
    }
    cpu_flags_have_hypervisor.then(|| "vm".to_string())
}

pub fn virtualization() -> Option<String> {
    let cpuinfo = read("/proc/cpuinfo");
    let flag = cpuinfo.lines().any(|l| l.starts_with("flags") && l.split_whitespace().any(|f| f == "hypervisor"));
    virtualization_from(&read("/run/systemd/container"), &dmi("sys_vendor"), &dmi("product_name"), flag)
}

/// Static hardware description (cheap: a few small files).
pub fn hardware() -> Hardware {
    let (cpu_model, cpu_cores) = parse_cpuinfo(&read("/proc/cpuinfo"));
    let (mem, _, swap, _) = parse_meminfo(&read("/proc/meminfo"));
    let bios = [dmi("bios_vendor"), dmi("bios_version")].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    Hardware { cpu_model, cpu_cores, mem_total_mb: mem / 1024, swap_total_mb: swap / 1024, vendor: dmi("sys_vendor"), product: dmi("product_name"), bios }
}

/// Takes CPU samples between heartbeats.
#[derive(Default)]
pub struct Sampler {
    prev: Mutex<Option<(u64, u64)>>,
}

impl Sampler {
    pub fn cpu(&self) -> Option<f32> {
        let now = parse_cpu_stat(&read("/proc/stat"))?;
        let mut prev = self.prev.lock().unwrap();
        let pct = prev.and_then(|p| cpu_pct(p, now));
        *prev = Some(now);
        pct
    }
}

/// Current resource use.
pub async fn metrics(sampler: &Sampler) -> Metrics {
    let (mem, avail, swap, swap_free) = parse_meminfo(&read("/proc/meminfo"));
    let load: Vec<f32> = read("/proc/loadavg").split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
    let disks = if exec::has_program("df") {
        let c = Cmd::new("df", &["-P", "-k", "-T", "-x", "tmpfs", "-x", "devtmpfs", "-x", "squashfs", "-x", "overlay", "-x", "efivarfs"]).timeout(10);
        exec::run(&c).await.ok().map(|o| parse_df(&o.stdout)).unwrap_or_default()
    } else {
        vec![]
    };
    Metrics {
        cpu_pct: sampler.cpu(),
        mem_total_mb: mem / 1024,
        mem_used_mb: mem.saturating_sub(avail) / 1024,
        swap_total_mb: swap / 1024,
        swap_used_mb: swap.saturating_sub(swap_free) / 1024,
        load_5m: load.get(1).copied().unwrap_or(0.0),
        load_15m: load.get(2).copied().unwrap_or(0.0),
        disks,
    }
}
