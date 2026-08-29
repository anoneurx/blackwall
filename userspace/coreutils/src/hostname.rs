//! hostname — get or set the system hostname
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        // Set hostname via /etc/hostname + sethostname syscall via `hostname` cmd.
        let name = &args[1];
        std::fs::write("/etc/hostname", format!("{}\n", name)).ok();
        // Use nix sethostname via /proc or fall back to command.
        let _ = std::process::Command::new("hostname").arg(name).status();
    } else {
        match std::fs::read_to_string("/etc/hostname") {
            Ok(h) => print!("{}", h.trim()),
            Err(_) => {
                // Try uname syscall via std.
                let out = std::process::Command::new("uname").arg("-n").output();
                if let Ok(o) = out {
                    print!("{}", String::from_utf8_lossy(&o.stdout).trim());
                }
            }
        }
        println!();
    }
}
