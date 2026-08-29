//! kill — send signal to process
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("kill: usage: kill [-SIGNAL] PID...");
        std::process::exit(1);
    }

    let mut sig: i32 = 15; // SIGTERM
    let mut pids: Vec<i32> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            let s = arg.trim_start_matches('-');
            sig = match s.to_uppercase().as_str() {
                "9" | "KILL" => 9,
                "15" | "TERM" => 15,
                "1" | "HUP" => 1,
                "2" | "INT" => 2,
                "3" | "QUIT" => 3,
                "19" | "STOP" => 19,
                _ => s.parse().unwrap_or(15),
            };
        } else {
            if let Ok(pid) = arg.parse::<i32>() {
                pids.push(pid);
            }
        }
    }

    for pid in &pids {
        let ret = libc_kill(*pid, sig);
        if ret != 0 {
            eprintln!("kill: ({}) - No such process", pid);
        }
    }
}

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
fn libc_kill(pid: i32, sig: i32) -> i32 {
    unsafe { kill(pid, sig) }
}
