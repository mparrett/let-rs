# Key Facts

## Toolchain

- Rust 1.93+, edition 2024
- Workspace at repo root, members = `["crates/*"]`, resolver = `"3"`
- `just` is the task runner (Homebrew install). No `Makefile`.
- Core `lisp` crate: zero dependencies — keep it that way.
- `runes`, `codons`, `world`, `genes` crates: depend on nothing (`runes`, `codons`) or only on `lisp` (`world`, `genes`).
- `spells` crate: depends on `lisp` + `world` (the `install_with_world` helper combines both installs).
- `wasm` crate: `wasm-bindgen` (=0.2.114 pinned to match CLI), `console_error_panic_hook`. Justified by ADR-002's "lisp stays platform-independent" caveat.

## WASM toolchain

- `wasm32-unknown-unknown` Rust target (`rustup target add wasm32-unknown-unknown`)
- `wasm-bindgen-cli` 0.2.114 (`cargo install -f wasm-bindgen-cli --version 0.2.114`)
- `binaryen` 129+ for `wasm-opt` (`brew install binaryen`)
- Plain `python3 -m http.server` for serving — **no COI / SAB / service-worker shim required** (the whole reason this exists; ADR-009)

Pipeline: `cargo build` → `wasm-bindgen --target web` → `wasm-opt -Oz --strip-debug --strip-producers`.

## Commands

```bash
just              # default → cargo test --workspace (97 tests)
just test         # same — explicit
just repl         # interactive REPL (examples/repl.rs)
just spells       # CLI rune-tape demo
just world        # CLI spell-paints-tiles demo
just genes        # CLI codon-tape → creature card demo
just check        # cargo check --all-targets
just fmt          # cargo fmt --all
just wasm-build   # cargo build wasm + wasm-bindgen → web/pkg/
just wasm-serve   # wasm-build + python3 -m http.server -d web 8000
just bench        # criterion benches under crates/bench/
```

## Layout

```
letrs/
├── Cargo.toml                 workspace root, members = ["crates/*"]
├── CLAUDE.md                  session-orientation
├── justfile
├── crates/
│   ├── lisp/                  the engine — zero deps, no host types
│   │   ├── src/{expr,val,env,k,step,prim,parse,lib}.rs
│   │   ├── tests/             eval.rs 56, express.rs 19, host_prim.rs 3, world.rs 4
│   │   ├── examples/{repl,spells,world,genes}.rs
│   │   └── Cargo.toml         dev-deps: runes, codons, spells, genes, world
│   ├── runes/                 rune-tape lexer + resolver — zero deps
│   │   ├── src/lib.rs         PLAIN / PARAM tables, tape_to_sexpr
│   │   ├── tests/lex.rs       9 tests
│   │   └── Cargo.toml
│   ├── codons/                ASCII RNA codon tape lexer — zero deps
│   │   ├── src/lib.rs         codon table, tape_to_sexpr
│   │   ├── tests/lex.rs       6 tests
│   │   └── Cargo.toml
│   ├── spells/                rune prelude + install/install_with_world (ADR-016)
│   │   ├── src/lib.rs         PRELUDE_DEFINES, install, install_with_world
│   │   └── Cargo.toml         deps: lisp, world
│   ├── genes/                 genome prelude + express!/mutate!/breed!/render (ADR-016)
│   │   ├── src/lib.rs         PRELUDE_DEFINES, install, seeded, render_creature
│   │   └── Cargo.toml         deps: lisp
│   ├── world/                 tile grid + 5 world prims (ADR-018)
│   │   ├── src/lib.rs         Tile, World, pub mod world_prim
│   │   └── Cargo.toml         deps: lisp
│   ├── bench/                 criterion benches (core + demos)
│   │   ├── benches/{core,demos}.rs
│   │   └── Cargo.toml         deps: lisp, runes, codons, spells, genes, world
│   └── wasm/                  JS-facing bridge — wasm-bindgen cdylib
│       ├── src/lib.rs         WasmVm wrapper, owns world handle
│       └── Cargo.toml
├── web/                       browser shell — no bundler, plain ESM
│   ├── {index,spells,genes}.html
│   ├── styles.css             palette from docs/letrs.html
│   ├── {common,spells,genes}.js
│   └── pkg/                   wasm-bindgen output (gitignored)
└── docs/
    ├── letrs.html             single-page narrative tour
    └── project_notes/         this directory
```

## Stats as of 2026-05-29 (after ADR-016/017/018 refactor sequence)

- Lisp tests: 56 + 19 + 3 + 4 = 82 (`tests/eval.rs`, `tests/express.rs`, `tests/host_prim.rs`, `tests/world.rs`)
- Runes: 9 tests (`crates/runes/tests/lex.rs`)
- Codons: 6 tests (`crates/codons/tests/lex.rs`)
- **Total: 97 tests passing**
- Dependencies in `lisp`: 0; in `runes`: 0; in `codons`: 0; in `genes`/`world`: 1 (lisp); in `spells`: 2 (lisp, world); in `wasm`: 2 (wasm-bindgen, console_error_panic_hook)
- WASM artifact size: ~104 KB after `wasm-opt -Oz`; ~42 KB gzipped on the wire

## URLs / ports

- Local dev server: `http://localhost:8000/` (default in `just wasm-serve`)
- If 8000 is occupied, edit the justfile or run `python3 -m http.server -d web <port>` directly

## Test highlights

- `tail_calls_dont_grow_the_stack` — counts 100,000 deep, no growth
- `letrec_mutual_recursion` — even?/odd? in terms of each other
- `recursion_via_y_combinator` — factorial without letrec
- `macro_thread_first` — `(-> 5 (+ 3) (* 2))` → `16`, defined in lisp
- `macro_calls_macro` — a macro body uses another macro
- `quasiquote_splice` — `\`(1 ,@xs 4)` with `xs = '(2 3)` → `(1 2 3 4)`
- `canonical_example` (runes) — `tape_to_sexpr("ᚠ ᛊ 3 ᛁ") == "(list fire (area 3) ice)"`

## Style & conventions

- Dev doc: `docs/style.md` — clippy gate, lint-override convention
  (`#[expect(..., reason = "…")]`), comment prefixes (`SAFETY:`, `PERF:`,
  `CONTEXT:`, `TODO(issue #N):`), TODO workflow (file in
  `docs/project_incoming/`, not GitHub Issues), error stance (zero-dep,
  `Result<_, String>` — see ADR-002), import order.
- Reference: [Apollo Rust Best Practices](https://github.com/apollographql/rust-best-practices).
  Not authoritative — we cherry-pick. Deviations are documented in
  `docs/style.md` and the relevant ADRs.

## Related repos

- `../xsofy` — the original roguelike whose spell DSL inspired this. Same
  authoring conventions; do not push to either upstream from local sessions.
- `../let-go` — the Go-based Clojure dialect that xsofy runs on. Not used by
  letrs; mentioned only for context.

## Narrative

`docs/letrs.html` is the human-readable tour — open in a browser. Same
typography/aesthetic as xsofy's quest notes, covering the CEK heart, the
"real lisp" feature set, the spell DSL pipeline, and the macro system.
