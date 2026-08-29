//! find — search for files in a directory hierarchy
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut roots: Vec<String> = Vec::new();
    let mut name_filter: Option<String> = None;
    let mut type_filter: Option<char> = None;
    let mut maxdepth: Option<usize> = None;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "-name" => {
                name_filter = args.get(i + 1).cloned();
                i += 1;
            }
            "-type" => {
                type_filter = args.get(i + 1).and_then(|s| s.chars().next());
                i += 1;
            }
            "-maxdepth" => {
                maxdepth = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 1;
            }
            a if !a.starts_with('-') => roots.push(a.to_string()),
            _ => {}
        }
        i += 1;
    }

    if roots.is_empty() {
        roots.push(".".to_string());
    }

    for root in &roots {
        walk(&PathBuf::from(root), 0, maxdepth, &name_filter, type_filter);
    }
}

fn walk(
    path: &PathBuf,
    depth: usize,
    maxdepth: Option<usize>,
    name: &Option<String>,
    typ: Option<char>,
) {
    if let Some(md) = maxdepth {
        if depth > md {
            return;
        }
    }

    let meta = match path.symlink_metadata() {
        Ok(m) => m,
        Err(_) => return,
    };

    let is_dir = meta.is_dir();
    let is_file = meta.is_file();
    let is_link = meta.file_type().is_symlink();

    let type_match = match typ {
        Some('f') => is_file,
        Some('d') => is_dir,
        Some('l') => is_link,
        None => true,
        _ => true,
    };

    let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let name_match = match name {
        Some(pat) => glob_match(pat, &file_name),
        None => true,
    };

    if type_match && name_match {
        println!("{}", path.display());
    }

    if is_dir {
        if let Ok(rd) = std::fs::read_dir(path) {
            let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                walk(&entry.path(), depth + 1, maxdepth, name, typ);
            }
        }
    }
}

/// Very basic glob: supports `*` and `?` wildcards.
fn glob_match(pat: &str, s: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    let s: Vec<char> = s.chars().collect();
    glob_rec(&pat, &s)
}

fn glob_rec(pat: &[char], s: &[char]) -> bool {
    match (pat.first(), s.first()) {
        (None, None) => true,
        (Some(&'*'), _) => glob_rec(&pat[1..], s) || (!s.is_empty() && glob_rec(pat, &s[1..])),
        (Some(&'?'), Some(_)) => glob_rec(&pat[1..], &s[1..]),
        (Some(p), Some(c)) => p == c && glob_rec(&pat[1..], &s[1..]),
        _ => false,
    }
}
