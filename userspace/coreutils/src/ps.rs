//! ps — report process status (reads /proc)
use colored::Colorize;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let show_all =
        args.iter().any(|a| a == "-e" || a == "-A" || a.contains('e') || a.contains('a'));
    let long_fmt = args.iter().any(|a| a.contains('f') || a == "-l");

    let my_pid = std::process::id();

    let procs = read_procs();

    if long_fmt {
        println!(
            "{:>7} {:>7} {:>5}  {:<16} {}",
            "PID".bold(),
            "PPID".bold(),
            "%CPU".bold(),
            "CMD".bold(),
            "ARGS".bold()
        );
        println!("{}", "─".repeat(72).dimmed());
    } else {
        println!("{:>7}  {}", "PID".bold(), "CMD".bold());
        println!("{}", "─".repeat(40).dimmed());
    }

    for p in &procs {
        if !show_all && p.pid != my_pid {
            continue;
        }
        if long_fmt {
            println!("{:>7} {:>7} {:>5.1}  {:<16} {}", p.pid, p.ppid, p.cpu, p.comm, p.cmdline);
        } else {
            println!("{:>7}  {}", p.pid, p.comm);
        }
    }
}

struct ProcInfo {
    pid: u32,
    ppid: u32,
    comm: String,
    cmdline: String,
    cpu: f32,
}

fn read_procs() -> Vec<ProcInfo> {
    let mut procs = Vec::new();
    let rd = match std::fs::read_dir("/proc") {
        Ok(r) => r,
        Err(_) => return procs,
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        if let Ok(pid) = pid_str.parse::<u32>() {
            let base = format!("/proc/{}", pid);
            let comm = std::fs::read_to_string(format!("{}/comm", base))
                .unwrap_or_default()
                .trim()
                .to_string();
            let cmdline = std::fs::read_to_string(format!("{}/cmdline", base))
                .unwrap_or_default()
                .replace('\0', " ")
                .trim()
                .to_string();
            let stat = std::fs::read_to_string(format!("{}/stat", base)).unwrap_or_default();
            let ppid = stat.split_whitespace().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            procs.push(ProcInfo { pid, ppid, comm, cmdline, cpu: 0.0 });
        }
    }
    procs.sort_by_key(|p| p.pid);
    procs
}
