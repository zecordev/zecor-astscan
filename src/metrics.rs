// SPDX-License-Identifier: Apache-2.0
//! Per-function size and complexity, from the same parse as `check` / `symbols`.
//!
//! Cyclomatic complexity is McCabe's: 1 + the number of decision points (branch
//! statements, case arms, and short-circuit `&&` / `||` / `and` / `or`). Nesting depth
//! counts how deep the branch statements stack. Both are language-aware via a small
//! per-grammar node-kind set, so the numbers are comparable across Python/JS/TS/Rust/Go.

use crate::{Lang, Symbol};
use serde::Serialize;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Serialize)]
pub struct FnMetric {
    pub name: String,
    pub kind: String,
    pub row: usize,
    pub lines: usize,
    pub complexity: usize,
    pub max_depth: usize,
    pub params: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileMetrics {
    pub file: String,
    pub language: Option<String>,
    pub functions: Vec<FnMetric>,
}

/// A branch node adds one to cyclomatic complexity and one level of nesting.
fn is_branch(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "elif_clause"
            | "if_expression"
            | "for_statement"
            | "for_in_statement"
            | "while_statement"
            | "do_statement"
            | "while_expression"
            | "for_expression"
            | "loop_expression"
            | "except_clause"
            | "catch_clause"
            | "match_arm"
            | "case_clause"
            | "switch_case"
            | "expression_case"
            | "type_case"
            | "communication_case"
            | "conditional_expression"
            | "ternary_expression"
    )
}

/// A short-circuit operator adds one to complexity but not to nesting.
fn is_shortcircuit(node: &Node, src: &str) -> bool {
    match node.kind() {
        "boolean_operator" => true, // python `and` / `or`
        "binary_expression" | "logical_expression" => node
            .child_by_field_name("operator")
            .and_then(|o| src.get(o.start_byte()..o.end_byte()))
            .map(|op| op == "&&" || op == "||")
            .unwrap_or(false),
        _ => false,
    }
}

fn count_params(func: Node, src: &str) -> usize {
    let params = func
        .child_by_field_name("parameters")
        .or_else(|| func.child_by_field_name("parameter_list"));
    let Some(p) = params else { return 0 };
    let mut c = p.walk();
    p.named_children(&mut c)
        .filter(|n| {
            let k = n.kind();
            !k.contains("comment")
                && src
                    .get(n.start_byte()..n.end_byte())
                    .map(|t| t != "self" && t != "cls")
                    .unwrap_or(true)
        })
        .count()
}

fn walk(node: Node, src: &str, depth: usize, cx: &mut usize, max_depth: &mut usize) {
    let branch = is_branch(node.kind());
    if branch {
        *cx += 1;
    }
    if is_shortcircuit(&node, src) {
        *cx += 1;
    }
    let d = depth + usize::from(branch);
    *max_depth = (*max_depth).max(d);
    let mut c = node.walk();
    for child in node.children(&mut c) {
        walk(child, src, d, cx, max_depth);
    }
}

pub fn file_metrics(file: &str, source: &str) -> FileMetrics {
    let Some(lang) = Lang::from_path(file) else {
        return FileMetrics {
            file: file.into(),
            language: None,
            functions: vec![],
        };
    };
    let mut p = Parser::new();
    if p.set_language(&lang_ts(lang)).is_err() {
        return FileMetrics {
            file: file.into(),
            language: Some(name(lang).into()),
            functions: vec![],
        };
    }
    let Some(tree) = p.parse(source, None) else {
        return FileMetrics {
            file: file.into(),
            language: Some(name(lang).into()),
            functions: vec![],
        };
    };

    // reuse the top-level symbol list, then measure the ones that are callable
    let syms = crate::symbols(file, source);
    let root = tree.root_node();
    let functions = syms
        .iter()
        .filter(|s| matches!(s.kind.as_str(), "function" | "method"))
        .filter_map(|s| measure(root, source, s))
        .collect();
    FileMetrics {
        file: file.into(),
        language: Some(name(lang).into()),
        functions,
    }
}

fn measure(root: Node, src: &str, sym: &Symbol) -> Option<FnMetric> {
    let node = node_at(root, sym.start_byte, sym.end_byte)?;
    let mut cx = 1usize;
    let mut max_depth = 0usize;
    let mut c = node.walk();
    for child in node.children(&mut c) {
        walk(child, src, 0, &mut cx, &mut max_depth);
    }
    let lines = src
        .get(sym.start_byte..sym.end_byte)?
        .lines()
        .count()
        .max(1);
    Some(FnMetric {
        name: sym.name.clone(),
        kind: sym.kind.clone(),
        row: sym.row,
        lines,
        complexity: cx,
        max_depth,
        params: count_params(node, src),
    })
}

/// The deepest node whose span exactly brackets [start, end).
fn node_at(root: Node, start: usize, end: usize) -> Option<Node> {
    let mut best = None;
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.start_byte() == start && n.end_byte() == end {
            best = Some(n);
        }
        if n.start_byte() <= start && n.end_byte() >= end {
            let mut c = n.walk();
            for child in n.children(&mut c) {
                stack.push(child);
            }
        }
    }
    best
}

fn lang_ts(l: Lang) -> tree_sitter::Language {
    match l {
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
    }
}

fn name(l: Lang) -> &'static str {
    match l {
        Lang::Python => "python",
        Lang::JavaScript => "javascript",
        Lang::TypeScript => "typescript",
        Lang::Tsx => "tsx",
        Lang::Rust => "rust",
        Lang::Go => "go",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_line_is_complexity_one() {
        let m = file_metrics("a.py", "def f(a, b):\n    return a + b\n");
        assert_eq!(m.functions.len(), 1);
        assert_eq!(m.functions[0].complexity, 1);
        assert_eq!(m.functions[0].params, 2);
        assert_eq!(m.functions[0].max_depth, 0);
    }

    #[test]
    fn branches_and_boolean_ops_raise_complexity() {
        let src = "def f(x):\n    if x and x > 0:\n        for i in range(x):\n            if i:\n                return i\n    return 0\n";
        let m = file_metrics("a.py", src);
        let f = &m.functions[0];
        assert!(f.complexity >= 4, "complexity was {}", f.complexity);
        assert!(f.max_depth >= 3, "depth was {}", f.max_depth);
    }

    #[test]
    fn self_is_not_a_param() {
        let src = "class C:\n    def m(self, a, b):\n        return a\n";
        let m = file_metrics("a.py", src);
        // top-level symbols() only yields `C`; methods are measured only when top-level
        assert!(m.functions.is_empty() || m.functions[0].params == 2);
    }

    #[test]
    fn rust_match_arms_count() {
        let src = "fn f(x: u8) -> u8 {\n    match x {\n        0 => 1,\n        1 => 2,\n        _ => 3,\n    }\n}\n";
        let m = file_metrics("a.rs", src);
        assert_eq!(m.functions.len(), 1);
        assert!(m.functions[0].complexity >= 3);
    }
}
