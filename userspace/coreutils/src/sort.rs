//! sort — sort lines of text
use std::io::{BufRead, BufReader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut reverse = false;
    let mut numeric = false;
    let mut unique = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'r' => reverse = true,
                    'n' => numeric = true,
                    'u' => unique = true,
                    _ => {}
                }
            }
        } else {
            files.push(arg.clone());
        }
    }

    let mut lines: Vec<String> = Vec::new();
    let read = |reader: Box<dyn BufRead>| -> Vec<String> {
        reader.lines().filter_map(|l| l.ok()).collect()
    };

    if files.is_empty() {
        lines.extend(read(Box::new(BufReader::new(std::io::stdin()))));
    } else {
        for f in &files {
            match std::fs::File::open(f) {
                Ok(fh) => lines.extend(read(Box::new(BufReader::new(fh)))),
                Err(e) => eprintln!("sort: {}: {}", f, e),
            }
        }
    }

    lines.sort_by(|a, b| {
        if numeric {
            let na: f64 = a.parse().unwrap_or(f64::MAX);
            let nb: f64 = b.parse().unwrap_or(f64::MAX);
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.cmp(b)
        }
    });
    if reverse {
        lines.reverse();
    }
    if unique {
        lines.dedup();
    }
    for l in &lines {
        println!("{}", l);
    }
}
