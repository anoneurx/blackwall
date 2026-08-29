//! whoami — print current user name
fn main() {
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| {
        // Try reading from /proc/self/status.
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(1).map(|u| u.to_string()))
            })
            .unwrap_or_else(|| "root".to_string())
    });
    println!("{}", user);
}
