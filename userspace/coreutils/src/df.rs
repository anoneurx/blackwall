//! df — report filesystem disk space usage
use colored::Colorize;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let human = args.iter().any(|a| a == "-h" || a == "--human-readable");

    println!(
        "{:<20} {:>12} {:>12} {:>12} {:>6}  {}",
        "Filesystem".bold(),
        "Size".bold(),
        "Used".bold(),
        "Available".bold(),
        "Use%".bold(),
        "Mounted on".bold()
    );
    println!("{}", "─".repeat(72).dimmed());

    // Read /proc/mounts for mount points.
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    for line in mounts.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let dev = parts[0];
        let mount = parts[1];

        // Skip pseudo-filesystems in normal view.
        if !args.iter().any(|a| a == "-a" || a == "--all") {
            if [
                "proc",
                "sysfs",
                "devtmpfs",
                "devpts",
                "tmpfs",
                "cgroup",
                "bpf",
                "pstore",
                "securityfs",
                "debugfs",
                "tracefs",
                "hugetlbfs",
                "mqueue",
                "fusectl",
                "configfs",
            ]
            .contains(&parts.get(2).copied().unwrap_or(""))
            {
                continue;
            }
        }

        let (size, used, avail) = statfs(mount);
        if size == 0 {
            continue;
        }
        let pct = if size > 0 { (used * 100) / size } else { 0 };
        let pct_str = if pct > 90 {
            format!("{}%", pct).bright_red().to_string()
        } else if pct > 70 {
            format!("{}%", pct).bright_yellow().to_string()
        } else {
            format!("{}%", pct)
        };

        println!(
            "{:<20} {:>12} {:>12} {:>12} {:>6}  {}",
            dev,
            fmt(size, human),
            fmt(used, human),
            fmt(avail, human),
            pct_str,
            mount
        );
    }
}

fn fmt(bytes: u64, human: bool) -> String {
    if !human {
        return bytes.to_string();
    }
    const K: u64 = 1024;
    if bytes < K {
        format!("{}B", bytes)
    } else if bytes < K * K {
        format!("{:.1}K", bytes as f64 / K as f64)
    } else if bytes < K * K * K {
        format!("{:.1}M", bytes as f64 / (K * K) as f64)
    } else {
        format!("{:.1}G", bytes as f64 / (K * K * K) as f64)
    }
}

fn statfs(path: &str) -> (u64, u64, u64) {
    // Parse /proc/mounts and use statvfs via the C library.
    #[repr(C)]
    struct Statvfs {
        f_bsize: u64,
        f_frsize: u64,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_favail: u64,
        f_fsid: u64,
        f_flag: u64,
        f_namemax: u64,
        _pad: [u8; 32],
    }
    extern "C" {
        fn statvfs(path: *const i8, buf: *mut Statvfs) -> i32;
    }
    let cpath = std::ffi::CString::new(path).unwrap_or_default();
    let mut buf = Statvfs {
        f_bsize: 0,
        f_frsize: 0,
        f_blocks: 0,
        f_bfree: 0,
        f_bavail: 0,
        f_files: 0,
        f_ffree: 0,
        f_favail: 0,
        f_fsid: 0,
        f_flag: 0,
        f_namemax: 0,
        _pad: [0; 32],
    };
    let ret = unsafe { statvfs(cpath.as_ptr(), &mut buf) };
    if ret != 0 {
        return (0, 0, 0);
    }
    let bs = buf.f_frsize;
    let size = buf.f_blocks * bs;
    let avail = buf.f_bavail * bs;
    let used = size.saturating_sub(buf.f_bfree * bs);
    (size, used, avail)
}
