//! uname — print system information
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut all = false;
    let mut kernel = false;
    let mut node = false;
    let mut release = false;
    let mut machine = false;

    for arg in args.iter().skip(1) {
        for c in arg.trim_start_matches('-').chars() {
            match c {
                'a' => all = true,
                's' => kernel = true,
                'n' => node = true,
                'r' => release = true,
                'm' => machine = true,
                _ => {}
            }
        }
    }

    if !all && !kernel && !node && !release && !machine {
        kernel = true; // default: -s
    }

    let hostname =
        std::fs::read_to_string("/etc/hostname").unwrap_or_else(|_| "blackwall\n".to_string());
    let hostname = hostname.trim().to_string();

    let mut parts = Vec::new();
    if all || kernel {
        parts.push("Linux");
    }
    if all || node {
        parts.push(hostname.as_str());
    }
    if all || release {
        parts.push("6.0.0-blackwall");
    }
    if all {
        parts.push("Black Wall Kernel");
    }
    if all || machine {
        parts.push("x86_64");
    }
    println!("{}", parts.join(" "));
}
