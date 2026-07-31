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
- procedural macros (sibling crate, ADR-024) with quasiquote, plus a minimal
  host world and a spell DSL demo end-to-end
- rune translation extracted to `crates/runes/` (zero-dep micro-crate)
- WASM bridge (`crates/wasm/` + `web/`) — REPL + Spell Lab in the browser via
  `wasm-bindgen`, no COI / SAB required (see ADR-009)
- structured errors with source spans (ADR-039) and a pausable machine
  (ADR-040) — errors carry `line:col` and render a caret; evaluation can
  be sliced, resumed, single-stepped, and cancelled
- in-language error handling (ADR-041) — `raise` / `error` / `guard`,
  with conditions as ordinary lists and prim failures catchable
- namespaces (ADR-042) — each DSL pack gets its own binding table
  chained to a shared root, so spells and genes can both define
  `thread`; exports are explicit and collisions are refused
- genes demo: codon-tape → diploid genome → phenotype creature card,
  parallel to spells but with genetics vocabulary (see ADR-011)
- curves demo: stroke-tape → L-system rewrite → 8-direction turtle →
  ASCII canvas, third sibling completing the rule of three (see ADR-019)

The `lisp` core stays zero-deps.

## Architecture (read this first)

The five CEK transition rules live in `crates/lisp/src/step.rs` — read that
file before anything else; the rest of the engine is decoration.

- `expr.rs` — AST: `Num | Bool | Var | Quote(Rc<Val>) | Lam | App | If |
  Letrec | SetBang | Raise | Guard`. `Lam` and `App` hold `Rc<[…]>`, not
  `Vec`, so the `K` that walks an application shares the slice instead of
  copying it (ADR-035). `Var`, `App`, and `Raise` carry an
  `Option<Span>` — those three, because they're the ones that can
  *originate* a runtime failure (ADR-039, extended by ADR-041). Don't add
  spans to the rest; a literal never errors.
- `error.rs` — `LispErr { msg, span }` + `Span { line, col, len }` +
  `render_span` (source line with a caret run under it). **`with_span`
  fills a span only if one isn't already set** — that's what lets
  `compile` annotate at a single point without walking every error's
  position out to the top-level form. `From<String>` keeps prims and host
  callbacks on their existing signatures. `None` is a real answer, not a
  gap: macro output and host-built forms report unpositioned (ADR-039).
- `val.rs` — runtime values: `Num | Ratio | Bool | Sym | Str | Nil | Cons | Clo | Prim`,
  plus `Arity` and `Display`. `Val::Prim` holds an
  `Rc<dyn Fn(&[Val]) -> Result<Val, String>>` so host prims can
  capture state at registration time without the engine knowing what
  they capture (ADR-017). `Val::Clo` holds `params: Rc<[Sym]>` —
  `Val` is `Clone` and `Env::lookup` clones out of the store, so a
  `Vec` here meant every mention of a function name allocated
  (ADR-035). Keep new `Val` fields cheap to clone for the same reason.
- `ns.rs` — `Namespace`: a top-level binding table plus an optional
  parent (ADR-042). Packs get a child of the root; lookup walks outward,
  `define` always writes to the table it started in. **Resolution is
  lexical**: `Env` holds the namespace and closures capture their `Env`,
  so a pack's internals resolve to its own definitions no matter who
  calls them. `export` shares the *cell*, so `set!` through either name
  writes the same slot — that's what keeps the mana counter readable from
  root. Exporting a name another pack exported is an error that names
  both; don't weaken that, the silence was the original bug. Collisions
  are decided by **provenance** (which pack published the name), not cell
  identity — re-running a prelude makes fresh cells, so an identity check
  turns every reinstall into a false collision. Handles are opaque
  `NsHandle` values, never `Rc<Namespace>`: handing out the `Rc` lets a
  caller keep the whole globals table alive past its `Vm`, which is the
  ADR-036 invariant.
- `env.rs` — Rc-linked immutable frames. Post-ADR-023 (CESK) each
  frame carries a `Copy` `Addr` into the Vm's `Store` rather than an
  `Rc<RefCell<Val>>` per slot; the top-level `globals` table kept its
  `Rc<RefCell<Val>>` cells (for the ADR-015 `Weak` back-edge). Letrec
  placeholders live in the store.
