# CLAUDE.md

Guidance for Claude Code when working in the `let-rs` repository.

## What this is

A small functional lisp built on a CEK abstract machine (Felleisen & Friedman,
1980s), written in zero-dependency Rust (workspace, edition 2024). The intended
use case is a rune-tape spell DSL — a clean-room spin-off of [xsofy](../xsofy)'s
magic system, where rune sequences compile to s-expressions that thread a
context through a pipeline of primitives. The point is that the smallest
interesting substrate you can call a real programming language fits in a few
hundred lines and once you have it, the rest is just a vocabulary.

The dev log lives in `web/let-rs.html` (also linked from
`web/index.html`) — open it in a browser, it's the running narrative
of what's here and why.

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
- curves demo: stroke-tape → L-system rewrite → 8-direction turtle →
  ASCII canvas, third sibling completing the rule of three (see ADR-019)

The `lisp` core stays zero-deps.

## Architecture (read this first)

The five CEK transition rules live in `crates/lisp/src/step.rs` — read that
file before anything else; the rest of the engine is decoration.

- `expr.rs` — AST: `Num | Bool | Var | Quote(Rc<Val>) | Lam | App | If | Letrec`
- `val.rs` — runtime values: `Num | Ratio | Bool | Sym | Nil | Cons | Clo | Prim`,
  plus `Arity` and `Display`. `Val::Prim` holds an
  `Rc<dyn Fn(&[Val]) -> Result<Val, String>>` so host prims can
  capture state at registration time without the engine knowing what
  they capture (ADR-017).
- `env.rs` — Rc-linked immutable frames; each slot is an `Rc<RefCell<Val>>` to
  support letrec placeholder bindings
- `k.rs` — continuation variants: `Halt | App | If | Letrec`
- `step.rs` — `step(State) -> Step` and the driver `run` loop. The
  engine no longer threads a `&World` through CEK state; that
  responsibility moved to host-owned prim closures (ADR-017).
- `prim.rs` — pure built-ins (arithmetic, list ops, predicates, eq?).
  Each fn-ptr is wrapped in an `Rc::new` at `initial_env` time so the
  one prim variant carries them uniformly with state-capturing host
  closures.

(`world.rs` and `world_prim.rs` lived in this crate until ADR-018
extracted them to `crates/world/`. The engine no longer ships any
host types.)
- `parse.rs` — tokenize, `read` (→ Datum), `read_many` (→ Vec<Datum>),
  `compile` (→ Expr), special forms, quasiquote compilation
- `lib.rs` — `Vm`, top-level `define` / `defmacro` registration,
  macro expansion, datum⇄val conversion. `eval_str` accepts a
  sequence of top-level forms; returns the last expression's value
  (ADR-014).

The spell, gene, and curve DSL packs live in sibling crates
(`crates/spells/`, `crates/genes/`, `crates/curves/`) as of ADR-016
and ADR-019 — see "Sibling crates" below.

Examples in `crates/lisp/examples/`:

- `repl.rs` — interactive REPL (`just repl`)
- `spells.rs` — rune tape → ctx pipeline; engine untouched, primitives in lisp
- `world.rs` — spell ctx applied to a 7×5 grid via `world-apply!`
- `genes.rs` — codon tape → diploid genome → `express!` resolver → ASCII
  creature card. Driver code only — the prelude, prim, and renderer all
  live in `crates/genes/` (shared with the WASM bridge). The `MUT` codon
  family (`MUT` 25%, `M01`/`M10`/`M50`) triggers seeded mutation; the
  example wraps each cast in `(let ((seed N)) …)` so the prelude's
  `mutate` closures see it via lexical scope. A `breeding(…)` helper
  in the example crosses two parent strands via `breed!` for Mendelian
  segregation. See ADR-011, ADR-012, ADR-013.
- `curves.rs` — stroke tape → `(grow axiom rules n)` → `(draw! …)` →
  ASCII canvas. The pure-lisp rewrite engine (`expand`, `expand-one`,
  `grow`) lives in the curves prelude; turtle state + the
  `draw!`/`render!`/`reset!` prims live in `crates/curves/`. First demo
  whose tape is rewritten before being interpreted. See ADR-019.

