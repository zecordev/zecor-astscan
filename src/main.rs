// SPDX-License-Identifier: Apache-2.0
//! `zecor-astscan` -- CLI wrapper. Fans files out over Rayon.
//!
//!   zecor-astscan check <FILE>...        -> [{file, language, ok, errors:[...]}]
//!                                            exit 1 if any file has a syntax error
//!   zecor-astscan symbols <FILE>         -> [{name, kind, start_byte, end_byte, row}]
//!   zecor-astscan metrics <FILE>...      -> [{file, language, functions:[{name, lines,
//!                                            complexity, max_depth, params}]}]
//!   zecor-astscan diff <OLD> <NEW>       -> [{change, kind, name, old_row, new_row}]
//!                                            (OLD/NEW are two versions of one file;
//!                                            language is taken from NEW's extension)
//!
//! With no file arguments, `check` / `metrics` read a NUL- or newline-separated list on
//! stdin.

use rayon::prelude::*;
use std::io::{self, Read};
use zecor_astscan::{check, metrics::file_metrics, structdiff::structural_diff, symbols};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => {
            let files = file_list(&args[1..]);
            let results: Vec<_> = files
                .par_iter()
                .map(|f| match std::fs::read_to_string(f) {
                    Ok(src) => check(f, &src),
                    Err(e) => zecor_astscan::CheckResult {
                        file: f.clone(),
                        language: None,
                        ok: false,
                        errors: vec![zecor_astscan::SyntaxError {
                            row: 0,
                            col: 0,
                            kind: "error".into(),
                            text: format!("cannot read: {e}"),
                        }],
                    },
                })
                .collect();
            let any_bad = results.iter().any(|r| !r.ok);
            println!("{}", serde_json::to_string(&results).unwrap());
            std::process::exit(if any_bad { 1 } else { 0 });
        }
        Some("symbols") => {
            let Some(f) = args.get(1) else {
                eprintln!("usage: zecor-astscan symbols <FILE>");
                std::process::exit(2);
            };
            let src = std::fs::read_to_string(f).unwrap_or_else(|e| {
                eprintln!("zecor-astscan: {f}: {e}");
                std::process::exit(2);
            });
            println!("{}", serde_json::to_string(&symbols(f, &src)).unwrap());
        }
        Some("metrics") => {
            let files = file_list(&args[1..]);
            let results: Vec<_> = files
                .par_iter()
                .map(|f| match std::fs::read_to_string(f) {
                    Ok(src) => file_metrics(f, &src),
                    Err(e) => {
                        eprintln!("zecor-astscan: {f}: {e}");
                        file_metrics(f, "")
                    }
                })
                .collect();
            println!("{}", serde_json::to_string(&results).unwrap());
        }
        Some("diff") => {
            let (Some(old), Some(new)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: zecor-astscan diff <OLD> <NEW>");
                std::process::exit(2);
            };
            let read = |p: &str| {
                std::fs::read_to_string(p).unwrap_or_else(|e| {
                    eprintln!("zecor-astscan: {p}: {e}");
                    std::process::exit(2);
                })
            };
            let changes = structural_diff(new, &read(old), &read(new));
            let any = !changes.is_empty();
            println!("{}", serde_json::to_string(&changes).unwrap());
            std::process::exit(if any { 1 } else { 0 });
        }
        _ => {
            eprintln!(
                "usage: zecor-astscan <check <FILE>... | symbols <FILE> | \
                 metrics <FILE>... | diff <OLD> <NEW>>"
            );
            std::process::exit(2);
        }
    }
}

fn file_list(args: &[String]) -> Vec<String> {
    if !args.is_empty() {
        return args.to_vec();
    }
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok();
    buf.split(['\0', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}