- `store.rs` — the CESK `Store` (ADR-023): a `Vec<Val>` addressed
  by `Addr(u32)`, with a free list. Frame slots and letrec
  placeholders are `Addr`s into it, so a closure capturing an env
  holds cheap `Copy` indices instead of refcounted cells. A frame
  owns its slot: `Frame::drop` returns it to the free list, so the
  arena is sized by live env depth, not by total evaluation
  (ADR-033). `Store::len` is live slots; `Store::slots` is the
  high-water mark. **Residual:** a closure capturing the frame that
  owns its own slot keeps that frame alive, so `Frame::drop` never
  fires and the slot is retained — one slot per recursive closure,
  pinned by `recursive_closures_retain_their_slot`, fix sketched in
  ADR-038. Don't restate reclamation as unconditional.
  `alloc`/`get`/`set` are `pub(crate)` and `Addr`'s
  index is private (ADR-036), so `Vm::store_weak` is a read-only
  diagnostic handle — there's no way to mint an `Addr` outside the
  engine and therefore nothing to read or write through it.
- `k.rs` — continuation variants: `Halt | App | If | Letrec | SetBang`.
  `apply_k` takes the `Rc<K>` **by value** and `Rc::try_unwrap`s it so
  fields move out rather than being cloned — valid because there are no
  first-class continuations, so every `K` is uniquely owned (ADR-035).
  Don't change it back to matching on `&*k`: that reintroduces an
  O(n²) clone per application.
- `step.rs` — `step(State) -> Step`, the driver `run` loop, and
  `Machine` (ADR-040). Three modes: `Eval`, `Apply`, and `Raise`
  (ADR-041). Every runtime failure enters `Raise` rather than returning
  `Err`, which is what makes prim complaints, unbound variables, arity
  and non-callable heads uniformly catchable — **except the step
  budget**, which lives in `Machine::run` and must stay uncatchable or a
  guarded runaway loop becomes unkillable. Unwinding discards one frame
  per step, so it stays interruptible and reclaims store slots as it
  goes. The engine no longer threads a `&World` through
  CEK state; that responsibility moved to host-owned prim closures
  (ADR-017). `Machine::run(budget)` returns `Progress::{Done, Paused}` —
  **pausing is not an error**; `run_bounded` is the wrapper that turns
  `Paused` back into the old budget error. `depth` / `position` /
  `value` / `backtrace` work while *paused* only. Post-mortem inspection
  of a failed machine looks cheap and isn't: retaining the `K` per step
  makes every `K` shared and silently defeats ADR-035's `try_unwrap`
  fast path.
- `prim.rs` — pure built-ins (arithmetic, list ops, predicates, eq?,
  condition accessors).
  Each fn-ptr is wrapped in an `Rc::new` at `initial_env` time so the
  one prim variant carries them uniformly with state-capturing host
  closures.

(`world.rs` and `world_prim.rs` lived in this crate until ADR-018
extracted them to `crates/world/`. The engine no longer ships any
host types.)
- `parse.rs` — tokenize, `read` (→ Datum), `read_many` (→ Vec<Datum>),
  `compile` (→ Expr), special forms, quasiquote compilation. Parser-
  level quasiquote (` `` `, `,`, `,@`) lives here as list-construction
  syntax — works without macros installed. `Datum` is
  `{ kind: DatumKind, span: Option<Span> }`; match on `.kind` or use the
  `as_list` / `as_sym` helpers. `read_datum` is **iterative** (an
  explicit `Open` stack for lists *and* reader prefixes) — keep it that
  way: when it recursed, `MAX_DEPTH` doubled as the native-stack guard,
  and ADR-039's wider frame made 1024 levels overflow a 2 MiB test
  thread. **Residual:** `compile` is still recursive and gives out
  between 500 and 750 levels, so the reader's 1024 cap is not one the
  rest of the pipeline can honor. Don't "fix" that by lowering
  `MAX_DEPTH`; see `core-followups.md`.
