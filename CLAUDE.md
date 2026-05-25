# CLAUDE.md

Guidance for Claude Code when working in the `letrs` repository.

## What this is

A small functional lisp built on a CEK abstract machine (Felleisen & Friedman,
1980s), written in zero-dependency Rust (workspace, edition 2024). The intended
use case is a rune-tape spell DSL — a clean-room spin-off of [xsofy](../xsofy)'s
magic system, where rune sequences compile to s-expressions that thread a
context through a pipeline of primitives. The point is that the smallest
interesting substrate you can call a real programming language fits in a few
hundred lines and once you have it, the rest is just a vocabulary.

Narrative overview lives in `docs/letrs.html` — open it in a browser, it's the
single-page tour of what's here and why.

Slices that have landed:

- the CEK machine (5 transition rules) + the run loop
- a "real lisp" feature set (closures, letrec, cons, quote, variadic prims,
  let/let*/cond, predicates, comparison chains)
- procedural macros with quasiquote, plus a minimal host world and a spell DSL
  demo end-to-end
- rune translation extracted to `crates/runes/` (zero-dep micro-crate)
- WASM bridge (`crates/wasm/` + `web/`) — REPL + Spell Lab in the browser via
  `wasm-bindgen`, no COI / SAB required (see ADR-009)

34 tests pass across the workspace; `lisp` core stays zero-deps.

## Architecture (read this first)

The five CEK transition rules live in `crates/lisp/src/step.rs` — read that
file before anything else; the rest of the engine is decoration.

- `expr.rs` — AST: `Num | Bool | Var | Quote(Rc<Val>) | Lam | App | If | Letrec`
- `val.rs` — runtime values: `Num | Bool | Sym | Nil | Cons | Clo | Prim | WorldPrim`,
  plus `Arity` and `Display`
- `env.rs` — Rc-linked immutable frames; each slot is an `Rc<RefCell<Val>>` to
  support letrec placeholder bindings
- `k.rs` — continuation variants: `Halt | App | If | Letrec`
- `step.rs` — `step(State, &world) -> Step` and the driver `run` loop
- `prim.rs` — pure built-ins (arithmetic, list ops, predicates, eq?)
- `world.rs` — minimal grid + log used by the demo
- `world_prim.rs` — `Val::WorldPrim` primitives that take `&mut World`
- `parse.rs` — tokenize, `read` (→ Datum), `compile` (→ Expr), special forms,
  quasiquote compilation
- `lib.rs` — `Vm`, macro expansion, datum⇄val conversion

Examples in `crates/lisp/examples/`:

- `repl.rs` — interactive REPL (`just repl`)
- `spells.rs` — rune tape → ctx pipeline; engine untouched, primitives in lisp
- `world.rs` — spell ctx applied to a 7×5 grid via `world-apply!`

Sibling crates:

- `crates/runes/` — Unicode rune tape → `(list …)` sexpr. Zero deps; the only
  source of truth for the rune table; consumed by both `examples/spells` and
  the WASM bridge. See ADR-010.
- `crates/wasm/` — JS-facing bridge (`wasm-bindgen` `cdylib`). Wraps
  `lisp::Vm` + `World`, embeds the spell prelude as a const string, exposes
  `new(width, height)`, `eval(src)`, `cast(tape, x, y)`, `grid()`, `log()`,
  `reset_world()`. ~90 LOC. Pinned to `wasm-bindgen =0.2.114` to match the
  installed CLI (ADR-009).

Web shell at `web/`:

- `web/index.html` — Cinzel masthead + two `<section>` panels (Spell Lab,
  REPL) + rune palette + cheatsheet
- `web/styles.css` — palette + typography lifted from `docs/letrs.html`
- `web/shell.js` — plain ESM: `await init(); const vm = new Vm(7, 5); …`
- `web/pkg/` — `wasm-bindgen` output (gitignored)

## Build / test

```bash
just              # default: cargo test --workspace (34 tests)
just test         # same — explicit
just repl
just spells       # CLI rune-tape demo
just world        # CLI spell-paints-tiles demo
just check
just wasm-build   # cargo build --target wasm32-unknown-unknown + wasm-bindgen
just wasm-serve   # build + python3 -m http.server -d web 8000
```

Rust 1.93+, edition 2024. The core `lisp` crate stays zero-deps —
keep it that way. `runes` is zero-deps too. `wasm` may take deps
(`wasm-bindgen`, `console_error_panic_hook`); this is allowed by
ADR-002's "lisp stays platform-independent" caveat.

WASM build requires `wasm-bindgen-cli` installed at the version pinned in
`crates/wasm/Cargo.toml`:

```bash
cargo install -f wasm-bindgen-cli --version 0.2.114
```

If you bump that pin, install the matching CLI version at the same time —
the error if they drift is loud but the fix is manual.

## Conventions

- Special forms (`lambda`, `if`, `quote`, `letrec`, `let`, `let*`, `cond`,
  `quasiquote`) live in `parse.rs`. Everything else can be a macro.
- World-aware primitives use `Val::WorldPrim`; pure ones use `Val::Prim`.
  Don't promote a pure prim to WorldPrim without reason — the split is what
  keeps the language testable in isolation from the host.
- The spell DSL is a *vocabulary*, not a feature of the language. Spell
  primitives are user-level closures over ctx. Adding behavior means adding
  a primitive (closure), not a new engine rule.
- Adding a new rune: edit `crates/runes/src/lib.rs` — both the CLI demo
  and the WASM bridge see it automatically. If the new rune needs a
  matching primitive, also extend the spell prelude in
  `crates/lisp/examples/spells.rs` AND `crates/wasm/src/lib.rs`
  (`SPELL_PRELUDE_BINDINGS`). The two prelude copies will eventually
  consolidate — until then, keep them in sync.

## Project Memory

Memory files live in `docs/project_notes/`.

**Before proposing changes**: Check `decisions.md` for existing ADRs
**When encountering errors**: Search `bugs.md` for known solutions
**When looking up config**: Check `key_facts.md` for ports, URLs, environments

When resolving bugs or making decisions, update the relevant file.
