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

Three slices land in `crates/lisp`:

- the CEK machine (5 transition rules) + the run loop
- a "real lisp" feature set (closures, letrec, cons, quote, variadic prims,
  let/let*/cond, predicates, comparison chains)
- procedural macros with quasiquote, plus a minimal host world and a spell DSL
  demo end-to-end

26 tests pass; no dependencies; ~1.5K LOC of Rust.

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

## Build / test

```bash
just              # default: cargo test -p lisp
just test         # 26 tests
just repl
just check
cargo run -q -p lisp --example spells
cargo run -q -p lisp --example world
```

Rust 1.93+, edition 2024. No external dependencies — keep it that way for the
core `lisp` crate. WASM/game crates will live alongside as separate crates
when they exist; they may take deps. `lisp` stays platform-independent.

## Conventions

- Special forms (`lambda`, `if`, `quote`, `letrec`, `let`, `let*`, `cond`,
  `quasiquote`) live in `parse.rs`. Everything else can be a macro.
- World-aware primitives use `Val::WorldPrim`; pure ones use `Val::Prim`.
  Don't promote a pure prim to WorldPrim without reason — the split is what
  keeps the language testable in isolation from the host.
- The spell DSL is a *vocabulary*, not a feature of the language. Spell
  primitives are user-level closures over ctx. Adding behavior means adding
  a primitive (closure), not a new engine rule.

## Project Memory

Memory files live in `docs/project_notes/`.

**Before proposing changes**: Check `decisions.md` for existing ADRs
**When encountering errors**: Search `bugs.md` for known solutions
**When looking up config**: Check `key_facts.md` for ports, URLs, environments

When resolving bugs or making decisions, update the relevant file.