Sibling crates:

- `crates/runes/` — Unicode rune tape → `(list …)` sexpr. Zero deps; the only
  source of truth for the rune table; consumed by both `examples/spells` and
  the WASM bridge. See ADR-010.
- `crates/codons/` — ASCII RNA codon tape (`AUG CGA …`) → `(list …)` sexpr.
  Zero deps; sole source of truth for the codon table. Mirrors the
  `runes/` shape; ADR-011.
- `crates/strokes/` — turtle glyph tape (`F + - [ ]`) → `(list 'F '+ …)`
  sexpr. Zero deps; sole source of truth for the stroke table. Output
  is *quoted symbols* (not function calls) so the curves prelude's
  pure-lisp `grow` can rewrite the tape before `draw!` interprets it.
  See ADR-019.
- `crates/world/` — `World` (tile grid + event log) and the 5 world
  prims (`world-tile`, `world-set-tile!`, `world-log!`, `world-size`,
  `world-apply!`). Hosts wire it in via `world::world_prim::install(&mut
  vm, world.clone())`. Sibling to runes/codons; engine has no awareness.
  See ADR-017, ADR-018.
- `crates/spells/` — rune prelude as `PRELUDE_DEFINES` (sequence of
  top-level `(define …)` forms) + `install(vm)` that installs them
  into the Vm env once, plus `install_with_world(vm, world)` that does
  both halves at once. Depends on `lisp` and `world`. Consumers
  (`examples/spells.rs`, WASM bridge) call `spells::install_with_world`
  at startup; each cast then evaluates just the body. Coord seeding
  still happens at the call site via `assoc-set`. See ADR-010, ADR-014,
  ADR-016, ADR-018.
- `crates/genes/` — genome vocabulary: `PRELUDE_DEFINES`
  (seed-independent half) + `install(vm)` (registers prims + installs
  defines) + `seeded(seed, body)` (per-cast wrapper that re-binds the
  four mutate variants over the caller's seed so ADR-012's
  lexical-seed pattern still holds). Plus the `express!` resolver and
  the creature-card renderer. Depends only on `lisp` (`Vm`, `Val`,
  `Arity`). Shared by `examples/genes.rs` and the WASM bridge. See
  ADR-011, ADR-014, ADR-016.
- `crates/curves/` — L-system vocabulary: `Turtle` (8-direction sparse
  canvas, host-owned via `Rc<RefCell<Turtle>>`), the three turtle prims
  (`draw!`, `render!`, `reset!`), `PRELUDE_DEFINES` with the pure-lisp
  rewrite engine (`expand`, `expand-one`, `grow`), and a Rust-side
  `render(&Turtle) -> String` for direct access. `install(vm, turtle)`
  wires all of it in one call. Depends only on `lisp`. Cast pipeline is
  `(draw! (grow axiom rules n))` then `(render!)`. See ADR-019.
- `crates/wasm/` — JS-facing bridge (`wasm-bindgen` `cdylib`). Wraps
  `lisp::Vm` + `World` + `Turtle`, installs all three DSL packs at
  construction, exposes `new(width, height)`, `eval(src)`,
  `cast(tape, x, y)`, `cast_genome(tape, seed)`, `cast_breed(a, b, seed)`,
  `cast_curve(axiom, rules_sexpr, iters)`, plus `grid()` / `log()` /
  `reset_world()`. Pinned to `wasm-bindgen =0.2.114` to match the
  installed CLI (ADR-009). The curve bridge expects `rules_sexpr` as a
  pre-built lisp form (the page module owns the `lhs = rhs` parser) so
  the Rust side stays domain-neutral.

Web shell at `web/` — four pages, one bundle:

- `web/index.html` — landing page. Three `.lab-card` links to the demos
  (plum-accented Spell Lab, honey-accented Gene Lab, copper-accented
  Curve Lab) over a Let-rs masthead. No WASM init on this page; it's
  pure HTML.
- `web/spells.html` — Spell Lab + REPL, two-column at ≥940px. Plum
  rune-palette aesthetic.
- `web/genes.html` — Gene Lab + REPL, two-column at ≥940px. Honey/sage
  codon-palette aesthetic.
