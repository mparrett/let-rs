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
- genes demo: codon-tape → diploid genome → phenotype creature card,
  parallel to spells but with genetics vocabulary (see ADR-011)

58 tests pass across the workspace; `lisp` core stays zero-deps.

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
- `world.rs` — minimal grid + log used by the spell demo
- `world_prim.rs` — `Val::WorldPrim` primitives that take `&mut World`
- `spells.rs` — the rune prelude as `PRELUDE_DEFINES` (sequence of
  top-level `(define …)` forms) + `install(vm)` that installs them
  into the Vm env once. Consumers (`examples/spells.rs`, WASM bridge)
  call `spells::install` at startup; each cast then evaluates just
  the body. Coord seeding still happens at the call site via
  `assoc-set`. See ADR-010, ADR-014.
- `genes.rs` — genome vocabulary: `PRELUDE_DEFINES` (seed-independent
  half) + `install(vm)` (registers prims + installs defines) +
  `seeded(seed, body)` (per-cast wrapper that re-binds the four
  mutate variants over the caller's seed so ADR-012's lexical-seed
  pattern still holds). Plus the `express!` resolver and the
  creature-card renderer. Shared by `examples/genes.rs` and the WASM
  bridge. See ADR-011, ADR-014.
- `parse.rs` — tokenize, `read` (→ Datum), `read_many` (→ Vec<Datum>),
  `compile` (→ Expr), special forms, quasiquote compilation
- `lib.rs` — `Vm`, top-level `define` / `defmacro` registration,
  macro expansion, datum⇄val conversion. `eval_str` accepts a
  sequence of top-level forms; returns the last expression's value
  (ADR-014).

Examples in `crates/lisp/examples/`:

- `repl.rs` — interactive REPL (`just repl`)
- `spells.rs` — rune tape → ctx pipeline; engine untouched, primitives in lisp
- `world.rs` — spell ctx applied to a 7×5 grid via `world-apply!`
- `genes.rs` — codon tape → diploid genome → `express!` resolver → ASCII
  creature card. Driver code only — the prelude, prim, and renderer all
  live in `lisp::genes` (shared with the WASM bridge). The `MUT` codon
  family (`MUT` 25%, `M01`/`M10`/`M50`) triggers seeded mutation; the
  example wraps each cast in `(let ((seed N)) …)` so the prelude's
  `mutate` closures see it via lexical scope. A `breeding(…)` helper
  in the example crosses two parent strands via `breed!` for Mendelian
  segregation. See ADR-011, ADR-012, ADR-013.

Sibling crates:

- `crates/runes/` — Unicode rune tape → `(list …)` sexpr. Zero deps; the only
  source of truth for the rune table; consumed by both `examples/spells` and
  the WASM bridge. See ADR-010.
- `crates/codons/` — ASCII RNA codon tape (`AUG CGA …`) → `(list …)` sexpr.
  Zero deps; sole source of truth for the codon table. Consumed by both
  `examples/genes` and the WASM bridge. The genome prelude + resolver +
  renderer live in `lisp::genes` (one source of truth, no prelude
  duplication). Mirrors the `runes/` shape; ADR-011.
- `crates/wasm/` — JS-facing bridge (`wasm-bindgen` `cdylib`). Wraps
  `lisp::Vm` + `World`, embeds the spell prelude as a const string, exposes
  `new(width, height)`, `eval(src)`, `cast(tape, x, y)`, `grid()`, `log()`,
  `reset_world()`, and `cast_genome(tape)` (returns a rendered creature
  card; consumes `lisp::genes` so there's a single source of truth across
  CLI + WASM). ~120 LOC. Pinned to `wasm-bindgen =0.2.114` to match the
  installed CLI (ADR-009).

Web shell at `web/` — three pages, one bundle:

- `web/index.html` — landing page. Two `.lab-card` links to the demos
  (plum-accented Spell Lab, honey-accented Gene Lab) over a Letrs
  masthead. No WASM init on this page; it's pure HTML.
