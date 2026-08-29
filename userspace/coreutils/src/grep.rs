//! grep — search file(s) for lines matching a pattern
use std::io::{BufRead, BufReader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut ignore_case = false;
    let mut invert = false;
    let mut line_nums = false;
    let mut count_only = false;
    let mut pattern: Option<String> = None;
    let mut files: Vec<String> = Vec::new();

    let mut skip = false;
    for (i, arg) in args.iter().skip(1).enumerate() {
        if skip {
            skip = false;
            continue;
        }
        if arg == "-e" || arg == "--regexp" {
            pattern = args.get(i + 2).cloned();
            skip = true;
        } else if arg.starts_with('-') && arg != "--" {
            for c in arg.chars().skip(1) {
                match c {
                    'i' => ignore_case = true,
                    'v' => invert = true,
                    'n' => line_nums = true,
                    'c' => count_only = true,
                    _ => {}
                }
            }
        } else if pattern.is_none() {
            pattern = Some(arg.clone());
        } else {
            files.push(arg.clone());
        }
    }

    let pat = match pattern {
        Some(p) => p,
        None => {
            eprintln!("grep: no pattern");
            std::process::exit(2);
        }
    };
    let pat_lower = pat.to_lowercase();
    let multi_file = files.len() > 1;
    let use_stdin = files.is_empty();
    let mut found = false;

    let process = |reader: Box<dyn BufRead>, fname: Option<&str>| -> bool {
        let mut matches = 0usize;
        for (ln, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let haystack = if ignore_case { line.to_lowercase() } else { line.clone() };
            let needle = if ignore_case { pat_lower.as_str() } else { pat.as_str() };
            let hit = haystack.contains(needle);
            if hit ^ invert {
                matches += 1;
                if !count_only {
                    let prefix = if let Some(f) = fname {
                        if multi_file {
                            format!("{}:", f)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    let lnum = if line_nums { format!("{}:", ln + 1) } else { String::new() };
                    println!("{}{}{}", prefix, lnum, line);
                }
            }
        }
        if count_only {
            println!("{}", matches);
        }
        matches > 0
    };

    if use_stdin {
        found = process(Box::new(BufReader::new(std::io::stdin())), None);
    } else {
        for f in &files {
            match std::fs::File::open(f) {
                Ok(fh) => {
                    if process(Box::new(BufReader::new(fh)), Some(f.as_str())) {
                        found = true;
                    }
                }
                Err(e) => eprintln!("grep: {}: {}", f, e),
            }
        }
    }
    std::process::exit(if found { 0 } else { 1 });
}
