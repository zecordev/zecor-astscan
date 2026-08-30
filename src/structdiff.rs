// SPDX-License-Identifier: Apache-2.0
//! Structural diff: which top-level symbols were added, removed, or had their body
//! change, comparing two versions of one file. Whitespace-only edits are `unchanged`.
//! Cheaper for a model to read than a line diff when the question is "what moved".

use crate::{symbols, Symbol};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SymbolChange {
    pub change: String, // "added" | "removed" | "modified"
    pub kind: String,
    pub name: String,
    pub old_row: Option<usize>,
    pub new_row: Option<usize>,
}

fn norm(s: &str) -> String {
    // collapse every run of ASCII whitespace to one space, so reindent / reflow is a no-op
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in s.chars() {
        if ch.is_ascii_whitespace() {
            in_ws = true;
        } else {
            if in_ws && !out.is_empty() {
                out.push(' ');
            }
            in_ws = false;
            out.push(ch);
        }
    }
    out
}

fn index(file: &str, src: &str) -> BTreeMap<(String, String), (Symbol, String)> {
    symbols(file, src)
        .into_iter()
        .map(|s| {
            let body = norm(src.get(s.start_byte..s.end_byte).unwrap_or(""));
            ((s.kind.clone(), s.name.clone()), (s, body))
        })
        .collect()
}

/// `file` names the language (by extension) for both sides.
pub fn structural_diff(file: &str, old_src: &str, new_src: &str) -> Vec<SymbolChange> {
    let old = index(file, old_src);
    let new = index(file, new_src);
    let mut out = Vec::new();
    for (key, (sym, obody)) in &old {
        match new.get(key) {
            None => out.push(SymbolChange {
                change: "removed".into(),
                kind: key.0.clone(),
                name: key.1.clone(),
                old_row: Some(sym.row),
                new_row: None,
            }),
            Some((nsym, nbody)) if nbody != obody => out.push(SymbolChange {
                change: "modified".into(),
                kind: key.0.clone(),
                name: key.1.clone(),
                old_row: Some(sym.row),
                new_row: Some(nsym.row),
            }),
            Some(_) => {}
        }
    }
    for (key, (sym, _)) in &new {
        if !old.contains_key(key) {
            out.push(SymbolChange {
                change: "added".into(),
                kind: key.0.clone(),
                name: key.1.clone(),
                old_row: None,
                new_row: Some(sym.row),
            });
        }
    }
    out.sort_by(|a, b| (a.new_row.or(a.old_row), &a.name).cmp(&(b.new_row.or(b.old_row), &b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_add_remove_modify_and_ignores_reformat() {
        let old = "def a():\n    return 1\n\ndef b():\n    return 2\n\ndef c():\n    return 3\n";
        let new =
            "def a():\n        return 1\n\ndef b():\n    return 22\n\ndef d():\n    return 4\n";
        let ch = structural_diff("x.py", old, new);
        let by_name: Vec<_> = ch
            .iter()
            .map(|c| (c.name.as_str(), c.change.as_str()))
            .collect();
        assert!(by_name.contains(&("b", "modified")));
        assert!(by_name.contains(&("c", "removed")));
        assert!(by_name.contains(&("d", "added")));
        // `a` only got reindented -> not reported
        assert!(!by_name.iter().any(|(n, _)| *n == "a"));
    }

    #[test]
    fn empty_when_identical() {
        let s = "fn f() {}\nstruct S;\n";
        assert!(structural_diff("x.rs", s, s).is_empty());
    }
}