- `web/spells.html` — Spell Lab + REPL, two-column at ≥940px. Plum
  rune-palette aesthetic.
- `web/genes.html` — Gene Lab + REPL, two-column at ≥940px. Honey/sage
  codon-palette aesthetic.
- `web/styles.css` — shared. Palette + typography lifted from
  `docs/letrs.html`. Per-page accents picked via per-element classes
  (`.sigil.gene`, `.lab-card.spells`, etc).
- `web/common.js` — plain ESM. `await init()`, `Vm` construction, COI
  chip, REPL wiring. Imported by spells.js and genes.js.
- `web/spells.js` — Spell Lab page module: rune palette, `vm.cast`,
  world refresh, seed cast.
- `web/genes.js` — Gene Lab page module: codon palette,
  `vm.cast_genome`, render card, seed.
- `web/pkg/` — `wasm-bindgen` output (gitignored).

## Build / test

```bash
just              # default: cargo test --workspace (40 tests)
just test         # same — explicit
just repl
just spells       # CLI rune-tape demo
just world        # CLI spell-paints-tiles demo
just genes        # CLI codon-tape → creature card demo
just check
just wasm-build   # cargo build --target wasm32-unknown-unknown + wasm-bindgen
just wasm-serve   # build + python3 -m http.server -d web 8000
```

Rust 1.93+, edition 2024. The core `lisp` crate stays zero-deps —
keep it that way. `runes` and `codons` are zero-deps too. `wasm` may take deps
(`wasm-bindgen`, `console_error_panic_hook`); this is allowed by
ADR-002's "lisp stays platform-independent" caveat.

WASM build requires three tools installed once:

```bash
rustup target add wasm32-unknown-unknown
cargo install -f wasm-bindgen-cli --version 0.2.114   # must match the pin in crates/wasm/Cargo.toml
brew install binaryen                                  # provides wasm-opt
```

If you bump the `wasm-bindgen` pin, install the matching CLI version at the
same time — the error if they drift is loud but the fix is manual.

The `wasm-build` pipeline is three stages: `cargo build` →
`wasm-bindgen --target web` → `wasm-opt -Oz --strip-debug --strip-producers`.
The optimizer takes ~135 KB raw down to ~104 KB (and ~49 KB → ~42 KB
gzipped on the wire). Removing the `wasm-opt` step is safe; it just
ships a larger bundle.

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
  `crates/lisp/src/spells.rs` (`PRELUDE_DEFINES`) — both consumers
  import from there, so one edit is enough.
- Adding a new codon: edit `crates/codons/src/lib.rs`. If the codon
  introduces a new trait, also extend the genome prelude
  (`PRELUDE_DEFINES`) and the `TRAITS` classification table in
  `crates/lisp/src/genes.rs` — both the CLI demo and the WASM bridge
  see it automatically through `lisp::genes`. Categorical allele
  payloads need to be quoted in the codon fragment (`(color 'green
  dom)`) so the evaluator treats them as symbols rather than variable
  lookups. For a new categorical trait, add its option pool to
  `Kind::Categorical(&[…])` so `mutate!` knows what values to draw from.
- Installable preludes (ADR-014): `eval_str` accepts a sequence of
  top-level forms. `(define name body)` extends the Vm env in place;
  single-binding self-recursion works via `extend_placeholder`,
  mutual recursion across separate defines does not (wrap in
  `letrec`). Both `define` and `defmacro` are rejected at non-top-
  level positions. DSL packs expose `install(vm)` for one-shot setup.

## Project Memory

Memory files live in `docs/project_notes/`.

**Before proposing changes**: Check `decisions.md` for existing ADRs
**When encountering errors**: Search `bugs.md` for known solutions
**When looking up config**: Check `key_facts.md` for ports, URLs, environments

When resolving bugs or making decisions, update the relevant file.
