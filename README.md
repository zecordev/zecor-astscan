# zecor-astscan

Multi-language syntax checking and top-level symbol extraction via Tree-sitter.

Part of [Zecor](https://zecor.dev) -- an autonomous software construction engine.
Apache-2.0. Prebuilt binaries for Linux / macOS / Windows are attached to each
[release](https://github.com/zecordev/zecor-astscan/releases); or `cargo install zecor-astscan`.

## 1. `zecor-astscan` — structural code analysis

**Incumbents.** `tree-sitter` CLI (parse only, no analysis), `ast-grep` (pattern
search/rewrite, not metrics or diff), `semgrep` (rule engine, JVM/Python startup cost,
cloud-tilted), `scc`/`tokei` (line counts, no structure), language-specific tools
(`ruff`, `eslint`) that do not compose across a polyglot repo.

**Gaps people hit.** No single fast tool that, across many languages at once: (a) tells
you a generated file will not parse *before* you run the suite, (b) gives a *structural*
diff (which symbols changed, not which lines), (c) exports a call graph for
change-impact analysis, (d) reports complexity so a reviewer knows where to look. Every
LLM-coding harness reimplements a weak version of (a).

**Shipped.** `check` (Tree-sitter `ERROR`/`MISSING` nodes with line/col, Rayon
fan-out, one report per broken region) and `symbols` (top-level defs across
Python/JS/TS/TSX/Rust/Go with byte spans). **`metrics <file>...`** — per top-level
function: line count, McCabe cyclomatic complexity (branch nodes + short-circuit
`&&`/`||`/`and`/`or`, language-aware node set), max nesting depth, arity (`self`/`cls`
not counted). **`diff <old> <new>`** — structural change set (`added` / `removed` /
`modified`) keyed by `kind:name`, whitespace-normalised so a reindent is not a change.
`zecor.selfreview` now raises an advisory `complexity` finding when a touched function
lands past the caps (complexity 20 / 120 lines / depth 5, all env-overridable).

**Still to world-class.**
- **`callgraph <files>`** — intra-repo caller→callee edges (resolved by name +
  same-file scope; cross-file by import), emitted as JSON adjacency + optional DOT.
- **Signature-vs-body** distinction for a `modified` symbol in `diff`; `moved` detection.
- **Cognitive complexity** (Sonar-style) and fan-in/fan-out alongside cyclomatic.
- **Incremental parsing** — persist Tree-sitter trees keyed by content hash under
  `target/.astcache`; re-parse only changed files. Sub-millisecond on a warm cache.
- **Query pass-through** — accept a Tree-sitter query (`.scm`) and return captures, so
  a repo can add its own structural lints without a new binary.
- **More grammars** — Java, C/C++, C#, Ruby, PHP, Kotlin, Swift, Bash, SQL, HCL.
- **Editor-grade errors** — recovery hints ("expected `)` to match line N").

## Build

```
cargo build --release      # -> target/release/zecor-astscan
cargo test --all-targets
```
