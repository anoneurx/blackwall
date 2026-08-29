//! cat — concatenate and print files

use std::io::{self, BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut show_ends = false; // -E
    let mut number_lines = false; // -n
    let mut squeeze = false; // -s

    let mut files: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "-E" | "--show-ends" => show_ends = true,
            "-n" | "--number" => number_lines = true,
            "-s" | "--squeeze-blank" => squeeze = true,
            "-A" => {
                show_ends = true;
            }
            a if a.starts_with('-') && a.len() > 1 => {
                for c in a.chars().skip(1) {
                    match c {
                        'E' => show_ends = true,
                        'n' => number_lines = true,
                        's' => squeeze = true,
                        _ => eprintln!("cat: invalid option -- '{}'", c),
                    }
                }
            }
            _ => files.push(arg.clone()),
        }
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if files.is_empty() {
        // Read from stdin.
        cat_reader(io::stdin().lock(), &mut out, show_ends, number_lines, squeeze, &mut 1);
    } else {
        let mut line_num = 1usize;
        for file in &files {
            if file == "-" {
                cat_reader(
                    io::stdin().lock(),
                    &mut out,
                    show_ends,
                    number_lines,
                    squeeze,
                    &mut line_num,
                );
            } else {
                match std::fs::File::open(file) {
                    Ok(f) => {
                        cat_reader(
                            io::BufReader::new(f),
                            &mut out,
                            show_ends,
                            number_lines,
                            squeeze,
                            &mut line_num,
                        );
                    }
                    Err(e) => eprintln!("cat: {}: {}", file, e),
                }
            }
        }
    }
}

fn cat_reader<R: BufRead>(
    reader: R,
    out: &mut impl Write,
    show_ends: bool,
    number: bool,
    squeeze: bool,
    line_num: &mut usize,
) {
    let mut prev_blank = false;
    for line in reader.lines() {
        match line {
            Ok(l) => {
                let blank = l.trim().is_empty();
                if squeeze && blank && prev_blank {
                    continue;
                }
                prev_blank = blank;

                if number {
                    let _ = write!(out, "{:6}  ", line_num);
                    *line_num += 1;
                }
                if show_ends {
                    let _ = writeln!(out, "{}$", l);
                } else {
                    let _ = writeln!(out, "{}", l);
                }
            }
            Err(e) => eprintln!("cat: read error: {}", e),
        }
    }
}
