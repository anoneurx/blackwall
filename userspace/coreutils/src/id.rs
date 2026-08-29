//! id — print real and effective user and group IDs
fn main() {
    // Use the system id command's data via /proc/self/status.
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let uid = parse_id_field(&status, "Uid:").unwrap_or(0);
    let gid = parse_id_field(&status, "Gid:").unwrap_or(0);
    let user = uid_to_name(uid).unwrap_or_else(|| uid.to_string());
    let group = gid_to_name(gid).unwrap_or_else(|| gid.to_string());
    println!("uid={}({}) gid={}({}) groups={}({})", uid, user, gid, group, gid, group);
}

fn parse_id_field(status: &str, field: &str) -> Option<u32> {
    status
        .lines()
        .find(|l| l.starts_with(field))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
}

fn uid_to_name(uid: u32) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd
        .lines()
        .find(|l| l.split(':').nth(2).and_then(|u| u.parse::<u32>().ok()) == Some(uid))
        .and_then(|l| l.split(':').next().map(|s| s.to_string()))
}

fn gid_to_name(gid: u32) -> Option<String> {
    let group = std::fs::read_to_string("/etc/group").ok()?;
    group
        .lines()
        .find(|l| l.split(':').nth(2).and_then(|g| g.parse::<u32>().ok()) == Some(gid))
        .and_then(|l| l.split(':').next().map(|s| s.to_string()))
}
