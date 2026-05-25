# Key Facts

## Toolchain

- Rust 1.93+, edition 2024
- Workspace at repo root, members = `["crates/*"]`, resolver = `"3"`
- `just` is the task runner (Homebrew install). No `Makefile`.
- Core `lisp` crate: zero dependencies — keep it that way.
- `runes` crate: zero dependencies.
- `wasm` crate: `wasm-bindgen` (=0.2.114 pinned to match CLI), `console_error_panic_hook`. Justified by ADR-002's "lisp stays platform-independent" caveat.

## WASM toolchain

- `wasm32-unknown-unknown` Rust target (`rustup target add wasm32-unknown-unknown`)
- `wasm-bindgen-cli` 0.2.114 (`cargo install -f wasm-bindgen-cli --version 0.2.114`)
- `binaryen` 129+ for `wasm-opt` (`brew install binaryen`)
- Plain `python3 -m http.server` for serving — **no COI / SAB / service-worker shim required** (the whole reason this exists; ADR-009)

Pipeline: `cargo build` → `wasm-bindgen --target web` → `wasm-opt -Oz --strip-debug --strip-producers`.

## Commands

```bash
just              # default → cargo test --workspace
just test         # run all tests (34 currently)
just repl         # interactive REPL (examples/repl.rs)
just spells       # CLI rune-tape demo
just world        # CLI spell-paints-tiles demo
just check        # cargo check --all-targets
just fmt          # cargo fmt --all
just wasm-build   # cargo build wasm + wasm-bindgen → web/pkg/
just wasm-serve   # wasm-build + python3 -m http.server -d web 8000
```

## Layout

```
letrs/
├── Cargo.toml                 workspace root, members = ["crates/*"]
├── CLAUDE.md                  session-orientation
├── justfile
├── crates/
│   ├── lisp/                  the engine — zero deps
│   │   ├── src/{expr,val,env,k,step,prim,world,world_prim,parse,lib}.rs
│   │   ├── tests/eval.rs      26 tests
│   │   ├── examples/{repl,spells,world}.rs
│   │   └── Cargo.toml         dev-deps: runes (for examples/spells only)
│   ├── runes/                 rune-tape lexer + resolver — zero deps
│   │   ├── src/lib.rs         PLAIN / PARAM tables, tape_to_sexpr
│   │   ├── tests/lex.rs       8 tests
│   │   └── Cargo.toml
│   └── wasm/                  JS-facing bridge — wasm-bindgen cdylib
│       ├── src/lib.rs         WasmVm wrapper, prelude const
│       └── Cargo.toml
├── web/                       browser shell — no bundler, plain ESM
│   ├── index.html             Cinzel masthead + Spell Lab + REPL panels
│   ├── styles.css             palette from docs/letrs.html
│   ├── shell.js               import init, { Vm } from './pkg/wasm.js'
│   └── pkg/                   wasm-bindgen output (gitignored)
└── docs/
    ├── letrs.html             single-page narrative tour
    └── project_notes/         this directory
```

## Stats as of 2026-05-25 (after WASM bridge)

- Rust LOC: ~2,222 across the workspace
- Lisp core: 26 tests (`crates/lisp/tests/eval.rs`)
- Runes: 8 tests (`crates/runes/tests/lex.rs`)
- **Total: 34 tests passing**
- Dependencies in `lisp`: 0; in `runes`: 0; in `wasm`: 2 (wasm-bindgen, console_error_panic_hook)
- Commits: 5 to date (initial; world; macros; HTML page + memory; WASM bridge pending this commit)
- WASM artifact size: ~104 KB after `wasm-opt -Oz` (down from ~135 KB raw); ~42 KB gzipped on the wire

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

## Related repos

- `../xsofy` — the original roguelike whose spell DSL inspired this. Same
  authoring conventions; do not push to either upstream from local sessions.
- `../let-go` — the Go-based Clojure dialect that xsofy runs on. Not used by
  letrs; mentioned only for context.

## Narrative

`docs/letrs.html` is the human-readable tour — open in a browser. Same
typography/aesthetic as xsofy's quest notes, covering the CEK heart, the
"real lisp" feature set, the spell DSL pipeline, and the macro system.
