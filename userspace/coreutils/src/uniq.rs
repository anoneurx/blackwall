//! uniq — report or filter out repeated lines
use std::io::{BufRead, BufReader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut count = false;
    let mut dup = false;
    let mut unique = false;
    let mut ignore_case = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'c' => count = true,
                    'd' => dup = true,
                    'u' => unique = true,
                    'i' => ignore_case = true,
                    _ => {}
                }
            }
        } else {
            files.push(arg.clone());
        }
    }

    let read = |reader: Box<dyn BufRead>| {
        let mut prev: Option<String> = None;
        let mut n = 0usize;

        let flush = |line: &str, n: usize| {
            let should_print = if dup {
                n > 1
            } else if unique {
                n == 1
            } else {
                true
            };
            if should_print {
                if count {
                    println!("{:7} {}", n, line);
                } else {
                    println!("{}", line);
                }
            }
        };

        for line in reader.lines().filter_map(|l| l.ok()) {
            let key = if ignore_case { line.to_lowercase() } else { line.clone() };
            let prev_key =
                prev.as_ref().map(|p| if ignore_case { p.to_lowercase() } else { p.clone() });

            if Some(&key) == prev_key.as_ref() {
                n += 1;
            } else {
                if let Some(ref p) = prev {
                    flush(p, n);
                }
                prev = Some(line.clone());
                n = 1;
            }
        }
        if let Some(ref p) = prev {
            flush(p, n);
        }
    };

    if files.is_empty() {
        read(Box::new(BufReader::new(std::io::stdin())));
    } else {
        for f in &files {
            match std::fs::File::open(f) {
                Ok(fh) => read(Box::new(BufReader::new(fh))),
                Err(e) => eprintln!("uniq: {}: {}", f, e),
            }
        }
    }
}