- `web/curves.html` — Curve Lab + REPL, two-column at ≥940px. Copper
  stroke-palette aesthetic; rotation buttons indigo, branching buttons
  sage so the three glyph groups read distinctly.
- `web/styles.css` — shared. Palette + typography lifted from
  `web/let-rs.html`. Per-page accents picked via per-element classes
  (`.sigil.gene`, `.lab-card.spells`, `.canvas`, etc).
- `web/common.js` — plain ESM. `await init()`, `Vm` construction, COI
  chip, REPL wiring. Imported by spells.js, genes.js, and curves.js.
- `web/spells.js` — Spell Lab page module: rune palette, `vm.cast`,
  world refresh, seed cast.
- `web/genes.js` — Gene Lab page module: codon palette,
  `vm.cast_genome`, render card, seed.
- `web/curves.js` — Curve Lab page module: stroke palette, rules-text
  parser (`lhs = rhs` per line → lisp alist), `vm.cast_curve`, canvas
  refresh, seed.
- `web/pkg/` — `wasm-bindgen` output (gitignored).

## Build / test

```bash
just              # default: cargo test --workspace
just test         # same — explicit
just repl
just spells       # CLI rune-tape demo
just world        # CLI spell-paints-tiles demo
just genes        # CLI codon-tape → creature card demo
just curves       # CLI stroke-tape → L-system → ASCII canvas demo
just check
just wasm-build   # cargo build --target wasm32-unknown-unknown + wasm-bindgen
just wasm-serve   # build + python3 -m http.server -d web 7670 (per-project port; see key_facts.md)
just bench        # criterion benches under crates/bench/ (core + demos)
```

Rust 1.93+, edition 2024. The core `lisp` crate stays zero-deps —
keep it that way. `runes`, `codons`, and `strokes` are zero-deps too.
`world`, `spells`, `genes`, and `curves` depend only on `lisp` (and
`spells` also depends on `world` for the `install_with_world` helper).
`wasm` may take deps (`wasm-bindgen`, `console_error_panic_hook`); this
is allowed by ADR-002's "lisp stays platform-independent" caveat.

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
- Host prims are registered via `vm.register_prim(name, arity, |args|
  …)`. The callback is wrapped in `Rc<dyn Fn>` so it can capture host
  state (`Rc<RefCell<World>>`, an `Rc<RefCell<Counter>>`, whatever).
  Pure prims are just closures that don't capture anything. ADR-017
  removed the `Val::WorldPrim` distinction; the engine is host-agnostic.
- The spell DSL is a *vocabulary*, not a feature of the language. Spell
  primitives are user-level closures over ctx. Adding behavior means adding
  a primitive (closure), not a new engine rule.
- Adding a new rune: edit `crates/runes/src/lib.rs` — both the CLI demo
  and the WASM bridge see it automatically. If the new rune needs a
  matching primitive, also extend the spell prelude in
  `crates/spells/src/lib.rs` (`PRELUDE_DEFINES`) — both consumers
  import from there, so one edit is enough.
- Adding a new codon: edit `crates/codons/src/lib.rs`. If the codon
  introduces a new trait, also extend the genome prelude
  (`PRELUDE_DEFINES`) and the `TRAITS` classification table in
  `crates/genes/src/lib.rs` — both the CLI demo and the WASM bridge
  see it automatically through the `genes` crate. Categorical allele
  payloads need to be quoted in the codon fragment (`(color 'green
  dom)`) so the evaluator treats them as symbols rather than variable
  lookups. For a new categorical trait, add its option pool to
  `Kind::Categorical(&[…])` so `mutate!` knows what values to draw from.
- Adding a new stroke: edit `crates/strokes/src/lib.rs` (table) AND
  extend the `draw!` dispatch in `crates/curves/src/lib.rs` to handle
  the new symbol — strokes emits quoted symbols, `draw!` is where the
  turtle action lives. Strokes that map to compound turtle motion
  (e.g. a 90° turn) belong as prelude-level lisp defines rather than
  new prims; keep the prim surface minimal.
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
**When asked about "world" or host coupling**: Read `host-state.md`
  for context on what `World` actually is, why it's not generic, and
  where it might go.

When resolving bugs or making decisions, update the relevant file.
