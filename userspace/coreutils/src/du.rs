//! du — estimate file space usage
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut human = false;
    let mut summarize = false;
    let mut maxdepth: Option<usize> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--human-readable" => human = true,
            "-s" | "--summarize" => summarize = true,
            "-d" | "--max-depth" => {
                maxdepth = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 1;
            }
            a if a.starts_with("-d") => maxdepth = a[2..].parse().ok(),
            a if !a.starts_with('-') => paths.push(a.to_string()),
            _ => {}
        }
        i += 1;
    }

    if paths.is_empty() {
        paths.push(".".to_string());
    }

    for path in &paths {
        let size = dir_size(
            std::path::Path::new(path),
            0,
            if summarize { Some(0) } else { maxdepth },
            human,
        );
        println!("{}\t{}", fmt(size, human), path);
    }
}

fn dir_size(path: &std::path::Path, depth: usize, maxdepth: Option<usize>, human: bool) -> u64 {
    let mut total = 0u64;
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.filter_map(|e| e.ok()) {
            let p = entry.path();
            let size = if p.is_dir() {
                dir_size(&p, depth + 1, maxdepth, human)
            } else {
                p.metadata().map(|m| m.len()).unwrap_or(0)
            };
            total += size;
            let show = maxdepth.map_or(true, |md| depth < md);
            if p.is_dir() && show {
                println!("{}\t{}", fmt(size, human), p.display());
            }
        }
    }
    total
}

fn fmt(bytes: u64, human: bool) -> String {
    const K: u64 = 1024;
    if !human {
        return (bytes / K).to_string();
    } // du shows KiB by default
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
