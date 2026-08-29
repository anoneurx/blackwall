//! wc — word, line, character count
use std::io::{BufRead, BufReader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut count_lines = false;
    let mut count_words = false;
    let mut count_chars = false;
    let mut count_bytes = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'l' => count_lines = true,
                    'w' => count_words = true,
                    'm' => count_chars = true,
                    'c' => count_bytes = true,
                    _ => {}
                }
            }
        } else {
            files.push(arg.clone());
        }
    }

    // Default: lwc
    if !count_lines && !count_words && !count_chars && !count_bytes {
        count_lines = true;
        count_words = true;
        count_bytes = true;
    }

    let mut total_l = 0u64;
    let mut total_w = 0u64;
    let mut total_c = 0u64;

    let process = |reader: Box<dyn BufRead>, fname: Option<&str>| -> (u64, u64, u64) {
        let mut l = 0u64;
        let mut w = 0u64;
        let mut c = 0u64;
        for line in reader.lines().filter_map(|r| r.ok()) {
            l += 1;
            w += line.split_whitespace().count() as u64;
            c += line.len() as u64 + 1;
        }
        let mut out = String::new();
        if count_lines {
            out += &format!("{:8} ", l);
        }
        if count_words {
            out += &format!("{:8} ", w);
        }
        if count_bytes || count_chars {
            out += &format!("{:8} ", c);
        }
        if let Some(f) = fname {
            out += f;
        }
        println!("{}", out.trim_end());
        (l, w, c)
    };

    if files.is_empty() {
        process(Box::new(BufReader::new(std::io::stdin())), None);
    } else {
        for f in &files {
            match std::fs::File::open(f) {
                Ok(fh) => {
                    let (l, w, c) = process(Box::new(BufReader::new(fh)), Some(f));
                    total_l += l;
                    total_w += w;
                    total_c += c;
                }
                Err(e) => eprintln!("wc: {}: {}", f, e),
            }
        }
        if files.len() > 1 {
            let mut out = String::new();
            if count_lines {
                out += &format!("{:8} ", total_l);
            }
            if count_words {
                out += &format!("{:8} ", total_w);
            }
            if count_bytes || count_chars {
                out += &format!("{:8} ", total_c);
            }
            println!("{} total", out.trim_end());
        }
    }
}
