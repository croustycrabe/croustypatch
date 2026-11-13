//! Hardware and resource readings: the parsers, on real-looking /proc and df output.
use croustypatch_agent::sysinfo::*;
use croustypatch_common::protocol::{Heartbeat, HostFacts};

const CPUINFO: &str = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 5800X 8-Core Processor\ncpu MHz\t\t: 3800.000\n\nprocessor\t: 1\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 5800X 8-Core Processor\n\nprocessor\t: 2\nmodel name\t: AMD Ryzen 7 5800X 8-Core Processor\n";
const MEMINFO: &str = "MemTotal:       16318452 kB\nMemFree:         1048576 kB\nMemAvailable:    8388608 kB\nBuffers:          102400 kB\nSwapTotal:       4194300 kB\nSwapFree:        4000000 kB\n";
const DF: &str = "Filesystem     Type 1024-blocks      Used Available Capacity Mounted on\n/dev/nvme0n1p2 ext4     489336968 120000000 369336968      25% /\n/dev/nvme0n1p1 vfat        523248      1234    522014       1% /boot/efi\n/dev/sdb1      xfs     976762584 500000000 476762584      52% /mnt/big disk\n/dev/nvme0n1p2 ext4     489336968 120000000 369336968      25% /var/lib/docker/overlay-mount\nbroken line\n";

#[test]
fn cpu_and_memory_are_read_from_proc() {
    assert_eq!(parse_cpuinfo(CPUINFO), ("AMD Ryzen 7 5800X 8-Core Processor".to_string(), 3));
    assert_eq!(parse_cpuinfo("processor : 0\nprocessor : 1\nHardware\t: BCM2835\n"), ("BCM2835".to_string(), 2), "ARM boards name the SoC differently");
    assert_eq!(parse_cpuinfo(""), (String::new(), 0));
    assert_eq!(parse_meminfo(MEMINFO), (16318452, 8388608, 4194300, 4000000));
    assert_eq!(parse_meminfo("garbage"), (0, 0, 0, 0));
}

#[test]
fn cpu_use_is_the_busy_share_between_two_samples() {
    let a = parse_cpu_stat("cpu  1000 0 500 8000 500 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
    assert_eq!(a, (8500, 10000), "iowait counts as idle");
    let b = parse_cpu_stat("cpu  1600 0 700 8600 600 0 0 0 0 0\n").unwrap();
    // 1500 jiffies passed, 700 of them idle (600 idle + 100 iowait): 800 busy -> 53.3 %
    assert!((cpu_pct(a, b).unwrap() - 53.333).abs() < 0.01, "{:?}", cpu_pct(a, b));
    assert_eq!(cpu_pct(a, a), None, "no time passed: no reading");
    assert_eq!(cpu_pct(b, a), None, "a counter that went backwards (reboot) is ignored");
    assert!(parse_cpu_stat("intr 12345").is_none());
    let s = Sampler::default();
    assert!(s.cpu().is_none(), "the first sample has nothing to compare with");
}

#[test]
fn disks_are_real_filesystems_once_each() {
    let d = parse_df(DF);
    assert_eq!(d.len(), 3, "the second mount of the same device and the broken line are dropped: {d:?}");
    assert_eq!((d[0].mount.as_str(), d[0].fs.as_str(), d[0].total_mb, d[0].used_mb), ("/", "ext4", 477868, 117187));
    assert_eq!(d[2].mount, "/mnt/big disk", "mount points may contain spaces");
}

#[test]
fn older_servers_and_agents_still_understand_each_other() {
    // a heartbeat from an agent before 0.6 has no metrics; one from 0.6 read by an older server only has extra fields
    let hb: Heartbeat = serde_json::from_str(r#"{"on_battery":false,"eco":false,"load_1m":0.5,"uptime_secs":10}"#).unwrap();
    assert!(hb.metrics.is_none());
    let facts: HostFacts = serde_json::from_str(r#"{"hostname":"h","os_family":"linux","distro":"x","os_version":"1","kernel":"k","arch":"x86_64","package_manager":null,"is_laptop":false,"on_battery":false,"virtualization":null,"ip_addresses":[],"agent_version":"0.5.1"}"#).unwrap();
    assert_eq!(facts.hardware.cpu_cores, 0);
}

#[test]
fn virtualization_is_recognised_from_what_the_system_says() {
    assert_eq!(virtualization_from("lxc\n", "", "", false).as_deref(), Some("lxc"), "a container manager wins");
    assert_eq!(virtualization_from("", "QEMU", "Standard PC (Q35 + ICH9, 2009)", true).as_deref(), Some("kvm/qemu"));
    assert_eq!(virtualization_from("", "Microsoft Corporation", "Virtual Machine", true).as_deref(), Some("hyper-v"));
    assert_eq!(virtualization_from("", "Dell Inc.", "PowerEdge R640", false), None, "bare metal");
    assert_eq!(virtualization_from("none", "ASUS", "ROG", false), None);
    assert_eq!(virtualization_from("", "", "", true).as_deref(), Some("vm"), "a hypervisor flag alone still says it is a VM");
}
