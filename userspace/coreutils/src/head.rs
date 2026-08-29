//! head — output the first part of files
use std::io::{BufRead, BufReader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut lines = 10usize;
    let mut files: Vec<String> = Vec::new();
    let mut i = 1;

    while i < args.len() {
        if args[i] == "-n" {
            lines = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(10);
            i += 2;
            continue;
        }
        if let Some(n) = args[i].strip_prefix("-n") {
            lines = n.parse().unwrap_or(10);
        } else if let Some(n) = args[i].strip_prefix('-').and_then(|s| s.parse::<usize>().ok()) {
            lines = n;
        } else {
            files.push(args[i].clone());
        }
        i += 1;
    }

    let multi = files.len() > 1;
    if files.is_empty() {
        print_head(BufReader::new(std::io::stdin()), lines);
    } else {
        for f in &files {
            if multi {
                println!("==> {} <==", f);
            }
            match std::fs::File::open(f) {
                Ok(fh) => print_head(BufReader::new(fh), lines),
                Err(e) => eprintln!("head: {}: {}", f, e),
            }
        }
    }
}

fn print_head<R: BufRead>(reader: R, n: usize) {
    for (i, line) in reader.lines().enumerate() {
        if i >= n {
            break;
        }
        if let Ok(l) = line {
            println!("{}", l);
        }
    }
}