- `lib.rs` — `Vm`, top-level `define` registration. `eval_str` accepts
  a sequence of top-level forms; returns the last expression's value
  (ADR-014). `eval_datums(&[Datum])` is the same thing for callers
  that already hold read forms — `eval_str` is `read_many` plus it.
  Hosts that build forms programmatically should use it rather than
  `format!`-ing source (ADR-034). The engine is macro-unaware:
  `defmacro` lives in the sibling `macros` crate (ADR-024). Hosts
  that want macros wrap a `Vm` in `macros::MacroVm`. `globals` and
  `store` are private (ADR-036) — reach them via `store_weak` /
  `global_cell_weak`, and add a purpose-built accessor rather than
  re-exposing either field. `Vm::start` / `Vm::resume` drive a `Session`
  (a resumable batch holding no borrow of the `Vm`, so a host can park
  one between event-loop turns); `eval_datums` is implemented as
  `start_datums` plus one unbounded `resume`, so **don't reintroduce a
  second batch loop** — pre-pass, per-form budget, and rollback live in
  one place. `resume`'s `slice` and `set_step_budget` are different
  things: the slice is host pacing, the budget is the per-form runaway
  guard (ADR-040).

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
- `crates/world/` — `World` (tile grid + per-cell lifetime + event
  log) and the 6 world prims (`world-tile`, `world-set-tile!`,
  `world-log!`, `world-size`, `world-apply!`, `world-tick!`). Hosts
  wire it in via `world::world_prim::install(&mut vm,
  world.clone())`. Sibling to runes/codons; engine has no
  awareness. `world-apply!` writes per-tile lifetime from ctx
  `power` (default 5); `(world-tick!)` decrements + reverts at
  zero (ADR-027). See ADR-017, ADR-018, ADR-027.
- `crates/spells/` — rune prelude as `PRELUDE_DEFINES`. As of
  ADR-025 the prelude registers two local macros (`defspell` for
  constant ctx-setters, `defparam` for parametric ones) and uses
  them to define the rune vocabulary in nine one-liners. ADR-028
  added the mana model: caster-side globals (`max-mana`, `mana`)
  with `cast!` / `tick!` / `reset-mana!` wrappers that gate
  `world-apply!` and `world-tick!`. `install` takes `&mut MacroVm`;
  `install_with_world(mvm, world)` wires the prelude + the world
  prims. Depends on `lisp`, `macros`, and `world`. First DSL pack
  to adopt the macros stdlib pattern. Consumers
  (`examples/spells.rs`, WASM bridge) wrap a `MacroVm` and call
  `install_with_world` at startup. The WASM bridge calls `(cast!
  …)` and `(tick!)`; CLI demos that want raw world prims call
  `world-apply!` directly. See ADR-010, ADR-014, ADR-016, ADR-018,
  ADR-025, ADR-028.
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
  `render(&Turtle) -> Result<String, String>` for direct access (it
  errs rather than allocating when the bbox exceeds
  `MAX_CANVAS_CELLS` — the canvas is sparse but the render grid is
  dense, so a long diagonal spans an N×N box). `install(vm, turtle)`
  wires all of it in one call. Depends only on `lisp`. Cast pipeline is
  `(draw! (grow axiom rules n))` then `(render!)`. See ADR-019.
- `crates/macros/` — `defmacro` + procedural expansion + quasiquote-
  with-macros. `Expander` struct owns the macro table; `MacroVm`
  bundles `lisp::Vm` + `Expander` with a macro-aware `eval_str`.
  Hosts that want macros wrap their `Vm` in `MacroVm`; hosts that
  don't (the CLI demos) stay on the raw engine. Depends only on
  `lisp`. Expanded datums go to the engine via `Vm::eval_datums`;
  they used to be re-serialized to source and re-read, which lost
  any symbol the printer can't represent (ADR-034) — don't
  reintroduce a printer on this path. See ADR-024, ADR-034.
- `crates/wasm/` — JS-facing bridge (`wasm-bindgen` `cdylib`). Wraps
  `macros::MacroVm` (which wraps `lisp::Vm`) + `World` + `Turtle`,
  installs all three DSL packs at construction via
  `inner.vm`, exposes `new(width, height)`, `eval(src)`,
  `cast(tape, x, y)`, `cast_genome(tape, seed)`, `cast_breed(a, b, seed)`,
  `cast_curve(axiom, rules_sexpr, iters)`, plus `grid()` / `log()` /
  `reset_world()`. Also `eval_start` / `eval_resume` / `eval_cancel` /
  `eval_steps` (ADR-040) — the sliced, cancellable eval the web REPL
  drives from `requestAnimationFrame`. Two error paths, and a new entry
  point has to pick one: `LispErr::render` for source the *user* wrote
  (`eval`), `generated_err` for source the bridge assembled (every
  `cast*`), which strips the span because it names a line in generated
  text. Pinned to `wasm-bindgen =0.2.114` to match the
  installed CLI (ADR-009). The curve bridge expects `rules_sexpr` as a
  pre-built lisp form (the page module owns the `lhs = rhs` parser) so
  the Rust side stays domain-neutral.

