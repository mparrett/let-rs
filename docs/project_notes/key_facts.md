# Key Facts

## Toolchain

- Rust 1.93+, edition 2024
- Workspace at repo root, members = `["crates/*"]`, resolver = `"3"`
- `just` is the task runner (Homebrew install). No `Makefile`.
- Core `lisp` crate: zero dependencies — keep it that way.
- `runes`, `codons`, `strokes` crates: zero deps.
- `world`, `genes`, `curves`, `macros` crates: depend only on `lisp`.
- `spells` crate: depends on `lisp` + `world` (the `install_with_world` helper combines both installs).
- `wasm` crate: `wasm-bindgen` (=0.2.114 pinned to match CLI), `console_error_panic_hook`, plus `macros` (for the user-facing REPL). Justified by ADR-002's "lisp stays platform-independent" caveat.

## WASM toolchain

- `wasm32-unknown-unknown` Rust target (`rustup target add wasm32-unknown-unknown`)
- `wasm-bindgen-cli` 0.2.114 (`cargo install -f wasm-bindgen-cli --version 0.2.114`)
- `binaryen` 129+ for `wasm-opt` (`brew install binaryen`)
- Any static-file server works — **no COI / SAB / service-worker shim required** (the whole reason this exists; ADR-009). Prefer `just wasm-serve` (which wraps `python3 -m http.server -d web 7670`) or `npx serve web -p 7670`. Agent sessions: raw `python3 -m http.server` may be blocked by the auto-mode classifier on the grounds of binding a port + exposing files; the `just` wrapper is a recognized task target and goes through.

Pipeline: `cargo build` → `wasm-bindgen --target web` → `wasm-opt -Oz --strip-debug --strip-producers`.

## Commands

```bash
just              # default → cargo test --workspace
just test         # same — explicit
just repl         # interactive REPL (examples/repl.rs)
just spells       # CLI rune-tape demo
just world        # CLI spell-paints-tiles demo
just genes        # CLI codon-tape → creature card demo
just curves       # CLI stroke-tape → L-system → ASCII canvas demo
just check        # cargo check --all-targets
just fmt          # cargo fmt --all
just wasm-build   # cargo build wasm + wasm-bindgen → web/pkg/
just wasm-serve   # wasm-build + python3 -m http.server -d web 7670
just bench        # criterion benches under crates/bench/
```

## Layout

```
let-rs/
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
│   ├── {index,spells,genes,curves}.html
│   ├── let-rs.html            dev log (narrative tour)
│   ├── styles.css             palette from let-rs.html
│   ├── {common,spells,genes,curves}.js
│   └── pkg/                   wasm-bindgen output (gitignored)
└── docs/
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

**Port-allocation scheme (across all projects on this machine):**
`6900 + first two letters of the project name, parsed as a base-36
number`. The "6900 + …" prefix keeps everything in a contiguous band
high enough to avoid common defaults (8000/3000/5173/etc.) yet low
enough to leave room above for ad-hoc binds.

For let-rs: `"le"` → `21·36 + 14 = 770` → **port 7670**.

| URL | Notes |
|---|---|
| `http://localhost:7670/let-rs.html` | Local dev log |
| `http://localhost:7670/index.html` | Landing page (links into the three labs) |
| `http://localhost:7670/{spells,genes,curves}.html` | The three labs |
| `http://100.126.31.103:7670/let-rs.html` | Same, over Tailscale (this machine's TS IP) |

**Starting the server:**

- Preferred: `just wasm-serve` — builds WASM, then serves on 7670.
  Recognized by the agent auto-mode classifier; goes through without
  prompting.
- One-off without a rebuild: `npx serve web -p 7670`. (Raw
  `python3 -m http.server -d web 7670` may be blocked by the
  classifier — use `just` or `npx serve`.)

**Gotchas (from prior incidents):**

- **Browser caching bites on the Tailscale URL.** If you "don't see"
  a change you just shipped, hard-refresh (⌘+Shift+R) before
  chasing a layout bug, and curl-verify the served bytes (e.g.
  `curl -s http://localhost:7670/let-rs.html | grep <marker>`).
  Prior incident: post-rune-refactor cache hid the new glyphs and
  triggered a wasted CSS hunt. Recorded in the 2026-05-31 handoff.
- **Multiple `serve` processes can fight for the port.** Many
  parallel projects on this machine often have their own dev
  servers running. `lsof -iTCP:7670 -sTCP:LISTEN` shows what's bound;
  `pkill -f 'serve web -p 7670'` clears stale `npx serve` runs.

## Test highlights

- `tail_calls_dont_grow_the_stack` — counts 100,000 deep, no growth
- `letrec_mutual_recursion` — even?/odd? in terms of each other
- `recursion_via_y_combinator` — factorial without letrec
- `macro_thread_first` — `(-> 5 (+ 3) (* 2))` → `16`, defined in lisp
- `macro_calls_macro` — a macro body uses another macro
- `quasiquote_splice` — `\`(1 ,@xs 4)` with `xs = '(2 3)` → `(1 2 3 4)`
- `canonical_example` (runes) — `tape_to_sexpr("ᚦ ᛞ 3 ᛇ") == "(list fire (area 3) ice)"`

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
  let-rs; mentioned only for context.

## Narrative

`web/let-rs.html` is the dev log — open in a browser. Same
typography/aesthetic as xsofy's quest notes, covering the CEK heart, the
"real lisp" feature set, the spell DSL pipeline, and the macro system.
