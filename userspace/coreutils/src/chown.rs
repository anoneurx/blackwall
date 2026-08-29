//! chown — change file owner and group
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut recursive = false;
    let mut operands: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg == "-R" || arg == "--recursive" {
            recursive = true;
        } else {
            operands.push(arg.clone());
        }
    }

    if operands.len() < 2 {
        eprintln!("chown: usage: chown [-R] OWNER[:GROUP] FILE...");
        std::process::exit(1);
    }

    let owner_spec = operands[0].clone();
    let (uid, gid) = parse_owner(&owner_spec);

    for file in &operands[1..] {
        if let Err(e) = apply_chown(file, uid, gid, recursive) {
            eprintln!("chown: {}: {}", file, e);
        }
    }
}

fn parse_owner(spec: &str) -> (u32, Option<u32>) {
    if let Some((u, g)) = spec.split_once(':') {
        let uid = resolve_user(u).unwrap_or(0);
        let gid = resolve_group(g);
        (uid, gid)
    } else {
        (resolve_user(spec).unwrap_or(0), None)
    }
}

fn resolve_user(name: &str) -> Option<u32> {
    if let Ok(n) = name.parse() {
        return Some(n);
    }
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd
        .lines()
        .find(|l| l.split(':').next() == Some(name))
        .and_then(|l| l.split(':').nth(2)?.parse().ok())
}

fn resolve_group(name: &str) -> Option<u32> {
    if let Ok(n) = name.parse() {
        return Some(n);
    }
    let group = std::fs::read_to_string("/etc/group").ok()?;
    group
        .lines()
        .find(|l| l.split(':').next() == Some(name))
        .and_then(|l| l.split(':').nth(2)?.parse().ok())
}

fn apply_chown(path: &str, uid: u32, gid: Option<u32>, recursive: bool) -> std::io::Result<()> {
    extern "C" {
        fn lchown(path: *const i8, owner: u32, group: u32) -> i32;
    }
    let cpath = std::ffi::CString::new(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let g = gid.unwrap_or(u32::MAX); // u32::MAX = -1 in C = "don't change"
    let ret = unsafe { lchown(cpath.as_ptr(), uid, g) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let p = std::path::Path::new(path);
    if recursive && p.is_dir() {
        for entry in std::fs::read_dir(p)?.filter_map(|e| e.ok()) {
            apply_chown(&entry.path().display().to_string(), uid, gid, recursive)?;
        }
    }
    Ok(())
}