Web shell at `web/` — four pages, one bundle:

- `web/index.html` — landing page. Three `.lab-card` links to the demos
  (jade-accented Spell Lab, honey-accented Gene Lab, copper-accented
  Curve Lab) over a Let-rs masthead. No WASM init on this page; it's
  pure HTML.
- `web/spells.html` — Spell Lab + REPL, two-column at ≥940px. Jade
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
  The REPL evaluates in 50k-step slices from `requestAnimationFrame`
  (ADR-040) so the page keeps painting and `cancel` works; the `cast*`
  paths stay synchronous, being bounded by construction.
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
`world`, `genes`, and `curves` depend only on `lisp`. `spells`
depends on `lisp`, `macros` (for the defspell/defparam macros baked
into its prelude — ADR-025), and `world` (for the
`install_with_world` helper). `wasm` may take deps (`wasm-bindgen`,
`console_error_panic_hook`); this is allowed by ADR-002's "lisp
stays platform-independent" caveat.

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
  `quasiquote`, `set!`, `raise`, `error`, `guard`) live in `parse.rs`.
  Everything else can be a macro. `set!` (ADR-026) is the only effecting
  form — everything else is expression-pure. `error` is a special form
  rather than a prim because a prim reports failure as a `String`, which
  would flatten its irritants into the message; it compiles to
  `(raise (list 'error …))` (ADR-041). `guard` can't be a macro at all —
  it needs a continuation frame.
- **Where state lives (ADR-037): state the host must read or render
  lives in the host; state only lisp reads lives in lisp.** `World`
  and `Turtle` follow it. The mana model doesn't — it's a lisp global
  the UI renders — and is a *grandfathered exception*, not a
  precedent; don't copy its shape for new state. The rule places the
  cell, not the model: host-side storage is compatible with the DSL
  owning its policy, exactly as `world-apply!` consumes a ctx that
  lisp vocabulary built. Hosts read lisp-side values with
  `Vm::global(name)` — never `eval_str("some-name")`.
- DSL packs install into their own namespace and return it:
  `let ns = spells::install(&mut mvm);`. Host code that evaluates *pack*
  source (a generated cast) must use `vm.eval_str_in(&ns, src)` — casts
  reference `thread`, which spells and genes both define privately.
  Root-level code reaches only a pack's `EXPORTS`. Adding public
  vocabulary means adding it to that pack's `EXPORTS` const, and the two
  names deliberately *not* exported by either spells or genes are
  `thread` and `assoc-set` (ADR-042).
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
  `crates/spells/src/lib.rs` (`PRELUDE_DEFINES`). Constant ctx
  setters use `(defspell NAME KEY VAL)`; parametric setters use
  `(defparam NAME KEY)`; *element* runes go hand-written through
  the `add-element` + `mix` helpers (ADR-030 — element runes mix
  with whatever the ctx already holds rather than overwriting);
  anything else still wants a hand-written `(define …)`. If the
  new element introduces a derived tile (e.g. fire+earth → lava),
  also extend the `Tile` enum in `crates/world/src/lib.rs` with
  `glyph()` / `from_sym()` / `as_sym()` arms. Both CLI and WASM
  consumers import from `crates/spells/`, so one prelude edit is
  enough. See ADR-025 and ADR-030.
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

Durable, reader-facing project docs live in `docs/project_notes/` (plus
`docs/style.md`) — read and update these:

**Before proposing changes**: Check `decisions.md` for existing ADRs
**When encountering errors**: Search `bugs.md` for known solutions
**When looking up config**: Check `key_facts.md` for ports, URLs, environments
**When planning engine work**: Check `core-followups.md` for the roadmap
**When asked about "world" or host coupling**: Read `host-state.md`
  for context on what `World` actually is, why it's not generic, and
  where it might go.

When resolving bugs or making decisions, update the relevant file.

**Keep internal working notes out of this repo.** Audits, bug/incident
logs, session handoffs, review transcripts, and issue/feature stubs are
process artifacts, not public reference — they live in the out-of-repo
notes archive (`project-docs/docs/let-rs/`), never committed here.
`docs/project_notes/` is only for durable docs a public visitor should
see. (Earlier such material has already been archived out of this repo.)
