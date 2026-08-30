// SPDX-License-Identifier: Apache-2.0
//! Structural code analysis over a whole change set, before it reaches a model.
//!
//! * `check` -- parse each file and report syntax errors (Tree-sitter `ERROR` /
//!   `MISSING` nodes). Catches obviously-broken model output for the cost of a parse.
//! * `symbols` -- top-level definitions (functions, classes/structs/impls, top-level
//!   consts) with byte spans. Generalises the Python-only scope-creep check to
//!   JS/TS/Rust/Go.
//!
//! Parsing is per-file and embarrassingly parallel; the CLI fans out with Rayon.

use serde::Serialize;
use tree_sitter::{Language, Node, Parser};

pub mod metrics;
pub mod structdiff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Rust,
    Go,
}

impl Lang {
    pub fn from_path(path: &str) -> Option<Lang> {
        let ext = path.rsplit('.').next()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "py" | "pyi" => Lang::Python,
            "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "rs" => Lang::Rust,
            "go" => Lang::Go,
            _ => return None,
        })
    }

    fn ts_language(self) -> Language {
        match self {
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyntaxError {
    pub row: usize,
    pub col: usize,
    pub kind: String, // "error" | "missing"
    pub text: String, // a short excerpt of the offending node
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub file: String,
    pub language: Option<String>,
    pub ok: bool,
    pub errors: Vec<SyntaxError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub row: usize,
}

fn parse(source: &str, lang: Lang) -> Option<tree_sitter::Tree> {
    let mut p = Parser::new();
    p.set_language(&lang.ts_language()).ok()?;
    p.parse(source, None)
}

fn lang_name(l: Lang) -> &'static str {
    match l {
        Lang::Python => "python",
        Lang::JavaScript => "javascript",
        Lang::TypeScript => "typescript",
        Lang::Tsx => "tsx",
        Lang::Rust => "rust",
        Lang::Go => "go",
    }
}

/// Parse `source` and collect every `ERROR` / `MISSING` node.
pub fn check(file: &str, source: &str) -> CheckResult {
    let Some(lang) = Lang::from_path(file) else {
        return CheckResult {
            file: file.into(),
            language: None,
            ok: true,
            errors: vec![],
        };
    };
    let Some(tree) = parse(source, lang) else {
        return CheckResult {
            file: file.into(),
            language: Some(lang_name(lang).into()),
            ok: false,
            errors: vec![SyntaxError {
                row: 0,
                col: 0,
                kind: "error".into(),
                text: "parser failed".into(),
            }],
        };
    };
    let mut errors = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.is_error() || n.is_missing() {
            let start = n.start_position();
            let excerpt: String = source[n.start_byte()..n.end_byte().min(n.start_byte() + 60)]
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            errors.push(SyntaxError {
                row: start.row + 1,
                col: start.column + 1,
                kind: if n.is_missing() {
                    "missing".into()
                } else {
                    "error".into()
                },
                text: if excerpt.is_empty() {
                    format!("missing `{}`", n.kind())
                } else {
                    excerpt
                },
            });
            // do not recurse into an error subtree -- one report per broken region
            continue;
        }
        if n.has_error() {
            let mut c = n.walk();
            for child in n.children(&mut c) {
                stack.push(child);
            }
        }
    }
    CheckResult {
        file: file.into(),
        language: Some(lang_name(lang).into()),
        ok: errors.is_empty(),
        errors,
    }
}

/// Top-level definitions, in source order.
pub fn symbols(file: &str, source: &str) -> Vec<Symbol> {
    let Some(lang) = Lang::from_path(file) else {
        return vec![];
    };
    let Some(tree) = parse(source, lang) else {
        return vec![];
    };
    let root = tree.root_node();
    let mut out = Vec::new();
    let mut c = root.walk();
    for node in root.children(&mut c) {
        // JS/TS wrap a top-level def in `export_statement`; Rust `mod_item` nests a
        // `declaration_list`. Descend one level in those cases.
        match node.kind() {
            "export_statement" => {
                let mut cc = node.walk();
                for inner in node.children(&mut cc) {
                    collect_symbol(inner, source, lang, &mut out);
                }
            }
            "mod_item" => {
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cc = body.walk();
                    for inner in body.children(&mut cc) {
                        collect_symbol(inner, source, lang, &mut out);
                    }
                }
            }
            _ => collect_symbol(node, source, lang, &mut out),
        }
    }
    out
}

fn collect_symbol(node: Node, source: &str, lang: Lang, out: &mut Vec<Symbol>) {
    let kind = match (lang, node.kind()) {
        (Lang::Python, "function_definition") => "function",
        (Lang::Python, "class_definition") => "class",
        (Lang::Python, "decorated_definition") => {
            if let Some(inner) = node.child_by_field_name("definition") {
                collect_symbol(inner, source, lang, out);
            }
            return;
        }
        (_, "function_declaration") => "function",
        (_, "generator_function_declaration") => "function",
        (_, "class_declaration") => "class",
        (Lang::TypeScript | Lang::Tsx, "interface_declaration") => "interface",
        (Lang::TypeScript | Lang::Tsx, "type_alias_declaration") => "type",
        (Lang::Rust, "function_item") => "function",
        (Lang::Rust, "struct_item") => "struct",
        (Lang::Rust, "enum_item") => "enum",
        (Lang::Rust, "trait_item") => "trait",
        (Lang::Rust, "impl_item") => "impl",
        (Lang::Rust, "const_item" | "static_item") => "const",
        (Lang::Go, "method_declaration") => "method",
        (Lang::Go, "type_declaration") => "type",
        _ => return,
    };
    let name = node
        .child_by_field_name("name")
        .and_then(|n| source.get(n.start_byte()..n.end_byte()))
        .unwrap_or("<anon>")
        .to_string();
    out.push(Symbol {
        name,
        kind: kind.into(),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        row: node.start_position().row + 1,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_python_has_no_errors() {
        let r = check("a.py", "def f(x):\n    return x + 1\n");
        assert!(r.ok && r.errors.is_empty());
    }

    #[test]
    fn broken_python_is_flagged_with_a_line() {
        let r = check("a.py", "def f(:\n    return\n");
        assert!(!r.ok);
        assert_eq!(r.errors[0].row, 1);
    }

    #[test]
    fn python_top_level_symbols() {
        let src =
            "import os\n\ndef a():\n    pass\n\nclass B:\n    def m(self): pass\n\nCONST = 1\n";
        let names: Vec<_> = symbols("x.py", src).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["a", "B"]);
    }

    #[test]
    fn typescript_symbols_and_errors() {
        let ok = check(
            "x.ts",
            "export function f(a: number): number { return a; }\n",
        );
        assert!(ok.ok);
        let bad = check("x.ts", "function f( { return\n");
        assert!(!bad.ok);
        let syms: Vec<_> = symbols(
            "x.ts",
            "export function f() {}\nclass C {}\ninterface I {}\n",
        )
        .into_iter()
        .map(|s| (s.kind, s.name))
        .collect();
        assert!(syms.contains(&("function".into(), "f".into())));
        assert!(syms.contains(&("interface".into(), "I".into())));
    }

    #[test]
    fn rust_symbols() {
        let syms: Vec<_> = symbols(
            "x.rs",
            "pub fn a() {}\nstruct S;\nimpl S { fn m(&self) {} }\nconst K: u8 = 1;\n",
        )
        .into_iter()
        .map(|s| s.kind)
        .collect();
        assert!(syms.contains(&"function".to_string()));
        assert!(syms.contains(&"struct".to_string()));
        assert!(syms.contains(&"const".to_string()));
    }
}
