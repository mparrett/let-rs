# Decisions

Architectural decision records, backfilled at the end of day one. Each entry
captures the choice as it was made and the alternatives that were on the table
at that moment. If a decision is later reversed, mark it superseded but leave
the original entry — the reasoning is part of the history.

## ADR-001: CEK abstract machine as the evaluator (2026-05-25)

**Context**: Let-rs needs an interpreter for a small functional lisp. The
choice of machine shape determines what's easy (tail calls, continuations,
debugging) and what's hard (performance, FFI), and influences every later
extension.

**Decision**: Build a CEK (Control / Environment / Kontinuation) abstract
machine (Felleisen & Friedman, 1980s) — five transition rules, one `step`
function returning a new State, a four-line `loop` driver in `run`.

**Alternatives**:
- **Tree-walking AST interpreter** — fastest to working lisp, but not really
  a VM. No continuation reification; tail calls require trampolining; you
  learn lisp semantics but not runtime craft.
- **SECD machine** (Landin, 1964) — historically the original answer for
  functional language VMs. Four registers, instruction-tape style. Slightly
  more ceremony than CEK for the same expressive power.
- **Stack-based bytecode VM** (Lua-4, CPython style) — most plumbing, closest
  to a "real" production runtime. Would have required a separate compiler
  pass and frame management.

**Consequences**:
- **+** Proper tail calls fall out for free (κ passed through unchanged on
  closure entry). Verified by `tail_calls_dont_grow_the_stack` (100k deep).
- **+** First-class continuations are essentially free if we ever add
  `call/cc` — the continuation is already a Val, capturing is one Rc clone.
- **+** No native call stack; all "stack" lives in `Rc<K>` chains on the heap.
  Trivially pausable / serializable / time-travel debuggable.
- **+** Clean upgrade path: CEK → CESK (add Store, get mutation) → eventually
  compile to bytecode by replacing `Expr` in C with a `Vec<Op>`.
- **−** Cloning Vecs in `K::App.evaled` on every arg evaluation isn't
  optimal. Not a problem at current scale; if it becomes one, switch to
  `Rc<[Val]>` or accumulate in reverse.

## ADR-002: Zero runtime dependencies for the core lisp crate (2026-05-25)

**Context**: A tiny lisp could pull in `im` for persistent maps, `thiserror`
for ergonomic errors, `logos` for tokenizing, etc. Each is reasonable in
isolation; collectively they would dwarf the engine and obscure how the
parts fit together.

**Decision**: `crates/lisp/Cargo.toml` lists no dependencies at all. Errors
are `Result<_, String>`; the tokenizer is ~60 lines of `chars().peekable()`;
the environment is a hand-rolled Rc-linked list; the macro table is a
plain `HashMap<String, Macro>`.

**Alternatives**:
- **`im` or `rpds`** for persistent collections — would replace the alist
  ctx and the Env linked-list with O(log n) structures.
- **`thiserror` / `anyhow`** for typed errors — would eliminate the
  `Result<_, String>` discipline.
- **`logos`** for the tokenizer — overkill at this scale.
- **`rustc_hash`** for faster macro lookup — premature.

**Consequences**:
- **+** The whole engine is auditable in an afternoon. New readers can hold
  the entire build in their head.
- **+** Compile times stay sub-second; no transitive vulnerability surface.
- **+** Trivial to embed (`crates/lisp` works in WASM, in tests, in a CLI,
  in anything) because it's portable Rust with no platform assumptions.
- **−** Some primitives are slower than they could be (alist lookup is O(n);
  env lookup is O(depth); error format strings allocate). All addressable
  later behind the same public API if measurement demands it.
- **−** Discipline cost: every time we want a crate we have to justify it.
  Acceptable for a learning project; revisit if a real game ships.

## ADR-003: Cargo workspace from day one, `lisp` crate first (2026-05-25)

**Context**: The end state will likely include a WASM bridge crate and a
game crate. Starting as a single crate is simpler now but means a refactor
when the second crate arrives.

**Decision**: Workspace at the repo root with `members = ["crates/*"]`,
resolver = "3", and shared edition/license. Initially only `crates/lisp/`
exists; future `crates/wasm/` and `crates/game/` will be thin shims that
depend on `lisp`.

**Alternatives**:
- **Single-crate repo, refactor later** — simpler day one, painful when
  splitting because all imports change.
- **Polyrepo** — one repo per crate. Reasonable for production teams; too
  much ceremony for a learning project.

**Consequences**:
- **+** Adding wasm/game crates becomes "create directory + Cargo.toml +
  add to members". No restructure of existing code.
- **+** Forces the discipline that `lisp` knows nothing about the browser,
  the game, or any host. It stays pure Rust + Vm + Val.
- **+** `cargo test -p lisp` runs only the engine; future game tests stay
  separate.
- **−** Slightly more Cargo.toml ceremony for a one-crate state. Negligible.

## ADR-004: Env slots as `Rc<RefCell<Val>>` (2026-05-25)

**Context**: A pure-functional env (slots store `Val` directly) is the
natural first instinct. But `letrec` requires that each binding be in
scope before its init expression runs, so closures created during init
can capture the recursive environment.

**Decision**: Every env slot is an `Rc<RefCell<Val>>`. `Env::extend(name,
val)` wraps the value internally and looks indistinguishable from a pure
binding. `Env::extend_placeholder(name)` returns `(Env, Rc<RefCell<Val>>)`
so the caller can patch the cell once the init evaluates.

**Alternatives**:
- **Y-combinator desugar** for letrec — works for single bindings, breaks
  down for mutual recursion (Y\* exists but is uglier than the engine
  change).
- **Separate `EnvRec` variant** with mutable slots, alongside an immutable
  `Env` — adds a layered API for negligible benefit.
- **Two-pass compile** that lifts letrec to a top-level mutual definition —
  doesn't compose with nested letrec.

**Consequences**:
- **+** Mutual recursion works (`letrec_mutual_recursion` test: even?/odd?
  defined in terms of each other).
- **+** Pays for itself again when `set!` arrives — the cells are already
  there, ~15 lines to expose mutation as a primitive.
- **+** Pure-functional usage is observationally unchanged. No external
  code writes to the cells except letrec, so the lisp stays observably
  immutable.
- **−** One `RefCell::borrow` on every variable lookup. Cheap (no
  contention; single-threaded) but not free.
- **−** A bug that writes to the wrong cell would corrupt the env. Mitigated
  by keeping the only writer being the letrec K-frame handler.

## ADR-005: Two-tier primitives — `Val::Prim` (pure) and `Val::WorldPrim` (host-aware) (2026-05-25)

**Context**: World-aware primitives need mutable access to the host's
world state. Pure primitives (arithmetic, list ops) don't. Forcing one
signature on both is a Hobson's choice.

**Decision**: Two `Val` variants. `Val::Prim` has `f: fn(&[Val]) -> R`,
unchanged from the original pure design. `Val::WorldPrim` has
`f: fn(&[Val], &mut World) -> R`. `apply` in `step.rs` borrows
`world.borrow_mut()` only for the WorldPrim branch.

**Alternatives**:
- **Single `Prim` variant taking `&mut World`** — would force every existing
  prim (22 of them) to add an unused `_w: &mut World` parameter, and every
  call site to acquire the borrow even when unused.
- **`thread_local!` world state** — minimal engine change but hides a
  global from the language POV; re-entry into the Vm from within a prim
  would break.
- **Host trait passed through `step`** — most generic, most plumbing. May
  matter later when we want sound, input, time, RNG; not yet.

**Consequences**:
- **+** Pure prims stay pure in signature, callable in tests without a
  World. The split is the type-system enforcing "this primitive can have
  side effects on the host".
- **+** `state.rs` doesn't carry the world; only the two apply helpers do.
- **+** Easy to grep for "what touches the world": just find `WorldPrim`.
- **−** Two variants where one might do. When the host context grows
  (sound, RNG, time), we'll likely refactor toward a `Host` trait — but
  the current shape is small enough to refactor cheaply.

## ADR-006: Procedural macros (Common Lisp style), unhygienic (2026-05-25)

**Context**: A real lisp needs macros — at minimum to express `->`
(thread-first), `when`, `unless`, eventually `defspell`. Two families:
template-based hygienic (Scheme's `syntax-rules`) or procedural
(Common Lisp's `defmacro` + `gensym`).

**Decision**: Procedural. A macro is a regular closure that runs at
compile time, receives the raw s-expression arguments as quoted Val data,
and returns a new s-expression. Built-in quasiquote (`\``, `,`, `,@`) for
ergonomic template construction. No hygiene; macros that introduce
bindings can shadow user code.

**Alternatives**:
- **`syntax-rules`** — hygienic, pattern-based, safer. Implementation is
  ~3x bigger (pattern matcher, scope tracker, renaming). Doesn't express
  things like recursive `->` expansion cleanly.
- **Template-only**, no eval — variable substitution into a template. Even
  smaller than syntax-rules but can't express conditional or variadic
  expansion (`->` needs recursion).
- **No macros**, syntactic forms only — would force every DSL feature into
  Rust as a special form. Defeats the "syntax is a library" goal.

**Consequences**:
- **+** Macros can do arbitrary compile-time computation. `->` is defined
  in lisp in 7 lines (`tests/eval.rs::macro_thread_first`).
- **+** Compatible with the rest of the engine — a macro is just a closure;
  expansion is just calling it; no new value kind needed.
- **+** Quasiquote turns 20-line cons-soup macros into 1-line templates.
- **−** Variable capture is possible. Documented; in practice the macros
  we've written are hygiene-safe by construction.
- **−** Macros must be defined before use (no forward references) and
  registered at top level only (no nested `defmacro` inside a let).
  Standard restriction; explicit error if violated.

## ADR-007: Spell DSL split — runes in Rust, primitives in lisp, resolver in Rust (2026-05-25)

**Context**: The spell DSL has three layers: (1) the rune tape (Unicode →
primitive name + numeral), (2) the spell pipeline (primitives that thread
a ctx), (3) the resolver (turns the final ctx into world state changes).
Each could live in Rust or in lisp; the split shapes what's testable
where and how big the engine grows.

**Decision**:
- **Rune translation: Rust.** Two `&[(char, &str)]` tables (plain vs.
  parametrized) + a ~70-line tokenizer/resolver in `examples/spells.rs`.
  Purely lexical; no runtime state.
- **Spell primitives: lisp.** Each rune name binds to a closure
  (`(lambda (ctx) (assoc-set 'element 'fire ctx))`) in a user-level
  prelude. Adding a new spell primitive means writing 3 lines of lisp.
- **Resolver: Rust.** `(world-apply! ctx)` is a `WorldPrim` that reads
  the final ctx and mutates the world.

**Alternatives**:
- **Everything in Rust** — fastest to ship, but kills the "game IS the
  language" payoff. Adding a spell becomes an engine change.
- **Everything in lisp** — runes parsed in lisp, world ops accessed via
  primitives. Cleaner conceptually but the rune translation has no
  reason to be runtime — it's a lex-time substitution.
- **Resolver in lisp** — would push world reads/writes through individual
  primitives (`tile-set!`, `damage!`). Possible, but `world-apply!` as one
  primitive keeps the demo small and makes the world-mutation surface
  obviously discoverable.

**Consequences**:
- **+** New primitives are 3 lines of user-level lisp. The engine doesn't
  learn about `bolt`, `area`, `power`, `sun`, etc. — they're vocabulary.
- **+** The spell pipeline is pure: returns a final ctx, replayable,
  testable in isolation. The resolver is the only side effect, and
  it's one named primitive.
- **+** Emergent combinations (a ctx with both `element=fire` and
  `element=ice`) become a resolver-level decision, not a parser-level
  rejection. Same DSL, different resolution strategies.
- **−** Two surfaces to learn (rune table in Rust; primitive closures in
  lisp). Acceptable since they're a few lines each.

## ADR-008: ctx as alist for v0; persistent map deferred (2026-05-25)

**Context**: The spell ctx accumulates ~5–10 key-value pairs (element,
target-x/y, area, power, …). Could be a Rust-backed persistent map, an
alist of `(key . value)` cons pairs, or a closure-of-bindings.

**Decision**: Alist of cons cells, manipulated with user-level
`assoc-set` and (eventually) `assoc-get`. No new Val variant.

**Alternatives**:
- **`Val::Map(Rc<BTreeMap<…>>)`** — O(log n) lookup, requires adding a Val
  variant + ~4 map primitives. Faster but bigger surface.
- **`Val::Map(im::HashMap<…>)`** — would force an `im` dependency, violating
  ADR-002.
- **Records / structs** — type-safe but loses the "ctx is dynamic, runes
  can add arbitrary keys" property that the design hinges on.

**Consequences**:
- **+** Zero engine work. Demonstrated end-to-end in the spell demo with
  the existing cons/list/eq? primitives.
- **+** Last-write-wins is naturally observable: both an old and new
  binding linger in the alist, and a resolver can choose to see both
  (the emergent-combinations payoff).
- **−** O(n) lookup. At n=10 it's irrelevant; if a future spell has 50
  keys the alist will hurt.
- **−** No real "remove key" — you can only shadow with a new binding.
  Acceptable since ctx is single-pass.
- **Reversibility**: switching to `Val::Map` later is a ~40-line patch
  with a compatibility shim if needed.

## ADR-009: Raw `wasm-bindgen` CLI over `wasm-pack` (2026-05-25)

**Context**: The WASM bridge needs a build step that turns the cdylib output
into JS-loadable artifacts (`.js` glue + `.wasm` + `.d.ts`). Two standard
tools do this: `wasm-pack` (a higher-level orchestrator that also handles
npm packaging) and `wasm-bindgen-cli` (the lower-level bindings generator
that `wasm-pack` itself shells out to).

**Decision**: Use the raw `wasm-bindgen-cli` directly. The justfile recipe
is a two-step `cargo build --target wasm32-unknown-unknown --release` +
`wasm-bindgen --target web --out-dir web/pkg`. The `wasm-bindgen` crate is
pinned with `=0.2.114` in `crates/wasm/Cargo.toml` to match the installed CLI;
version drift between the two produces a confusing-but-obvious error at
bindgen time.

**Alternatives**:
- **`wasm-pack`**: the more standard tool, generates a `package.json` and
  is npm-friendly. Adds an install step (`cargo install wasm-pack`) for any
  new reader of the repo. Not currently installed on this machine.
- **`trunk`**: bundles HTML/CSS/JS too. Opinionated; obscures the bridge
  ↔ JS module boundary that's pedagogically interesting.
- **`wasm-pack` via Docker**: portability without local install. Overkill
  for a learning project.

**Consequences**:
- **+** One fewer tool to install. `wasm-bindgen-cli` was already present.
- **+** Clearer pedagogically — the `cargo build` then `wasm-bindgen`
  two-step shows what's happening at each stage; `wasm-pack` would hide it.
- **+** The output (`web/pkg/{wasm.js, wasm_bg.wasm, wasm.d.ts}`) is the
  same regardless of which tool produced it, so we can switch to `wasm-pack`
  later without rewriting the JS shell.
- **−** No `package.json` — can't `npm publish`. Not on the roadmap.
- **−** Version pinning friction: bump the `=0.2.114` and the
  `cargo install` in parallel; the error message when they drift is clear
  but the discipline is manual.

## ADR-010: Rune translation in its own `crates/runes/` micro-crate (2026-05-25)

**Context**: ADR-007 split the spell DSL into three layers (rune tape →
primitives → resolver) with each layer living in a different language /
crate. Day-one had the rune translator privately inside
`crates/lisp/examples/spells.rs`. When the WASM bridge arrived it needed
the same translator, forcing a decision: duplicate the code, put it inside
the lisp crate, or extract a third crate.

**Decision**: Extract into `crates/runes/` — a zero-dependency micro-crate
(~90 LOC) exposing `pub fn tape_to_sexpr(tape: &str) -> Result<String,
String>` plus the `PLAIN`/`PARAM` rune tables. Both the CLI example
(`crates/lisp/examples/spells.rs`, via dev-dep) and the WASM bridge
(`crates/wasm/`, via runtime dep) consume it.

**Alternatives**:
- **`pub mod runes` inside `crates/lisp/`**: simpler — one fewer Cargo.toml.
  But the lisp crate would carry a DSL-specific module that nothing in the
  language uses. Mild ADR-007 violation; future readers would wonder why
  `lisp::runes` exists alongside `lisp::Vm`.
- **Duplicate in `crates/wasm/src/runes.rs`**: smallest diff today but
  introduces two sources of truth for the rune table. Drift was certain.
- **Inline in `crates/wasm/` only**, deleting the CLI example: would lose
  a useful reference for what the rune surface looks like at the
  command line.

**Consequences**:
- **+** Honors ADR-007's layering explicitly. Each layer has its own home.
- **+** Both consumers see the same `PLAIN`/`PARAM` definitions; adding
  a new rune is a one-line change in one place.
- **+** `runes` stays zero-dep — `cargo test -p runes` is fast and
  isolated.
- **+** Sets the pattern for future DSL layers (e.g. a hypothetical
  `crates/spells/` that owns the prelude prelude as `.lg` source files).
- **−** One more `Cargo.toml`, one more workspace member to compile.
  Negligible at this scale.
- **−** Slight indirection: changing `(area 3)`'s rune now requires editing
  `crates/runes/src/lib.rs` rather than the example. Documented in
  `CLAUDE.md`.

**Follow-up landed (2026-05-25)**: the rune *prelude* (the lisp source
defining `assoc-set`, `thread`, `start`, `fire`, `ice`, etc.) was
extracted into `lisp::spells::PRELUDE_BINDINGS`, mirroring the
`lisp::genes` pattern from ADR-011. The wasm bridge's previous local
`SPELL_PRELUDE_BINDINGS` const is gone; coord seeding for `(world-apply!
…)` moved out of the prelude into a call-site `assoc-set` wrap, which
keeps `start` zero-arg and bit-identical across CLI + WASM. Closes the
"two prelude copies will eventually consolidate" caveat above.

## ADR-011: Genes demo — codon tape, diploid-by-accumulation, host-side phenotype resolver (2026-05-25)

**Context**: The runes/spells demo proves the mini-lisp can host a real
vocabulary on top of the CEK engine. The open question was whether the
three-layer pattern (tape → lisp pipeline → host resolver) *generalized*
or just happened to fit magic. Genetics is structurally different from
spells in three load-bearing ways: alleles are diploid (two values per
locus, not one), numeric traits average rather than last-write-wins, and
categorical traits resolve by Mendelian dominance rather than simple
overwrite. If the same architecture absorbs all three twists with no
engine change, the pattern is real. Sibling project `../kaiju-elements`
provided the concrete genetic model (`Gene { value, dominant }` diploid
pairs, numeric averaging, Mendelian categoricals, biome affinities).

**Decision**: Mirror the runes/spells slice with three deliberate twists:

1. **Codon tape.** RNA-style ASCII triplets (`AUG CGA UUA UGA`) in a new
   zero-dep `crates/codons/` micro-crate parallel to `crates/runes/`.
   Codons don't take following numerals — the allele payload is baked
   into the table fragment — so the lexer is whitespace-split +
   triplet-validate. Kaiju doesn't actually use codons; we added them as
   the cleanest possible parallel to runes (one tape symbol → one
   binding call) and because biology gives us `AUG`/`UAA`/`UGA` as
   ready-made start/stop anchors.
2. **Diploid by accumulation in the ctx.** Stating two codons for the
   same trait stacks two alleles in a per-trait list under that key. A
   new ctx idiom (list-per-key) that contrasts with spells' flat
   key/value alist. Fragmentary genomes still express what they have.
3. **`express!` as a pure `Val::Prim`** registered by the example via
   a new `Vm::register_prim` helper (~5 lines added to
   `crates/lisp/src/lib.rs`). The resolver walks the genome ctx,
   averages numerics, runs Mendelian dominance on categoricals
   (deterministic tiebreak from FNV hash of the genome string), and
   returns a phenotype alist. ASCII creature-card rendering happens in
   Rust on the example side.

**Alternatives**:
- **Allele-as-tape** (one tape token = one fully-formed allele, no codon
  indirection): rejected. Codons are the parallel to runes (single
  symbol → binding call); the start/stop biology nod is essentially
  free; "codon → allele → trait" is a story worth telling.
- **Diploid as explicit pairs in tape** (e.g. `pair(AUG, AUG)`):
  rejected. Accumulation in the ctx is the more lispy answer and lets
  fragmentary genomes express. The cost is permissiveness (N>2 alleles
  per locus possible) which the resolver handles by taking the first
  two.
- **`express!` in the lisp crate** (next to `world_prim.rs`): rejected
  for now. World is a shared host concept (CLI + WASM); creatures are
  demo-local. Promote later if a second consumer appears.
- **`express!` in pure lisp**: rejected. Phenotype resolution +
  Mendelian dominance + deterministic hashing is cleaner in 40 lines of
  Rust than 80 lines of lisp, and the structured `Val` output threads
  naturally to the Rust-side card renderer.
- **`crates/codons/` as a second table inside `runes/`**: rejected per
  ADR-010's logic. Sibling DSLs each get their own table — keeps
  `cargo test -p codons` isolated and the "extending the rune table"
  story uncluttered.
- **Categorical payloads unquoted** (e.g. `(color green dom)`):
  rejected — `green` resolves as a variable lookup and bombs. Quoted
  form (`(color 'green dom)`) keeps the prelude small (no need to bind
  every color/ability/biome symbol as a self-referential variable).

**Consequences**:
- **+** Vocabulary swap validated end-to-end. The CEK engine, the
  parser, and `crates/lisp/src/*` (except the 5-line `register_prim`
  helper) are untouched.
- **+** New ctx idiom (list-per-key for diploids) demonstrated; future
  DSLs that need multi-value-per-key state have a pattern to borrow.
- **+** `codons` is zero-dep and self-contained (`cargo test -p codons`
  runs in <1s).
- **+** `Vm::register_prim` is generally useful — any future example
  that needs a custom pure prim can use it.
- **−** Now two example-local preludes (spells, genes). The "two copies
  in sync" warning from ADR-010 doesn't apply yet (no WASM exposure for
  genes), but if/when genes goes to WASM, the duplicate-prelude
  bookkeeping starts.
- **−** The `(color 'green dom)` quote-symbol convention is a sharp
  edge — easy to forget when adding a new categorical codon. Documented
  in `CLAUDE.md`'s "Adding a new codon" paragraph and protected by the
  fact that an unquoted symbol bombs loudly at eval time.

**Deferred to future slices** (each their own ADR):
- Mutation primitive `(mutate 0.05)` — needs RNG (thread-local?
  WorldPrim against a per-Vm seed?), a non-trivial decision.
- Breeding primitive that combines two genomes per Mendelian
  segregation. Trivial in lisp once we have the ctx representation;
  the question is whether breeding belongs in the pipeline or the
  resolver.

**Follow-up landed**: WASM exposure as a Gene Lab panel (2026-05-25,
same day). The "promote to lisp crate when a second consumer appears"
clause fired immediately: rather than duplicate the genome prelude,
`express!` prim, and creature renderer into `crates/wasm/src/lib.rs`,
we hoisted them into a new `lisp::genes` module alongside `world.rs` /
`world_prim.rs`. Both `examples/genes.rs` (slimmed to driver code)
and the WASM bridge import from `lisp::genes`, so unlike the spells
demo there is no "two prelude copies to keep in sync" warning. The
WASM bridge gains a single `cast_genome(tape: &str) -> String` method
that returns the rendered card; the web shell adds a Gene Lab panel
below the existing two-column Spell Lab / REPL layout with a 20-button
codon palette color-coded by trait category. Bundle grew ~19 KB
(104 KB → 123 KB).

## ADR-012: Mutation primitive — seeded xorshift, codon-style, lexical-scope seed (2026-05-25)

**Context**: ADR-011 deferred the mutation primitive with the note
"needs RNG (thread-local? WorldPrim against a per-Vm seed?), a
non-trivial decision." With the genes demo otherwise working, the
question matured: what shape should `(mutate …)` take so it composes
with the codon-tape style and stays consistent with let-rs's "same
input → same output" flavor everywhere else (pure CEK eval, FNV
deterministic tiebreaks, no global state)?

**Decision**: A single `mutate!` host prim with three arguments —
`(rate-percent, seed, ctx)` — registered alongside `express!` by
`genes::install`. The seed is **explicit, caller-provided, integer**;
the RNG is **xorshift32** (4 lines, dep-free); mutations are
**pure**: same `(rate, seed, ctx)` always produces the same output.

Surface choices:
- **Codon-style trigger.** A single `MUT` codon emits the symbol
  `mutate`, bound in the prelude to `(lambda (ctx) (mutate! 25 seed
  ctx))`. The rate (25%) is baked into the codon's prelude binding —
  not into the codon itself — so adding higher- or lower-rate codons
  later is a one-line addition.
- **Seed comes via lexical scope.** Drivers wrap the prelude in
  `(let ((seed N)) …)`. The `mutate` closure captures `seed` in its
  closure env; calling `(mutate ctx)` evaluates `(mutate! 25 seed ctx)`
  against the outer binding. The CLI passes a per-sequence seed; the
  WASM bridge's `cast_genome(tape, seed)` gains a second arg; the web
  shell adds a number input + "evolve →" button that increments the
  seed and re-expresses.
- **Mutation rules mirror kaiju.** Per-allele Bernoulli at `rate%`,
  then ±10 drift clamped `[0,100]` for numerics, swap-to-other-pool
  for categoricals (which means we extended `Kind::Categorical(&[&str])`
  to carry an option pool per categorical trait). `dom`/`rec` flag is
  preserved across mutation — only the value changes.
- **Default rate raised from kaiju's 5% to 25%** for demo visibility.
  At 5% over 7 alleles, ~70% of casts are no-ops; at 25% it's ~84%
  *at least one* visible drift. The kaiju rule is the only thing
  load-bearing here; the rate is an art choice for an interactive
  demo and is documented in the prelude binding's comment.

**Alternatives considered**:
- **True random (non-seeded)** — rejected. Breaks "same input → same
  output", makes tests flaky, and the web shell's "evolve →" button
  loses meaning if every click is unrepeatable. Seeded is more let-rs-y.
- **Per-Vm seed via a `seed!` prim** (`(seed! 42)` once, then `(mutate
  ctx)` consumes the next number) — rejected. Adds mutable RNG state
  to `Vm` for one consumer's benefit. Lexical scope via outer `let`
  achieves the same without polluting the engine.
- **Hash-of-genome as seed** — rejected per the user's note that it
  defeats the point ("same genome → always same mutation").
- **Time-based seed** — rejected. Non-reproducible; would force a
  `js_sys::Date::now()` dep in WASM that the bridge doesn't otherwise
  need.
- **`mutate!` as a `WorldPrim`** holding an RNG seed in the World —
  rejected. World is the spell demo's concept; the gene demo
  shouldn't reach into it. The pure `Val::Prim` shape (seed-in /
  ctx-out) keeps genes independent of `world.rs`.
- **Floating-point rate** (`(mutate 0.05 …)`) — rejected. This lisp
  is integer-only; adding floats for one prim is a large scope change.
  Integer percentage is fine.
- **Mutation as a separate (non-codon) prim call** — rejected. The
  user prefers codon-style so the demo's full pipeline still reads as
  a tape. The codon (`MUT`) is the trigger; the prim is just plumbing.
- **Always-different categorical (force a flip)** vs **random pick
  from pool (might land on current value)** — chose force-flip. A
  mutation event that returns the same value isn't visibly a mutation,
  which would mislead readers. With 2-option pools today it's a
  deterministic flip; with larger pools later it would be random-pool-
  minus-current.

**Consequences**:
- **+** Mutation is composable with the existing pipeline — `(MUT)`
  just goes anywhere in a tape, before `UAA`, and the `express!`
  resolver runs over the mutated ctx.
- **+** Five new tests in `tests/express.rs` lock the contract:
  same-seed determinism, cross-seed difference, numeric bounds,
  categorical pool membership, no-MUT-no-effect.
- **+** Engine still untouched. The `Vm::register_prim` helper from
  ADR-011 is reused for `mutate!`. The CEK rules / step / k / env
  files have no idea genes or mutation exist.
- **+** Seed plumbing via lexical `let` works because the lisp has
  real closures and lexical scope. This is a small but satisfying
  proof that the engine's design (env capture in `Val::Clo`) carries
  weight beyond the basic spell demo.
- **−** The `MUT` codon makes the codon table technically aware of
  the prelude binding name `mutate`. Same coupling already exists for
  `start`/`stop`. Documented in the codon-table comment.
- **−** Existing example helpers in `tests/express.rs` had to grow a
  seeded variant; the base helper now wraps with `(let ((seed 0)) …)`
  unconditionally. Harmless for non-MUT tapes (seed is unreferenced),
  but tests must be careful to pass a real seed when MUT is present.

**Deferred**:
- **Breeding** (combine two genomes per Mendelian segregation) — still
  open. The lexical-seed pattern from this slice transfers directly:
  `(breed! seed parent-A parent-B)` would be a sibling prim. Decision
  point is whether the seed comes from the same `seed` binding or
  whether breeding gets its own.
- **Multi-rate MUT codons** (`M01`/`M10`/`M50`) — easy to add; punted
  until a use case appears.
- **Mutation rate exposed in the Gene Lab UI** (slider 0-100%) —
  currently fixed at 25%. Could wire a second number input
  symmetrically to the seed input.

**Follow-up landed (2026-05-25, same day)**: multi-rate MUT codons
shipped — `M01` (1%, kaiju-match), `M10` (10%), `M50` (50%) live
alongside the default `MUT` (25%). Each maps to a separate prelude
binding that calls `mutate!` with a different rate. Trivial one-line
additions; no design wrinkles.

## ADR-013: Breeding primitive — Mendelian segregation across two genomes (2026-05-25)

**Context**: With mutation working (ADR-012), the next obvious move
was crossing two genomes. Kaiju does this with breeding pairs that
produce offspring inheriting one allele per locus from each parent.
The question was how to fit "takes two inputs" into a model that has
mostly been about "one tape → one creature."

**Decision**: A `breed!` host prim with signature `(seed parent-A
parent-B) → child-genome`. Pure: same `(seed, A, B)` → same child.
Parents are arbitrary genome ctxs (whatever any pipeline produces),
not codon tapes — the prim doesn't know about codons. Drivers
compose the two parents (typically two `(thread '() (list …))`
expressions, one per parent tape) and pass them in. The child is
itself a genome ctx, so it can be `express!`'d, `mutate!`'d, or
re-bred with another genome — fully composable.

Mendelian rule:
- For each trait present in either parent, the child receives one
  random allele from each parent that has the trait.
- A parent with 0 alleles for a trait contributes nothing — the
  child is haploid for that trait (one allele, from the other
  parent).
- A trait missing from both parents is missing from the child.
- Trait order in the child preserves parent-A's order, then appends
  parent-B's unique traits.

Surface details:
- **No new codon for breeding.** A `XBR` (cross-breed) codon would
  need to act as a tape delimiter — single-tape semantics break. Two
  inputs is fundamentally a two-tape operation.
- **Driver composes both pipelines.** The CLI example gets a
  `breeding(vm, label, seed, tape_a, tape_b)` helper; the WASM bridge
  gets `cast_breed(tape_a, tape_b, seed) → String`; the Gene Lab gets
  a "Breeding Pen" collapsible below the express UI with a parent-B
  input + breed button.
- **Mutation stays orthogonal.** `breed!` does NOT apply mutation
  itself. Callers wanting drift on top do `(express! (mutate (breed!
  seed A B)))` or include `MUT` in one of the parent tapes (which
  would mutate that parent's alleles before breeding — a different
  semantics, intentionally allowed).
- **Seed source reuses ADR-012's lexical-`seed` pattern.** The
  prelude has no `breed` closure equivalent to `mutate` because
  `breed!` is always invoked from driver-composed lisp, not a single
  codon — so the prim takes `seed` explicitly as its first arg
  rather than via lexical capture. (The driver still wraps in `(let
  ((seed N)) …)` so `MUT` continues to work alongside breeding.)

**Alternatives considered**:
- **`breed!` as a per-Vm operation taking two tape strings** —
  rejected. Couples the prim to the codon layer; loses the
  composability of "any genome ctx is a valid parent." Today, a
  caller can breed two manually-constructed alists or a mutation
  result with a hand-built genome.
- **`XBR` codon as tape delimiter** — rejected. Single-tape lex stays
  simple; a delimiter would force the codon crate's parser to
  understand multi-genome shape.
- **Crossover within a chromosome** (multi-locus blocks inherited
  together) — out of scope. We don't model linkage; each locus
  segregates independently, which is the textbook simplification.
- **Mutation as part of breeding** — rejected on the orthogonality
  argument. Keeping the two prims separate means each is one job;
  the caller composes.

**Consequences**:
- **+** Genomes are now first-class values you can pass around,
  combine, and re-feed into the pipeline. The genes demo gains a
  recursive structure (express ∘ breed ∘ mutate ∘ thread) it didn't
  have before.
- **+** Five new tests in `tests/express.rs` lock the contract:
  same-seed determinism, cross-seed variation, trait union, trait
  intersection of absence, child phenotype that differs from both
  parents (the diploid-averaging case).
- **+** Engine still untouched. The `breed!` prim reuses the helpers
  added for `mutate!` (`collect_first_pairs`, `unpack_pairs`,
  `xorshift32`, `traits_to_genome_ctx`).
- **−** The codon table now has four MUT variants but no breeding
  codon — the demo has an asymmetry between "things you do with one
  parent" (a codon) and "things you do with two parents" (a driver
  call). Documented in the cheatsheet; the Breeding Pen UI is the
  user-facing answer.
- **−** A subtle: invoking `breed!` from inside a `thread` body would
  re-enter the prim chain mid-flight, which our prelude doesn't
  attempt. Today `breed!` is always at the top level (wrapping a
  `express!`); composing it inside thread closures would need a
  re-shape of the prelude.

**Deferred**:
- **Generation tracking.** Kaiju records `generation = max(parents) +
  1`. Could add a `generation` trait that breed increments. Punted.
- **Per-parent gamete bias** (e.g., recessive-allele suppression in
  meiosis). Pure simplification; not load-bearing for the demo.
- **Multi-rate breed variants** (e.g., asymmetric inheritance
  probabilities) — no use case; deferred.

---

## ADR-014: Installable preludes via top-level `define` (2026-05-26)

**Context**: Both DSL demos (spells, genes) were shipping their
preludes as `(letrec ((…))` open-bindings strings that consumers
appended a body and a closing paren to. Every `cast()` re-tokenized,
re-expanded, re-compiled, and re-evaluated ~25 lines of prelude just
to evaluate a one-line body. The lisp had no way to "install" bindings
into a `Vm`'s env in place — every form had to be wrapped in some
expression. With WASM in the loop, this per-call parsing was the
dominant cost. The `core-followups.md` plan's #1 item.

**Decision**: Two coupled engine additions, plus a prelude reshape:

1. **`Vm::eval_str` accepts a sequence of top-level forms.** Built on
   `parse::read_many`. Each form is either `(defmacro …)` (registers a
   macro, no value), `(define name body)` (extends `self.env` in
   place, no value), or any expression (compiled + run normally). The
   return value is the last expression's value, or `#t` if every form
   was a `defmacro`/`define`. Subsequent `eval_str` calls see all
   prior `define`s and `defmacro`s.

2. **Top-level `(define name body)`.** Mirrors how `defmacro` is
   detected and side-effects `self.env` at the top level. Uses
   `Env::extend_placeholder` (already present, originally for
   `letrec`) so a lambda body can refer to its own name — single-
   binding self-recursion works: `(define f (lambda () (f)))`.
   Mutual recursion *across* separate defines does **not** work — the
   first `define` runs before the second exists, so its lambda
   captures an env without the second binding. Users wanting mutual
   recursion wrap the group in `letrec`.

   `define` is rejected anywhere other than the top level (mirroring
   `defmacro`'s rejection), so silent mis-registration inside a `let`
   or `lambda` body fails loudly.

3. **Per-DSL `install(vm)` entry points.** Each DSL crate exposes:
   - A `PRELUDE_DEFINES` const — a sequence of `(define …)` forms.
   - An `install(vm)` function that registers any host prims AND
     evaluates `PRELUDE_DEFINES`, leaving the DSL vocabulary baked
     into the Vm's env.

   Consumers call `install` once at Vm construction, then each cast
   is just the per-cast body — no prelude string carried per call.

4. **Genes' seed-dependent half stays per-cast.** ADR-012's lexical-
   seed pattern (`mutate` reads `seed` from its captured env) is
   load-bearing. Installing `mutate` once at Vm construction would
   capture the install-time env, which has no `seed` — a runtime
   `(let ((seed N)) (mutate ctx))` cannot reach into a pre-built
   closure to inject one (lexical scope, working as intended).

   Resolution: `genes::install` installs the 14 seed-independent
   bindings (dom, rec, assoc-set, thread, start, stop, add-allele,
   size/strength/speed/armor/color/ability/biome) as defines. The
   four seed-dependent mutate variants (mutate, mut01, mut10, mut50)
   are re-bound per cast by `genes::seeded(seed, body) -> String`,
   which produces a `(let ((seed N)) (let ((mutate …) (mut01 …) (mut10
   …) (mut50 …)) body))` wrapper. The closures still capture seed
   lexically; ADR-012 still holds; ~85% of the per-cast prelude
   parsing is gone.

**Alternatives considered**:
- **`Vm::install_prelude(src)` as a separate API surface from
  `eval_str`.** Rejected as redundant once `eval_str` understands
  top-level forms. `install_prelude(src)` is just `eval_str(src)`
  where `src` happens to be all defines; making it a distinct method
  would just split the same machinery into two doors.
- **Implicit two-pass evaluation across all top-level defines** (so
  mutual recursion works without `letrec`). Rejected on
  YAGNI / Scheme-tradition grounds. R5RS top-level defines are
  *not* generally mutually recursive between separate top-level
  forms; they are within a single `define` if the body is a lambda
  (the self-cell trick). Adding multi-define mutual recursion would
  cost a pre-scan pass and a documented "your define ran out of
  order" edge case, with no demo needing it. If a future demo wants
  it, `letrec` already provides it inline.
- **Move seed to a host-side mutable cell + `current-seed` prim.**
  Would let mutate install once and ~100% of the prelude parsing go
  away. Rejected — breaks ADR-012's choice of lexical-scope-as-
  parameter-passing and introduces hidden mutable state for one demo's
  convenience. The 85% / 100% tradeoff is small; the architectural
  cost is real.
- **Drop the `letrec`-wrapping prelude pattern entirely without
  adding `define`.** Could have made `install(vm)` build the bindings
  in Rust by walking the letrec source and calling `env.extend` per
  binding. Rejected as a worse version of the same idea — it
  duplicates evaluation logic in Rust that already exists in the
  CEK loop, and it doesn't help anyone wanting to write a `.scm`
  file of top-level defines.

**Consequences**:
- **+** `Vm` is now usefully stateful in a *lispy* way: a DSL is "a
  const string + an `install(vm)` function." Two lines to add a new
  DSL pack to a Vm. The "Vm + DSL pack" concept core-followups.md
  predicted is now load-bearing.
- **+** Per-cast eval is much leaner. Spell cast: one `(world-apply!
  …)` form, ~5 lines down from ~30. Genome cast: the body plus the
  per-cast `(let …)` wrapper, ~8 lines down from ~35. The actual
  tokenize/expand/compile cost scales with what's left.
- **+** `eval_str` is more useful generally. A user can now feed it a
  `.scm`-style source with defines + a result expression and get the
  result back. The CLI repl, web repl, and tests all benefit.
- **+** Six new engine tests in `eval.rs` lock the contract: multi-
  form return value, define-extends-env, self-recursion, define
  persistence across calls, mixed define+expression sequencing,
  top-level-only enforcement.
- **−** WASM bundle grew ~10 KB (133 → 143 KB raw). One-time bundle
  cost for per-call win.
- **−** Mutual recursion across separate defines is a sharp edge —
  someone will hit it. Mitigated by clear error path ("unbound
  variable: foo") + the doc on `eval_str` saying "wrap in letrec for
  mutual recursion." (Update 2026-05-26: pre-pass scan added in the
  same session; mutual rec now works freely within a single
  `eval_str` call. Cross-call rec was still a sharp edge, by design.
  Update 2026-05-26 #2: superseded by ADR-015 — cross-call mutual
  recursion now works too, as a side effect of routing top-level
  bindings through a Vm-owned globals table.)
- **−** `genes::seeded` is now a third place (after `PRELUDE_DEFINES`
  and the `install` registration) where the seed-related prelude
  lives. Mitigated by the function being the *only* place those four
  bindings exist as source — there's no duplication, just a split
  between "installable" and "per-cast" halves of the same vocabulary.

**Deferred**:
- ~~**Multi-define mutual recursion via two-pass scan.**~~
  **Resolved 2026-05-26.** `eval_str` now does a pre-pass that
  allocates placeholder cells for every top-level `define` in a
  single source string before any body runs, so siblings in the
  same batch can refer to each other freely. (Update: ADR-015
  also fixed mutual rec across separate `eval_str` calls — the
  shared globals table means a closure looking up a forward
  reference at call time finds whatever's in the table *then*.)
- **`.scm` file loading.** Now trivial — `vm.eval_str(read_to_string("foo.scm"))`
  already works. A `Vm::load(path)` helper would just be sugar.
- **Macro-aware `define`.** Today `(define name body)` runs `body`
  through `expand_all` before compile, so macros inside the body
  work. But `(define foo (defmacro …))` doesn't, because `defmacro`
  isn't an expression that produces a value. If a future DSL needs
  macros generated from data, this is the seam to look at.



## ADR-015: Top-level defines in a Vm-owned globals table (2026-05-26)

**Context**: ADR-014 routed top-level `define` through
`Env::extend_placeholder`, putting each binding in a fresh env frame
on `self.env`. This worked but built a permanent `Rc` cycle per
prelude closure: env-frame slot → `Val::Clo` → captured env →
that same frame chain → slot. Closures captured the env *containing
their own cell*. Spells and genes preludes installed ~30 + ~14 such
cycles per `Vm::new()` + `install(vm)`. Costless at REPL scale,
visibly noisy under Criterion (every iter built+leaked a fresh Vm
worth of closures), and a slow leak in long-lived web sessions that
re-installed the prelude without dropping the Vm. The fix had to
break the cycle without losing the property that a closure can refer
to its own name (`(define f (lambda () (f)))`) or to a sibling
define (mutual recursion).

**Decision**: Split top-level bindings off the env frame chain into a
`Vm`-owned `Rc<RefCell<HashMap<Sym, Rc<RefCell<Val>>>>>` (aliased as
`Globals`). `Env` keeps its frame chain (still used for `let`,
`letrec`, closure params) and gains a `globals: Weak<…>` back-edge
to the same table. `Env::lookup` walks frames first, then upgrades
the `Weak` and checks the table on miss. `Vm::with_world`
constructs `globals` first and threads `Rc::downgrade` into
`prim::initial_env(&globals)` so every Env in this Vm shares the
same `Weak`. `eval_str`'s pre-pass and `try_register_define` insert
placeholder cells into `globals` instead of extending `self.env`.
Rollback on error snapshots `globals.borrow().clone()` and restores
on `Err` (same semantics as the prior env-snapshot path; codex #3
fix is preserved).

The cycle is broken: globals → cell (strong) → `Val::Clo` (strong) →
`Env` (strong) → frames (strong, points up to prim base) and
`globals` (Weak, no cycle). Dropping the Vm drops the strong globals
ref; the table drops, every cell drops, every closure drops, every
closure's captured env drops — clean shutdown.

**Alternatives considered**:
- **`Weak` for the closure → env back-edge directly** (the issue
  filing's option 1). Minimal structural change, but the semantics
  question is real: a `Weak::upgrade` failure at lookup time means
  the closure outlived its env. For `letrec`-style local bindings
  that's a bug (the surrounding lexical scope is gone); for
  top-level defines it's "the Vm is gone." Conflating the two would
  make every lookup ambiguous about *which* it is. Globals split
  resolves this cleanly — the top-level case is the only one that
  uses Weak; lexical scopes stay strong.
- **Cycle collector.** Overkill for current scale, complicates the
  zero-deps story (ADR-002), and the cycles are all the same shape
  so an algorithmic fix beats a runtime one.
- **Stay on `Env`-extension and snapshot/replay defines per
  install.** Would only delay the leak by one Vm lifetime; the cycle
  is intrinsic to the storage shape, not to the install pattern.
- **Move prims into globals too** (so prim lookup is `O(1)` hash).
  Tempting — `BUILTINS` is ~40 entries, walked on every variable
  miss. Rejected for now: prims don't hold envs (no cycle through
  them), so they aren't part of the issue, and moving them would
  change shadowing semantics (`(define + 5)` would overwrite the
  builtin instead of lexically shadowing it). Worth revisiting as a
  pure perf change if benches show the prim chain dominates.
- **`letrec` cycles too.** Same shape (closure captures env
  containing its own cell). Punted per the issue's recommendation —
  the top-level case is the dominant source by 10×, and `letrec`'s
  semantic constraint ("the surrounding lexical scope is what holds
  this cell alive") makes the fix design-noisier. Filed as a
  follow-up if measurement warrants.

**Consequences**:
- **+** No `Rc` cycle through top-level defines. Confirmed by
  `dropping_vm_releases_top_level_closures` in `tests/eval.rs`:
  install spells, take a `Weak` to one of the prelude cells, drop
  the Vm, `Weak::upgrade()` returns `None`.
- **+** Mutual recursion across separate `eval_str` calls now
  works, as a side effect of the shared globals table — a closure
  resolves forward references at call time against the table's
  current contents, not a snapshot of its capture-time env. ADR-014
  documented this as a known limitation (with a pinned test); the
  limitation is gone, the test flipped to assert success.
- **+** `vm.globals` is exposed publicly (mirroring `vm.world`).
  Hosts can introspect defined names, reset state without dropping
  the Vm (`globals.borrow_mut().clear()` after re-running the
  prelude), or — eventually — implement `(forget 'foo)` cheaply.
- **−** Lookup now does an extra `Weak::upgrade` + HashMap probe
  per miss on the frame chain. Cheap (Weak::upgrade is a
  branch + Rc bump; HashMap probe is O(1)) but not free.
- **−** Rollback now clones the globals HashMap on every
  `eval_str` entry rather than Rc-bumping a single env handle.
  O(globals.len()) instead of O(1); cells inside are still
  Rc-shared so the values themselves don't copy. Negligible at
  current sizes; revisit if a host calls `eval_str` in a hot loop
  with a large globals table.
- **−** Slight API surface growth: `Globals` is a new public type
  alias, `Vm::globals` is a new public field. Both feel earned —
  they map a real concept (top-level binding table) instead of
  being incidental.
- **−** Prims still walk the env frame chain on lookup miss
  (because they live in the chain, not in globals). ~40 frames in
  practice. Not a regression — same as before — but the asymmetry
  ("prims in frames, defines in globals") will look odd until
  someone moves prims too. Documented in alternatives above.

**Deferred**:
- **Same cycle in `letrec`.** Closures captured during letrec init
  hold the env containing their own placeholder cell. Issue: the
  cell *needs* to outlive the closure (the closure's whole point
  is to reference its own name). Fix probably requires a `Weak`
  back-edge specifically for letrec-allocated cells, with a panic
  path if the cell is collected before the closure is called.
  Tracked in `core-followups.md`.
- **Move prims to globals.** ~~Would unify lookup (everything via
  hash, no frame walk for the common case) but changes
  `(define + 5)` semantics from shadowing to overwrite. Reasonable;
  needs a separate ADR for the semantics call.~~ **Done — ADR-020
  (2026-05-31).** Semantics is overwrite; discovery during the ADR
  was that today's behavior isn't lexical shadowing as assumed but
  silent dead-write (lookup walks the prim chain and never reaches
  the globals slot the define wrote into).

## ADR-016: Spell + gene packs in their own sibling crates (2026-05-29)

**Context**: `lisp::spells` (38 lines) and `lisp::genes` (458 lines)
were parked inside the engine crate when there was nowhere else for
them. Neither touches `World` / `world_prim`; both depend only on
`Vm`, `Val`, and `Arity`. Leaving them in `lisp` muddied the crate
boundary — the engine was "the language" plus "two specific DSL
packs," and every consumer of the language got the packs for free
whether wanted or not. ADR-010 set the "promote on second consumer"
rule (`crates/runes/`); ADR-011 invoked it for the codon table. Both
DSL packs already have three consumers each (CLI example + WASM
bridge + bench), so the same promotion was overdue. The audit on
2026-05-29 (top-three refactor sequence) made it explicit: this is
step 1 of getting the lisp crate back to "engine only."

**Decision**: Move `crates/lisp/src/spells.rs` and
`crates/lisp/src/genes.rs` verbatim into new `crates/spells/` and
`crates/genes/` sibling crates parallel to `crates/runes/` and
`crates/codons/`. Each declares `lisp = { path = "../lisp" }` and
nothing else. Public API is unchanged: same `install(vm)`,
`PRELUDE_DEFINES`, plus `seeded(seed, body)` / `render_creature(v)`
for genes. Internal `use crate::Vm` → `use lisp::Vm`. Consumers
(`examples/{spells,genes}.rs`, `crates/wasm`, `crates/bench`, the
lisp crate's own dev-dep tests) import `spells::install` /
`genes::install` instead of `lisp::spells::install` /
`lisp::genes::install`. The `lisp` crate's `[dev-dependencies]` gain
the two paths so its own examples and tests still compile (mirrors
the existing `runes`/`codons` dev-dep pattern).

**Alternatives considered**:
- **Keep them in `lisp` until a third pack appears.** Rejected: the
  "promote on second consumer" rule has already fired twice; there's
  nothing engine-specific about either module; the cost is zero.
- **Single `crates/dsl/` umbrella crate** holding both packs.
  Rejected: spells and genes share no code and have no reason to
  ship together. A host wanting only spells shouldn't compile the
  genome prelude (or its 459 LOC of resolver / renderer).
- **Re-export from `lisp` for back-compat** (`pub use spells as
  _spells`). Rejected: only ~7 callsites across the workspace. Fix
  once, drop the proxy.
- **Move spells but leave genes in `lisp`** (since genes is bigger /
  more "infrastructure-shaped"). Rejected: the genes prelude +
  resolver + renderer are exactly as demo-specific as the spell
  prelude. Asymmetric extraction would muddy the layering rule the
  refactor sets up.

**Consequences**:
- **+** `lisp` is "engine + parser + numeric tower" again. The
  crate's public surface stops growing with each new demo. After
  ADR-017 + ADR-018 land, the engine ships *zero* host-specific
  vocabulary.
- **+** Each pack documents its dep surface explicitly. Reading
  `crates/spells/Cargo.toml` shows exactly what the spell DSL needs
  from the language (just `lisp`); same for genes.
- **+** Sets up Step 2 / Step 3 (ADR-017 / ADR-018). Once the engine
  is host-agnostic and `world` lives in its own crate, the spell
  pack becomes the natural place to wire `world` + `spells` together
  via a thin `install_with_world(vm, world)` helper.
- **+** All 94 tests stay green by re-import only. No behavior
  change; this is a code-move ADR.
- **−** Two more workspace members (`crates/spells/`,
  `crates/genes/`). Stable build cost; negligible.
- **−** `lisp`'s `[dev-dependencies]` grows by two paths so its own
  examples + tests still compile. Acceptable — `runes` and `codons`
  already sit there for the same reason.

**Deferred**:
- ADR-017 (host-agnostic Vm) and ADR-018 (`crates/world/`
  extraction) are the next two steps of the same sequence. See the
  approved plan at `~/.claude/plans/nice-audit-can-you-elegant-duckling.md`.

## ADR-017: Host-agnostic Vm — closure-capable prims, no engine-owned `World` (2026-05-29)

**Context**: ADR-005 split prims into pure (`Val::Prim`,
`fn(&[Val]) -> R`) and host-state (`Val::WorldPrim`, `fn(&[Val],
&mut World) -> R`) so the engine could carry a `Vm.world:
Rc<RefCell<World>>` field and dispatch state-aware primitives without
losing testability. ADR-011 (the genes demo) made the limit visible:
genes wants zero host state, yet `Vm::new()` always carried a 0×0
`World::empty()` to satisfy the type. A roguelike / config DSL /
music sequencer would each want a different typed host state and
would have to ignore, force-fit, or fork the lisp crate (see
`docs/project_notes/host-state.md`). Two prim flavors, one fixed
host type baked in — the engine had de facto coupled to the spell
demo's needs. The 2026-05-29 audit (top-three refactor sequence)
made this Step 2.

**Decision**: Drop the engine's awareness of the host type entirely.
Three coupled changes:

1. **Collapse `Val::WorldPrim` into `Val::Prim`** as a single
   closure-capable variant. The field becomes
   `f: Rc<dyn Fn(&[Val]) -> Result<Val, String>>`. Same call shape as
   before; the closure may capture any host handle (or none). The
   `unsafe_code = "forbid"` workspace lint is preserved (`Rc<dyn Fn>`
   is safe). Zero new deps preserves ADR-002.

2. **Drop `Vm.world` and `Vm::with_world`.** The engine no longer
   knows `World` exists at all. `step`, `apply_k`, `apply`, `run`,
   `run_bounded` lose their `world: &Rc<RefCell<World>>` thread; the
   apply path dispatches on one prim variant that doesn't need it.

3. **`Vm::register_prim` becomes generic**: `F: Fn(&[Val]) ->
   Result<Val, String> + 'static`. Hosts that need state register a
   closure that captures their handle. `Vm::register_world_prim` is
   removed (had no external callers anyway).

`world.rs` + `world_prim.rs` stay inside the lisp crate for this
commit but `world_prim` gains a `pub fn install(vm: &mut Vm, world:
Rc<RefCell<World>>)` helper that wraps each of the 5 world prims in a
closure capturing `world.clone()`, then calls
`vm.register_prim(name, arity, |args| { f(args, &mut world.borrow_mut()) })`.
The spell-CLI example, the world-test file, and the WASM bridge all
go through this helper. ADR-018 (next commit) moves these files to
their own crate.

The `prim::initial_env` table wraps each builtin's fn-ptr in
`Rc::new` at Vm construction (~40 allocations per `Vm::new`), so the
single `Val::Prim` variant carries pure and state-capturing prims
uniformly. Per-call cost rises from "copy fn ptr" to "Rc::clone of a
thin handle" — a non-atomic refcount bump per prim lookup.

**Alternatives considered**:
- **Keep two variants — pure `Prim` (fn-ptr) plus new `Closure`
  (`Rc<dyn Fn>`)**. Rejected: once the engine has no privileged host
  type, both variants would carry identical signatures and identical
  dispatch logic in `apply`. The split would preserve a distinction
  nothing else respects.
- **`Box<dyn Fn>` instead of `Rc`**. Rejected: `Val` is already
  cheaply cloneable (everything else is `Rc` or `Copy`); a `Box`-typed
  closure would force `Val: Clone` for a prim into a non-trivial
  path, and any place a prim cell ends up Rc'd twice (the
  `eval_str` rollback snapshot bumps each globals cell) would not
  share storage.
- **Trait object via a `Primitive` trait** (`Box<dyn Primitive>`
  where `Primitive::call(&self, args: &[Val]) -> R`). Rejected: more
  ceremony for the same capability; the `Fn` closure path is what
  every Rust developer reaches for first.
- **Keep `Vm.world` as `Box<dyn Any>` for type-erased host state**.
  Rejected: pushes the type recovery into every host prim and makes
  the registration API uglier than capturing the handle in a closure.
- **Stay on fn-pointers; introduce a `HostHandle` thread-local** for
  prims to reach into. Rejected: hidden global state, breaks
  composition with multiple Vms or nested evaluation.

**Consequences**:
- **+** Engine is host-agnostic. The same `lisp` crate can host the
  spell DSL, the genes DSL, a roguelike, a config interpreter — all
  via `register_prim(name, arity, |args| { /* capture whatever */ })`.
  Three new tests in `tests/host_prim.rs` lock the promise:
  closure-prim mutates captured state; closure-prim reads + returns
  captured state; dropping the Vm releases captured cells.
- **+** `Val::WorldPrim` and its dispatch arm are gone. `step.rs`
  drops one match arm and three function-parameter threadings.
  ~20 LOC of engine simplification.
- **+** `Vm::new()` no longer auto-installs world prims. A host that
  wants them calls `lisp::world_prim::install(&mut vm,
  world.clone())` after constructing the Vm. The WASM bridge already
  does this; the world-CLI example and `tests/world.rs` updated.
- **+** Sets up ADR-018 (next commit): `world.rs` + `world_prim.rs`
  can move out of `lisp` because they no longer have privileged
  status. The lisp crate will stop shipping a tile grid.
- **+** Closures unlock per-host state shapes the old API couldn't
  express (multiple host handles, non-`World` typed state, host state
  that holds a `RefCell` of something the engine couldn't name).
- **−** `Val::Prim` lookup is now `Rc::clone` instead of fn-ptr
  copy. ~40 builtin allocations at `Vm::new` time; per-call cost is a
  non-atomic refcount bump. Bench delta (microsecond medians, `cargo
  bench -p bench --bench demos`):
    - `cast_spell_canonical`: 22.8 → 26.3 µs (+15%; many prim calls
      per cast, plus the captured `world.borrow_mut()` on every
      `world-apply!` adds overhead the old direct-`&mut World` path
      didn't have)
    - `cast_genome_balanced`: 73.0 → 75.8 µs (+4%)
    - `cast_genome_with_mut`: 81.3 → 83.3 µs (+2.5%)
    - `breed_diploid`: 260.7 → 249.2 µs (−4%, noise band)
- **−** `register_world_prim` removed — its only role was a
  fn-pointer with a `&mut World` arg, now covered by `register_prim(name,
  arity, move |args| { let mut w = world.borrow_mut(); f(args, &mut w) })`.
  No external callers; safe to delete.
- **−** WASM bridge takes a small structural change: `WasmVm` holds
  its own `world: Rc<RefCell<World>>` field rather than reaching
  through `vm.inner.world`. Same allocation pattern, different
  ownership location.
- **−** `Vm::new()`'s default semantics changed silently: today
  `(world-tile 0 0)` against a fresh `Vm::new()` returns "unbound
  variable: world-tile" instead of "world has zero dimensions." Safe
  in-repo (all world-touching tests use `world_prim::install`
  explicitly); flagged here for any downstream consumer.

**Deferred**:
- ADR-018 (`crates/world/` extraction) is the next commit in the
  same sequence — `world.rs` + `world_prim.rs` move out of the lisp
  crate now that nothing engine-side references them.

## ADR-018: `world` extracted to its own sibling crate (2026-05-29)

**Context**: ADR-017 dropped the engine's awareness of `World` but
the type still lived inside `crates/lisp/src/`. With `Val::WorldPrim`
gone and host wiring done via closure-capable `register_prim`, there's
no engine-side reason for the lisp crate to carry a tile grid + event
log + 5 host primitives. `docs/project_notes/host-state.md` called the
split out explicitly: bullet 2 of "What the endgame could look like"
was "add a `crates/world/` micro-crate as a reusable building block,
sibling to `runes/` and `codons/`." Step 3 of the 2026-05-29 audit
refactor sequence does that.

**Decision**: Move `world.rs` and `world_prim.rs` from
`crates/lisp/src/` into a new `crates/world/` sibling crate. The new
crate depends only on `lisp` (for `Vm`, `Val`, `Arity`) and exposes:
- `pub struct World`, `pub enum Tile` — verbatim port.
- `pub mod world_prim` — the 5 prims plus the
  `install(vm: &mut Vm, world: Rc<RefCell<World>>)` helper introduced
  in ADR-017 as the public wiring entry point.

`crates/spells/` adds an `install_with_world(vm, world)` one-liner
that calls `spells::install(vm)` plus
`world::world_prim::install(vm, world)` — both consumers
(`examples/spells.rs`, the WASM bridge, the world-touching tests in
`tests/world.rs`, and the spell bench) want exactly that pair, so the
helper saves the duplication. The lisp crate stops re-exporting
`Tile` / `World`; consumers import from `world` directly.

**Alternatives considered**:
- **Leave `world.rs` in `lisp` as an opt-in module with a feature
  flag**. Rejected: cargo features for a "what crate is the file in"
  problem; the project's promotion mechanism is sibling crates (ADR-010,
  ADR-011, ADR-016).
- **Fold `world` into `crates/spells/` instead of a separate crate**.
  Rejected: `World` is reusable host state (a roguelike, a Conway
  demo, anything grid-shaped) that has nothing to do with the spell
  vocabulary. Coupling it to spells would force the next host to either
  depend on spells or duplicate the file.
- **Tie the extraction to a generic `Grid<T>` rev** before moving
  (`host-state.md` mentions this as the "complementary, not competing"
  follow-up). Rejected: that's a separate design decision (what does
  the trait surface look like?) and a separate refactor. ADR-018 only
  re-locates today's concrete `World`; a future ADR can rev to
  `Grid<T>` once a second grid-shaped host appears.
- **No `install_with_world` helper on `crates/spells/`**. Leave each
  consumer to call `spells::install` + `world::world_prim::install`
  themselves. Rejected: every consumer wanted exactly the same pair;
  one helper is cleaner than two-line duplication scattered across
  three call sites. Accepting that `spells` depends on `world` as a
  result is a deliberate trade — see Consequences.

**Consequences**:
- **+** `lisp` ships zero host types. Reading the lisp crate root, a
  new contributor sees "engine + parser + numeric tower" with no
  demo-shaped vocabulary or host types leaking in. Closes the
  audit's #1 layering smell.
- **+** Symmetry restored across the demo-adjacent crates: `runes`
  and `codons` are translation tables; `spells` and `genes` are
  vocabulary packs; `world` is the host-state building block. Each
  is independent and opt-in.
- **+** Closes both bullets of `host-state.md` §What the endgame
  could look like: ADR-017 made the engine host-agnostic; ADR-018
  ships the grid as an opt-in building block.
- **+** All 97 tests pass; clippy clean. No behavior change — this
  step is mechanical relocation on top of ADR-017's substantive
  refactor.
- **−** `crates/spells/` now depends on `world` (for the
  `install_with_world` helper) — a deliberate coupling of one
  vocabulary pack to one host-state shape. Consumers wanting just the
  spell prelude with a non-`World` host call `spells::install(vm)`
  alone; the helper is opt-in.
- **−** Three new path-dependency edits across the workspace (`lisp`
  dev-deps, `wasm` deps, `bench` deps, `spells` deps). Stable build
  cost; negligible.

**Deferred**:
- **Generalize `World` to `Grid<T> + EventLog`** as
  `host-state.md` proposed. Out of scope until a second grid-shaped
  host appears (the project's "promote on second consumer" rule).
- **`docs/let-rs.html` narrative** mentions `world.rs` and
  `Val::WorldPrim`. Will need a refresh; tracked separately so this
  refactor sequence stays scoped.

## ADR-019: Curves demo — L-systems via symbol tape + turtle host state (2026-05-29)

**Context**: Runes/spells and codons/genes followed the same shape —
a tiny tape alphabet in its own zero-dep crate, paired with a
vocabulary pack that installs prims + a prelude on top of `lisp`. The
project memory already calls this the "rule of three": two siblings
exist; a third would either confirm the pattern or expose where it
bends. ADR-018 closed the host-state refactor by extracting `world`,
which left the demo-adjacent crates fully orthogonal — a clean
moment to add a third sibling pair before anything else moves.

L-systems (Lindenmayer, 1968) fit the existing shape almost
suspiciously well: a small alphabet of turtle glyphs (`F + - [ ]`),
production rules that rewrite the tape in place, and a visual ASCII
payoff (curves, fractals, branching plants). They also flex something
neither earlier demo does: the rewrite step grows the *tape itself*
before it's interpreted, which exercises pure-lisp recursion in a way
spell pipelines and genome resolvers don't.

**Decision**: Add two sibling crates following the established
split:

- `crates/strokes/` — turtle-glyph tape alphabet. Six glyphs:
  `F` (forward draw), `G` (forward no-draw), `+` (turn left 45°),
  `-` (turn right 45°), `[` (push state), `]` (pop state). Each
  glyph emits a *quoted symbol* into the output list, so
  `tape_to_sexpr("F+F")` → `"(list 'F '+ 'F)"`. Zero-dep, sole
  source of truth for the glyph table (parallel to `runes` and
  `codons`).
- `crates/curves/` — L-system DSL pack. Owns the turtle state
  (`Turtle`, an `Rc<RefCell<Turtle>>` captured by prims at install
  time, mirroring ADR-017's `World` pattern), the side-effecting
  turtle prims (`draw!`, `render!`, `reset!`), and a small prelude
  with the pure-lisp rewrite engine (`expand`, `grow`). Depends only
  on `lisp`.

Tape representation as a list of *symbols* (not function calls) is
the key shape decision — it's what makes pure-lisp rewrite natural.
`grow` walks a symbol list, looks each symbol up in a rules alist
(`((F . (F + F)) …)`), and splices the replacement in via `append`.
A final `draw!` host prim dispatches each symbol to the matching
turtle action.

8-direction turtle (45° per `±`). Heading is a `u8` in `0..8`;
`forward!` stamps a heading-dependent glyph (`─ ╱ │ ╲`) into a
sparse `HashMap<(i32, i32), char>` so the canvas auto-sizes from
the actual bbox of visited cells at render time.

**Alternatives considered**:
- **4-direction turtle (90° per `±`)**. Simpler — every Hilbert /
  dragon / Sierpiński example from the L-system literature works
  unchanged. Rejected for v1: 4-dir ASCII is all `─` and `│`, which
  looks like Pac-Man. 8-dir loses some canonical examples (Hilbert
  needs `++`/`--` instead of `+`/`-`) but the diagonal glyphs are
  visibly richer, which is the whole point of an ASCII demo. A
  later ADR can revisit if a 4-dir-only example becomes important.
- **Tape as a list of function calls** (matching spells / genes).
  E.g. `(list (forward) (turn-left) (forward))`. Rejected: the
  L-system rewrite step needs to splice symbol sequences in
  arbitrary order, which is trivial on a symbol list but awkward
  on a list of resolved function values (you'd compare closures, or
  re-introduce symbolic indirection). The symbol-list shape is the
  natural form for L-system production rules; making the curves
  pack the odd one out is the right local choice.
- **6-dir hex turtle** for Koch / Sierpiński triangle. Rejected:
  ASCII renders hex grids poorly; we'd need wider unicode
  half-block trickery, and the engine doesn't have float math for
  the cell-mapping anyway. Out of scope.
- **Engine-side `begin` for sequencing**. The cleanest
  user-facing API would be `(begin (reset!) (draw! …) (render!))`,
  but our lisp doesn't have `begin` and adding it is an engine
  change. Rejected: stick with the existing `let` chain idiom
  (each prim returns a value, sequencing via nested `let`s) or
  do the three calls as separate top-level forms in the example /
  REPL. Adding `begin` is a separate decision; this ADR shouldn't
  drag the engine.
- **Bake an `install_with_turtle(vm)` that owns the turtle
  internally** (no host-supplied `Rc<RefCell<Turtle>>`). Rejected:
  symmetric with `world::world_prim::install(vm, world)` is more
  valuable than the one-line save; consumers that want to peek at
  turtle state from Rust (the WASM bridge if it ever gets a Curve
  Lab page) need a handle.

**Consequences**:
- **+** Confirms the rule-of-three: the
  `<alphabet-crate> + <vocabulary-crate>` split survives a third
  pass without bending. Pattern is now established, not just
  observed twice.
- **+** First demo whose tape is rewritten before being interpreted
  — exercises pure-lisp `letrec`/`cons`/`append` recursion in a
  visible way (the test suite gains "grow N iterations produces
  expected sequence" cases that double as recursion smoke tests).
- **+** First demo with side-effecting prims that don't thread
  ctx — turtle ops mutate `Rc<RefCell<Turtle>>` directly. Validates
  that ADR-017's prim shape handles "imperative" hosts as cleanly
  as the ctx-folding spell pipeline.
- **+** Canvas auto-sizing means no `(canvas! w h)` ceremony.
  Single `(render!)` call → string keyed off whatever the turtle
  actually visited.
- **−** 8-dir means the canonical Hilbert / dragon curves render
  oddly without `++`/`--` doubling. Documented in the example with
  curves chosen for 45° fit (Lévy C, fractal plant); a contributor
  who reaches for Hilbert will need to know.
- **−** Tape-as-symbol-list breaks symmetry with the other two DSL
  packs (spells/genes both produce function-value lists). The
  divergence is justified by the rewrite step but is worth flagging
  for future DSL designs — not every domain wants the same shape.
- **−** `+` and `-` are also arithmetic primitives in the engine.
  Quoting (`'+`) avoids the collision at the tape level; no engine
  change needed. Worth noting in the example's intro so a reader
  doesn't think we shadow them.

**Deferred**:
- **WASM bridge page for the Curve Lab.** The natural shape is a
  per-iteration slider — drag from 1→5 and watch the curve
  unfold. Out of scope for v1 (CLI demo lands first; promote to
  WASM when the existing two pages need a sibling).
- **`docs/let-rs.html` narrative refresh** to add curves alongside
  spells/genes (the ADR-018 deferral covered the host-state edits;
  this is an additive pass).
- ~~**`begin` as an engine special form.** Tracked as a follow-up if
  any later DSL pack also wants imperative sequencing; not worth a
  one-off engine change for this demo.~~ **DONE 2026-06-05 as a
  macro, not an engine form.** Shipped in `macros::install_stdlib`
  (ADR-024 made macros a sibling crate, so adding stdlib macros
  doesn't grow the engine). The WASM bridge's `cast_curve` now uses
  `(begin (reset!) (draw! …) (render!))` directly. The original
  "engine special form" rejection still stands — the macro path
  honored the ADR-019 reasoning.
- **Generalize the turtle to a configurable angle / N-direction
  table.** Out of scope until a second turtle-shaped host appears,
  same "promote on second consumer" rule.

## ADR-020: Prims live in globals; `(define +)` overwrites (2026-05-31)

**Context**: ADR-015 split top-level `define` bindings off the env
frame chain into a `Vm`-owned globals table to break an `Rc` cycle,
and explicitly punted on moving built-in prims to the same table.
The result is an asymmetry: `prim::initial_env` still installs ~40
prims as `env.extend` frames at `Vm::new` time, while defines write
to `globals`. `Env::lookup` walks frames first, then falls through
to globals on miss (`env.rs:94`).

Two visible costs:

1. **Lookup walks ~40 prim frames before reaching globals on every
   miss.** Negligible in practice; cosmetically odd.
2. **`(define + 5)` is silently inert today.** The pre-pass
   allocates `globals['+'] = cell`, the body writes `5` into it,
   and then `(+ 1 2)` looks up `+`, walks the prim frame chain,
   finds the built-in `+`, and returns `3`. The new globals binding
   is never reached. No error, no warning — just a dead write.
   Worse than either shadowing or overwriting.

The 2026-05-29 architecture audit (item #3) called the asymmetry
out and noted the blocker: nobody had picked the semantics for
`(define + 5)` once the prim chain goes away. This ADR resolves
that.

**Decision**: Move `BUILTINS` registration from
`env.extend(...)` at `prim::initial_env` time to
`globals.insert(...)` at `Vm::new` time. Drop the prim frame chain
entirely — `prim::initial_env(&globals)` becomes "seed the globals
table with the built-ins, then return an empty Env that points to
it." Lookup is now: walk lexical frames (`let` / `letrec` / closure
params) → fall through to globals (where prims and user defines
both live). One home for top-level names.

For the semantics question — `(define + 5)`:

> **Overwrite.** `(define name body)` unconditionally writes the
> body's value into `globals[name]`, regardless of whether `name`
> already holds a prim, a previous user define, or nothing.

That means after `(define + 5)`, subsequent `(+ 1 2)` fails at
apply time with "5 is not callable" (or whatever the engine's
"applied non-procedure" path says). The new binding is reachable;
the prim is gone for this Vm's lifetime.

**Alternatives considered**:
- **Reject redefinition of names that came in via `BUILTINS`.**
  Defensive — `(define + 5)` would error at register time with
  "cannot redefine built-in `+`". Pros: preserves prelude
  invariants; surfaces collisions loudly at the point of writing.
  Cons: (a) requires marking globals entries as built-in vs
  user-defined, so the entries are no longer just `Rc<RefCell<Val>>`
  — there's metadata. (b) Constrains legitimate use: a user can't
  write `(define + my-generic-plus)` to extend arithmetic, which is
  a real Scheme idiom. (c) The "what counts as built-in" line is
  fuzzy once preludes (`spells::install`, `genes::install`,
  `curves::install`) start installing their own defines that look
  exactly like prims to a downstream reader. Rejected.
- **Lexical shadow only — `define` always errors on a top-level
  collision; `(let ((+ 5)) …)` is the only way to shadow.** Strict
  and predictable, but breaks the "preludes install via top-level
  `define`" pattern (ADR-014): a prelude couldn't define a name
  that any other pack also defined. The DSL packs already collide
  in practice (spells and genes both defined `start` /
  `stop` until ADR-019's namespace fix). Rejected — too rigid for
  the pattern.
- **Overwrite, but mark the new entry as "user-defined" so tooling
  can warn.** A `(value: Val, source: BuiltinOr<User>)` shape on
  globals entries. Pros: enables a host to highlight "you just
  shadowed a prim" in a REPL. Cons: adds a field to every globals
  entry to support a feature no caller has asked for. Filed under
  "do it when a host wants it"; not part of this ADR.
- **Keep prims in env frames, add an explicit error in the define
  pre-pass when `name` is a known built-in.** Cheaper than the
  move (no Env shape change). Cons: doesn't fix the lookup walk;
  doesn't unify the two homes for top-level names; treats a
  cosmetic asymmetry by adding a guard rather than removing it.
  Rejected as a half-measure.

**Consequences**:
- **+** Uniform top-level lookup: one home for prims, defines, and
  prelude-installed bindings. `Env::lookup`'s frame walk is now
  meaningful (it's only lexical scopes), and the fall-through is
  the only path to a top-level name.
- **+** `(define + 5)` is no longer silently inert. After the
  define, `(+ 1 2)` errors at apply time with a clear "non-procedure
  applied" message. The footgun moved from "silently dead" to
  "loud at the next call."
- **+** Lookup is faster on the common miss: no ~40-frame prim
  chain walk before the globals hit. Microbench territory; not
  worth a perf claim, but it's not worse.
- **+** `Vm::new` no longer threads `prim::initial_env(globals)` →
  `self.env`. `self.env` can be `Env::with_globals(&globals)` plain
  — a single line, no fold over `BUILTINS`. The `prim` module
  becomes "the BUILTINS table plus their implementations"; the
  registration mechanism moves to `lib.rs` (or stays in `prim.rs`
  but writes to globals instead of returning an Env). Smaller
  surface area for the registration story.
- **+** Cycle is still broken. Prims don't capture env, so moving
  them into globals (where they're held by strong `Rc`) doesn't
  re-introduce the ADR-015 cycle. Closures still see globals via
  `Weak`; prims see globals via strong `Rc` only because the Vm
  itself owns the map.
- **−** Possessing a working `+` after `(define + 5)` requires
  resetting the Vm (drop and recreate) or implementing
  `(forget 'name)` to delete a globals entry. ADR-015 noted that
  globals are publicly exposed, so `vm.globals.borrow_mut().remove("+")`
  works from the host side today, but there's no in-language affordance.
  Filed as a follow-up if a REPL wants it.
- **−** `register_prim`'s public API (Vm-level prim registration
  for hosts wiring `world-set-tile!` etc.) needs to also write to
  globals instead of `env.extend`. One-line change; documented in
  implementation.
- **−** ADR-014's "preludes are just top-level defines" property
  now applies to overwriting prims by accident — a prelude with
  `(define start …)` will shadow a prim called `start` if one
  exists. Today the spell/gene/curve packs are namespaced and don't
  collide with prims, but the failure mode shifts from "silently
  dead" to "active overwrite," which a careless pack author would
  surface in a runtime error instead of as a confused dead binding.
  Net better, but worth a one-line note in the DSL-pack contract:
  "your `define`s land in the same table as the built-ins."

**Implementation sketch** (for the follow-up commit, not this ADR):

```rust
// crates/lisp/src/prim.rs
pub fn install_builtins(globals: &Globals) {
    let mut g = globals.borrow_mut();
    for &(name, arity, f) in BUILTINS {
        let val = Val::Prim { name, arity, f: Rc::new(f) };
        g.insert(name.into(), Rc::new(RefCell::new(val)));
    }
}

// crates/lisp/src/lib.rs — Vm::new
let globals = Rc::new(RefCell::new(HashMap::new()));
prim::install_builtins(&globals);
let env = Env::with_globals(&globals);
```

Tests to add in `tests/eval.rs`:
1. `define_over_prim_overwrites` — `(define + 5) +` returns `5`.
2. `define_over_prim_then_call_errors` — `(define + 5) (+ 1 2)`
   errors with "non-procedure applied" (or whatever the canonical
   apply-error string is).
3. `prim_still_callable_in_lexical_scope` — `(let ((+ 100)) (+ 1 2))`
   returns `100` via lexical shadowing (frame walk wins for `let`
   bindings). This was true before and stays true; pin it.

**Deferred**:
- `(forget 'name)` engine prim to remove a globals entry. Trivial
  implementation (`globals.borrow_mut().remove(name)`); waiting on
  a host that wants it.
- Marking globals entries as "user" vs "built-in" for tooling
  (REPL highlighting on collision). Not part of this ADR.
- The `letrec` Rc cycle (ADR-015 punt). Independent of this move;
  filed in `core-followups.md`.

## ADR-021: letrec Rc cycle — pinned, deferred (2026-05-31)

**Context**: ADR-015 broke the top-level `define` Rc cycle by
making `Env::globals` a `Weak` back-edge to the Vm-owned globals
table. The same cycle exists in `letrec`, but the fix doesn't
transfer. The 2026-05-29 audit listed it as item #4 (lower priority
than the host-coupling and demo-crate moves); we filed it in
`core-followups.md` after the audit triage; this ADR records the
diagnosis and the conclusion that no clean fix exists without a
substantially more invasive engine change.

**The cycle.** Tracing `(letrec ((f (lambda () (f)))) f)` through
`step.rs:93`:

1. `env_rec = env.extend_placeholder("f")` allocates a `cell:
   Rc<RefCell<Val>>` initialized to `Val::Bool(false)`, wraps it in
   a `Frame { slot: cell, ... }`, hangs that off `env_rec.frame`.
   `K::Letrec.cells[0]` also holds the cell strong.
2. The lambda init evaluates in `env_rec`. The closure captures
   `env_rec` by clone (`step.rs:50`); `Val::Clo { env: env_rec, …
   }`. `closure.env.frame` is an `Rc::clone(env_rec.frame)`.
3. `K::Letrec` patches `*cell.borrow_mut() = closure`. The cell now
   contains the closure; the closure's env contains the frame; the
   frame contains the cell. Cycle closed.

After body eval finishes and `K::Letrec` drops, the strong refs are:

- `frame` strong: 1 (held by `closure.env.frame`)
- `cell` strong: 1 (held by `frame.slot`)
- `closure` strong: 1 (lives by-value inside `cell`'s RefCell)

Each cycle node has exactly one strong incoming reference from the
next. Nothing reaches zero. Leak.

**Why ADR-015's pattern doesn't transfer.** ADR-015 worked because
globals have an unambiguous owner whose lifetime is strictly longer
than every closure's: the Vm. Closures borrow globals via `Weak`;
when the Vm drops, globals drop, and every closure stored there
becomes unreachable and drops too. For letrec, the cells *must*
live as long as any closure that closes over them — Scheme
semantics require `(letrec ((f (lambda () (f)))) f)` to return a
closure that, when called, still finds `f`. So `closure → cell`
can't be `Weak` without breaking valid recursion.

**Alternatives considered**:

1. **Closure-converted letrec captures.** At lambda compile time,
   identify free vars resolving to letrec-allocated cells; carry
   them as a `Vec<(Sym, Rc<RefCell<Val>>)>` on `Val::Clo` instead
   of through the captured env; make the letrec frame slots `Weak`.
   Pros: closures stop capturing entire letrec env chains
   (memory-cost win even if cycles remain). Cons: still a
   `cell ↔ closure` self-cycle for any closure that references its
   own name (`closure.letrec_captures[0]` strong-holds cell, cell
   strong-holds Val::Clo, which is the same closure shape). Reduces
   the cycle from three nodes to two; doesn't eliminate it.
2. **A separate `LetrecScope` struct held strong by returned
   closures.** Closures hold `Rc<LetrecScope>`; scope holds cells;
   frames hold `Weak`. Pros: env can drop cleanly. Cons: same two-
   node `cell ↔ scope` cycle via `cell.value = Val::Clo {
   letrec_scope: Rc<scope> }`, `scope.cells[0] = Rc<cell>`.
   Equivalent leak shape, more code.
3. **Y-combinator desugaring at compile time.** Rewrite
   `(letrec ((f init)) body)` into application of a fixed-point
   operator so the lambda body doesn't reference `f` by name at
   all. Pros: zero cycles — the closure is freshly materialized on
   each call. Cons: substantial compile-pass work, semantic edge
   cases for mutually-recursive bindings, fresh-closure-per-call
   has a perf cost. Plausible but invasive.
4. **Cycle collector.** Hand-rolled mark-and-sweep over `Val::Clo`
   reachability. Breaks ADR-002's zero-deps stance unless a
   one-off implementation is written. Heavy.
5. **Weak self-ref with cell-stored-as-Weak.** Make the cell hold
   `Weak<Val::Clo>` for self-referencing letrec bindings, with the
   strong ref living in whatever externally holds the closure.
   Cons: breaks valid Scheme — `(letrec ((f (lambda () (f)))) f)`
   returns a closure whose internal `f` lookup fails because the
   external holder is the result of letrec, not the cell.

None of (1)-(5) ship today's bang for the buck. (1) and (2) are
half-fixes; (3) is a real fix but a meaty refactor; (4) breaks
ADR-002 or eats months of hand-rolled GC code; (5) breaks
semantics.

**Decision**: Pin the cycle's shape with a diagnostic test, accept
the leak, defer the fix until either a host actually observes
material growth or the engine is ready for the Y-style desugaring
refactor (probably alongside an ADR-NNN CESK upgrade, which is
where store-reified bindings would already be in scope).

The diagnostic test (`letrec_cycle_persists_after_drop` in
`tests/eval.rs`) asserts that, today, a `Weak` handle to a letrec-
allocated cell *still upgrades* after the closure has been dropped
from the user's scope. That's the inverse of
`dropping_vm_releases_top_level_closures` — it pins the leak so a
silent fix in the future would flip it loudly.

**Consequences**:
- **+** Cycle is documented in code (the test) and in this ADR.
  Future engine work that fixes it has a regression target.
- **+** No engine change ships today. Zero risk to existing
  semantics; no behavior shift.
- **−** Per letrec form with a recursive closure, the leak is:
  one `Frame` + one `Rc<RefCell<Val>>` cell + one `Val::Clo`
  (body `Rc<Expr>` + captured `Env`). Ballpark ~200 bytes plus
  the closure body's compiled-expr Rc graph. At REPL scale
  (one-shot evaluations) negligible; in a loop that creates
  letrec closures repeatedly it grows linearly. The web REPL is
  bounded by the step budget so a single eval can't loop letrec
  unboundedly, but successive REPL submissions can accumulate.
- **−** Hosts running long sessions with heavy `letrec` use will
  see slow heap growth. Workarounds: drop and recreate the Vm
  periodically; prefer top-level `define` over `letrec` for
  recursive procs (ADR-015 already broke that cycle).
- **−** The audit's clean-up debt remains visible. Subsequent
  audits will flag it; this ADR is the canonical "we know, we
  measured, we chose to wait" answer.

**Implementation sketch** (for the diagnostic test, not the fix):

```rust
#[test]
fn letrec_cycle_persists_after_drop() {
    // ADR-021: documents the residual letrec Rc cycle. A letrec
    // closure that references its own name forms cell → Val::Clo
    // → env.frame → cell. After the user's strong handle drops,
    // the cycle keeps every node alive (leak). When the engine
    // grows a real fix, this assertion flips — and that flip is
    // the signal we want.
    use std::rc::Rc;
    let mut vm = lisp::Vm::new();
    let v = vm
        .eval_str("(letrec ((f (lambda () (f)))) f)")
        .unwrap();
    // Walk into the returned closure to get a Weak handle on its
    // captured env's letrec cell …
    // (helper digs through Val::Clo → env.frame.slot for "f")
    let weak = letrec_cell_weak(&v).expect("cell handle");
    drop(v);
    drop(vm);
    assert!(
        weak.upgrade().is_some(),
        "today: letrec cycle keeps the cell alive past every \
         strong handle; when this flips to is_none(), a real fix \
         landed"
    );
}
```

The `letrec_cell_weak` helper needs to reach into `Val::Clo`'s env
and find the named slot. That's the only added surface; everything
else is the test body.

**Deferred**:
- ~~The fix itself. The two leading candidates are option 1
  (closure conversion, half-fix) and option 3 (Y-style desugaring,
  full fix). Pick when a host needs it. The CESK upgrade (separate
  ADR, also deferred) would refactor Env's storage anyway, so the
  letrec fix probably lands alongside or after CESK.~~ **DONE
  2026-06-02 via ADR-023 (CESK store).** Frame slots are now
  `Addr` indices into a Vm-owned `Store`; closures hold a
  `Weak<Store>` via env, so the cycle dissolves by construction.
  The diagnostic test renamed `letrec_does_not_leak` and now
  asserts the store drops with the Vm.
- `Vm::heap_summary()` or similar host-visible diagnostic for
  measuring growth. Not part of this ADR; would be filed if a
  consumer asked for it. (Post-CESK: `Vm::store.len()` is the
  one-liner here; full host-visible diagnostic is still a future
  ADR if needed.)

## ADR-022: Structured parse errors with source spans (2026-05-31)

**Context**: Every error in `lisp` is a flat `Result<_, String>`.
Tokenizer, reader, compiler, macro-expander, CEK step, and built-in
prims all return bare strings. The web REPL surfaces those strings
raw. A user typing a multi-line define with a missing close-paren
sees `unexpected eof` with no idea which paren or line.

This has been the top item in `web/let-rs.html`'s "What comes
after" coda since day one, and shows up as a TLC item in the
2026-05-29 architecture audit. The fix is straightforward but
unavoidably wide — it touches the public API of `eval_str` and
most internal error sites — so it deserves an ADR rather than a
silent migration.

**Decision**: Introduce a structured `LispErr` carrying an optional
source `Span`. Ship in two phases; this ADR scopes Phase 1
explicitly and acknowledges Phase 2 as a separate later move.

```rust
// crates/lisp/src/error.rs (new module)
pub struct LispErr {
    pub msg: String,
    pub span: Option<Span>,
}

pub struct Span {
    pub line: u32,  // 1-indexed
    pub col:  u32,  // 1-indexed (byte column for ASCII, char column otherwise)
    pub len:  u32,  // span length in source bytes, for highlight rendering
}

impl Display for LispErr {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match &self.span {
            Some(s) => write!(f, "{}:{}: {}", s.line, s.col, self.msg),
            None    => write!(f, "{}", self.msg),
        }
    }
}

impl From<String> for LispErr { /* span: None */ }
impl From<&str>   for LispErr { /* span: None */ }
```

**Phase 1 (this ADR)**: parse-time errors get spans.

- `Tok` becomes `Spanned<TokKind>` (or `Tok` gains a `span: Span`
  field — implementation detail). `tokenize` attaches a span to
  every emitted token.
- `read_datum` errors carry the offending token's span. EOF errors
  carry a synthesized span at the end of source.
- `Datum` gains an optional `span: Option<Span>` field. Parse-
  produced datums have `Some`; macro-synthesized datums have
  `None`. Macros that want spanned output can borrow the call-site
  span when synthesizing.
- `compile` errors propagate the source datum's span.
- Public API: `eval_str(&mut self, src: &str) -> Result<Val, LispErr>`.
  `From<String>` lets every internal `String` error propagate
  unchanged via `?`; the span is `None` and the host's `Display`
  rendering matches today.
- Internal call sites that emit positioned errors do so explicitly
  by building a `LispErr` with span attached (e.g., the
  unmatched-paren site in `tokenize`).

**Phase 2 (separate ADR, deferred)**: runtime errors get spans.
Plumbing `Span` through `Expr` so step-time errors carry the
source location of the failing form. The change is mechanical but
touches every `Expr` variant — bigger than Phase 1. Filed as a
follow-up; this ADR doesn't ship it.

**Alternatives considered**:

1. **No span; structured `kind` enum only.** Solves nothing for
   the actual UX gap. Rejected.
2. **Full Expr-level spans in one go (Phase 1 + 2 together).**
   Bigger change; touches every step.rs site that constructs or
   matches `Expr`. Deferring Phase 2 lets us ship the parse
   wins quickly and revisit runtime spans when we know which
   site categories matter for UX.
3. **Byte-offset spans (single `start: u32` instead of `line:col`).**
   More compact internally but every host wants `line:col` for
   display. Converting at error-construction time is fine and
   keeps the public API human-readable.
4. **`thiserror` / `anyhow` for the error type.** Violates ADR-002
   (zero deps). `LispErr` is small enough to hand-roll.
5. **A separate `parse-error` crate.** Too small to justify; the
   error type is intrinsically shared between parse and eval.
6. **Inline-error-position via panicking with a string.** Already
   exists implicitly; not a structured solution.

**Consequences**:
- **+** Web REPL errors gain `line:col` immediately. The Spell
  Lab and Gene Lab errors (currently raw strings in `<pre
  class="log">`) become clickable / highlightable.
- **+** Internal code keeps emitting `String` via `?`. `From<String>`
  is the bridge — no rewrite of `step.rs` / `prim.rs` needed for
  Phase 1.
- **+** Hosts that want richer rendering (IDE-style underline) get
  `len`. The WASM bridge can surface `Span` to JS as a struct,
  not just a string.
- **+** Phase 2 is incremental: add `span: Option<Span>` to `Expr`
  variants, attach during compile, look up at step-time. No
  re-architecture.
- **−** Public API change: `eval_str -> Result<Val, LispErr>`
  instead of `Result<Val, String>`. Hosts that match on the error
  string need `.to_string()` interposed, or to read `e.msg`. The
  WASM bridge needs a one-line update
  (`.map_err(|e| JsValue::from_str(&e.to_string()))?`).
- **−** Internal `Result<_, String>` signatures fan out to
  `Result<_, LispErr>` for every function on the parse / expand /
  compile / eval path. ~30 signatures. The bodies stay the same
  thanks to `From<String>`; the churn is mostly type signatures
  and `?`-bridging.
- **−** Tests that call `.unwrap_err()` and `assert!(err.contains("…"))`
  need `.to_string()` interposed or `err.msg.contains(…)`. ~15-20
  tests in `tests/eval.rs` and `tests/express.rs`.
- **−** `Datum` gains an `Option<Span>` field (8 bytes on 64-bit
  with `Option<u32>` triple — actually 16 bytes for `Span { u32,
  u32, u32 }` plus discriminant, padded). Memory cost is a few
  hundred bytes per parsed top-level form. Negligible at REPL
  scale.

**Implementation order**:

1. Add `crates/lisp/src/error.rs` with `LispErr` + `Span` +
   `Display` + `From<String>`/`From<&str>`. `pub use` from
   `lib.rs`.
2. `tokenize` returns `Result<Vec<Tok>, LispErr>`; `Tok` carries a
   `span` field. Single positioned error site: the catch-all that
   today returns a plain "unexpected char" string.
3. `read_datum` returns `Result<Datum, LispErr>`; `Datum` gains
   `span: Option<Span>`. The EOF error gets a synthesized end-of-
   source span.
4. `compile` and friends switch to `Result<_, LispErr>`. Existing
   `String` errors propagate via `From`. Compile-time errors that
   want spans pull from the input `Datum::span`.
5. `eval_str` public signature flips. `step.rs` / `prim.rs` stay
   on `Result<_, String>` for Phase 1 and get wrapped at the
   `eval_str_inner` boundary (`.map_err(LispErr::from)?`).
6. WASM bridge: one-line `.to_string()` change.
7. Tests: ~15 string-match assertions get `.msg.contains(…)` or
   `.to_string().contains(…)`.
8. Add new tests pinning positioned errors:
   - `unmatched_open_paren_at_position` — `"(+ 1\n  2"` errors at
     line 1 col 1.
   - `unknown_symbol_carries_span` — `"(\n  foo)"` errors at line 2
     col 3 with msg referencing `foo`.
   - `runtime_error_has_no_span_yet` — `"(define + 5) (+ 1 2)"`
     errors with `not callable: 5` and `span: None` (this pins
     Phase 1's boundary and flips to `Some` when Phase 2 lands).

**Deferred**:
- **Phase 2**: runtime errors carry spans by plumbing `Span` through
  `Expr`. Separate ADR when shipped.
- **Multi-line error rendering / underlines**: a `LispErr::render(src)`
  helper that returns a multi-line string with a caret pointing
  at the span. Not engine-level; could live host-side or in a
  thin utility crate when a host wants it.
- **Macro-expansion span tracking** (the call-site → expanded
  forms mapping). For Phase 1, macro-expanded datums have
  `span: None`. A future ADR could attach the call-site span to
  every datum the macro emits, making expanded-code errors point
  back to the user's source rather than into macro-generated
  forms.

## ADR-023: CESK migration — designed, deferred (2026-06-01)

**Context**: The engine today is CEK. The three explicit registers
are control, environment, continuation (`State { mode, k }` at
`step.rs:13-16`; `Env { frame, globals }` at `env.rs:27-30`), with
the closure value carrying its own captured env as a fourth
implicit register. Each frame slot is an `Rc<RefCell<Val>>` so
`letrec` can hand a placeholder cell to a freshly-evaluated
closure and patch it later. ADR-015 broke the top-level `define`
cycle by making `Env::globals` a `Weak` back-edge to the Vm-owned
globals table; ADR-021 documented why the same fix can't apply to
`letrec` (closures must keep their own self-referential cells
alive) and pinned the residual leak with the diagnostic test
`letrec_cycle_persists_after_drop`. That ADR concluded the cleanest
fix is option 3 (Y-style desugaring) and noted: *"the CESK upgrade
(separate ADR, also deferred) would refactor Env's storage anyway,
so the letrec fix probably lands alongside or after CESK."* This
ADR is the separate one.

The gap CESK closes: the cell that holds a recursive binding has
to be reachable two ways — by the closure (so calls find it) and
by something not-the-closure (so the closure can drop without
keeping the cell alive). Today there is no such layer; the env's
frame slots and the closure's captured env are the same Rc-linked
data. CESK introduces a store as exactly that layer.

**What CESK is**: A four-register state machine in the Felleisen &
Friedman tradition (mid-1980s). CESK adds a **Store** that maps
addresses to values; the environment becomes a map from names to
addresses; lookup grows one indirection.

```
CEK today:   env: Sym → Rc<RefCell<Val>>   (cell is the binding)
CESK after:  env: Sym → Addr               (env names a slot)
             store: Addr → Val             (store holds the value)
```

Today's `Frame.slot: Rc<RefCell<Val>>` is already a proto-store —
a single-binding-at-a-time allocation that exists precisely to
support letrec's two-phase placeholder pattern. CESK reifies that
pattern, unifies allocation in one place, and lets the engine
treat bindings as data the Vm owns rather than as Rc graphs each
closure participates in.

**Decision**: Pin the design. Defer the build until a host or UX
need pulls it (see *Deferred* for the trigger conditions). When
pulled, follow the migration sketch below in the listed order.
The five CEK transitions remain five transitions; the diff is
"every place that reads or writes a binding now goes through the
store" plus one new file.

**Migration sketch** (concrete enough to act on when triggered):

1. **New `crates/lisp/src/store.rs`.** `pub struct Addr(u32);` and
   `pub struct Store(Vec<Val>);` with `alloc(Val) -> Addr`,
   `get(Addr) -> &Val`, `set(Addr, Val)`. `Addr` is `Copy`,
   no Rc. Vec-indexed is the simplest viable representation;
   revisit for a HAMT only if the snapshot/undo trigger (see
   below) is what un-defers this.
2. **`env.rs`.** `Frame.slot: Rc<RefCell<Val>>` becomes
   `Frame.slot: Addr`. `Env::lookup` returns `Option<Addr>` (or
   takes a `&Store` and returns `Option<Val>` — caller's choice).
   `extend_placeholder(name)` becomes "allocate a placeholder
   `Val::Bool(false)` in the store, return the new Env extended
   with that Addr plus the Addr itself for later patching."
3. **`step.rs`.** `State` grows a `store: Store` (or the driver
   threads it alongside). The five transitions thread it. The
   letrec arm at `step.rs:93` allocates via the store instead of
   `Rc::new(RefCell::new(...))`. The closure-creation arm at
   `step.rs:53` is unchanged — `Val::Clo { env, ... }` still
   captures env by clone; the env's slots are now Addrs.
4. **`k.rs`.** `K::Letrec.cells: Vec<Rc<RefCell<Val>>>` becomes
   `Vec<Addr>`. The patch step in `apply_k` (around `step.rs:178`,
   today `*cells[*next].borrow_mut() = v`) becomes
   `store.set(addrs[*next], v)`. Other K variants unchanged.
5. **`val.rs`.** `Val::Clo { params, body, env }` unchanged.
   The cycle dissolves by construction: closure → env → frame →
   Addr (plain int) → store (Vm-owned). `Addr` is `Copy`, so
   there is no Rc edge back from the closure to the cell.
6. **`lib.rs`.** `Vm` holds the store. Top-level `define`
   installation (`lib.rs:139` region) is unchanged in spirit —
   globals are still a `HashMap<Sym, ???>`, but `???` can become
   `Addr` and the slot value lives in the store alongside frame
   slots. Whether globals collapse fully into the store or remain
   a sibling region is a follow-on decision (see *Deferred*).

Mechanical surface: ~30–40 lines net change across the five
existing files, plus the one new file. The driver loop in
`step::run` grows a store argument; demo crates that call
`Vm::eval_str` see no signature change.

**Alternatives considered**:

1. **Status quo — CEK plus the pinned ADR-021 leak.** Legitimate.
   The leak is bounded at REPL scale (one-shot evals; the web
   REPL has a step budget). If no UX or host work pulls and no
   other CESK-shaped capability is wanted, this is the right
   answer. The cost of doing nothing is the leak's slow heap
   growth in letrec-heavy long-running sessions, plus the doors
   that stay closed (no mutation, no snapshots, no undo).
2. **Y-style desugaring only** (ADR-021 option 3). Rewrite
   `(letrec ((f init)) body)` at compile time into application of
   a fixed-point operator so the closure body doesn't reference
   `f` by name. Solves the leak; opens no doors. Right call *only
   if* the letrec leak alone forces action and CESK still hasn't
   been pulled by other capability demand. Cheaper to ship but
   strictly less capable than CESK. After CESK lands this option
   becomes moot — the store dissolves the cycle without the
   compile-pass.
3. **Closure conversion only** (ADR-021 option 1). Half-fix. More
   code than Y-style. Already rejected in ADR-021; listed here
   only to record that CESK does not change its standing.
4. **Custom hand-rolled cycle collector** (ADR-021 option 4).
   Heavy. Solves only the leak. Breaks ADR-002's zero-deps stance
   unless hand-rolled, in which case it's months of GC code for
   one specific cycle shape. Strictly worse than CESK once CESK
   is on the table.

**Consequences**:
- **+** The letrec leak goes away by construction. `Addr` is
  `Copy`, so closure → store has no Rc edge; the store is the
  sole strong owner of binding values, owned by the Vm.
- **+** `set!` becomes ~5 lines (`store.set(addr, new_val)`).
  Every closure that captured an env containing that Addr sees
  the update on the next lookup. The capability that today
  requires `Rc<RefCell<…>>` plumbing everywhere becomes a
  one-line change at the call site.
- **+** State snapshots become viable. A `Vec<State>` for undo or
  replay scrubbing is no longer blocked by "you can't cheaply
  snapshot Rc graphs." Store clone dominates the cost; with a
  persistent HAMT the snapshot becomes O(log n) per write.
- **+** The engine aligns with the standard CESK literature.
  Future features like continuation marks, delimited control,
  or formal small-step semantics have an established vocabulary
  and shape to draw on.
- **+** Globals can collapse into the store (one allocation pool,
  simpler ownership story) — though this is a separate decision.
- **−** Every variable lookup adds a store hop. Probably sub-µs at
  REPL scale; not measured yet. A criterion bench in
  `crates/bench/` would be the place to land the before/after.
- **−** The five transitions grow new store threading. Mechanical
  surface, ~30–40 lines net, but every state-transition site
  touches it.
- **−** Spell/gene/curve preludes are semantically unchanged but
  the existing test surface (60+ tests in `tests/eval.rs` plus
  the demo-prelude tests) must stay green throughout. The
  migration is "one transition at a time, keep tests green," not
  a single big-bang swap.
- **−** The snapshot use case will eventually want a persistent
  store (HAMT / im-rs-style) for cheap undo depth. A plain
  `Vec<Val>` is the right starting point but it caps undo at the
  cost of full-store clones per snapshot. The HAMT decision is
  deferred until the snapshot trigger fires (see below).
- **−** ADR-002's zero-deps stance constrains the persistent-store
  choice. A hand-rolled HAMT is feasible (the lisp crate is
  zero-deps today and would stay so); an `im` dependency would
  require revisiting ADR-002 for the lisp crate.

**Deferred**:

*Pull triggers — any one un-defers this ADR.*

1. A web-UI undo button or replay scrubber for the labs
   (Spells / Genes / Curves). This is the canonical case.
2. A first-class mutation primitive (`set!` or analogous), either
   in the language surface or as a host-level capability.
3. A host that needs reactive bindings (re-evaluate consumers
   when a cell changes — e.g. a spreadsheet-style lab).
4. A host observing material memory growth from letrec-heavy
   programs in long-running sessions. Today: not observed; the
   REPL is bounded by step budget and one-shot evals.
5. A formal-semantics or interpreter-spec write-up that wants the
   canonical CESK shape rather than the current CEK-with-cells.

*Follow-on decisions, after this ADR un-defers:*

- Persistent store representation: plain `Vec<Val>` first; HAMT
  (hand-rolled vs `im` dep) if the snapshot trigger is what
  un-defers and undo depth matters.
- Globals: collapse fully into the store, or keep as a sibling
  region. The Weak back-edge from ADR-015 may need re-derivation
  either way.
- `set!` semantic ADR: cheap to implement post-CESK; the question
  becomes whether the language *should* have it, not whether it
  *can*.
- Undo-button UX shape (out of scope here — that's the trigger,
  not the design).

*Regression targets when implemented:*

- `letrec_cycle_persists_after_drop` (`tests/eval.rs`) — the
  assertion flips from `is_some()` to `is_none()`. The test name
  and comment need updating, or the test is removed and a
  positive `letrec_does_not_leak` replaces it. Either way, the
  flip is the signal we want.
- Currently green and must stay green: `letrec_self_recursion`,
  `letrec_mutual_recursion`, `map_via_letrec`,
  `closure_captures_lexical_env`,
  `defines_in_one_eval_str_are_mutually_recursive`,
  `dropping_vm_releases_top_level_closures`,
  `define_over_prim_overwrites_globals_slot` (ADR-020).

**Postscript — implemented 2026-06-02**: shipped the same day the
ADR was drafted, against the design above. The migration came in
smaller than the sketch's ~30-40 line estimate:

- `crates/lisp/src/store.rs` — new, ~50 lines: `Addr(u32)` Copy
  newtype + `Store { cells: RefCell<Vec<Val>> }` with `alloc` /
  `get` / `set` / `len`.
- `crates/lisp/src/env.rs` — `Frame.slot` is now `Addr`. `Env`
  gained a `store: Weak<Store>` field alongside the existing
  `globals: Weak<…>`. `Env::with_globals(globals, store)` takes
  both. The previously-dead `Env::empty` was removed.
- `crates/lisp/src/k.rs` — `K::Letrec.cells` renamed `addrs:
  Vec<Addr>`. Other variants unchanged.
- `crates/lisp/src/step.rs` — letrec setup uses the new
  `extend_placeholder` (now returning `Addr`); `K::Letrec`'s
  apply step calls `env.store_handle().expect(...).set(addr, v)`.
  The five transitions did not need new signatures — the store is
  reachable from every `Env`, so `step` / `run` / `run_bounded`
  signatures stayed identical.
- `crates/lisp/src/lib.rs` — `Vm` gained `store: Rc<Store>` and
  `store_weak()` (for the diagnostic test). `Vm::new` wires it
  through `Env::with_globals`.
- `crates/lisp/tests/eval.rs` — `letrec_cycle_persists_after_drop`
  flipped to `letrec_does_not_leak`. New assertion: after Vm drop,
  the `Weak<Store>` taken pre-drop fails to upgrade.

All 121 workspace tests pass. WASM build clean. `just check`
clean. No host crates required changes — everything funnels
through `Vm`, whose external surface was preserved.

What was *not* changed (preserved as separate decisions):
- Globals stayed as `Rc<RefCell<HashMap<Sym, Rc<RefCell<Val>>>>>`
  — not collapsed into the store. The ADR-015 `Weak` back-edge
  pattern still applies end-to-end.
- Persistent store representation — `Vec<Val>` ships; HAMT remains
  a future decision if the snapshot/undo trigger pulls.
- No `set!` primitive added — that's a separate ADR if/when
  pulled.
- `Vm::heap_summary()` not added — `Vm::store.len()` is
  sufficient for ad-hoc diagnostics today.

## ADR-024: Macros extracted to sibling crate (2026-06-04)

**Context**: A retrospective audit of "have we drifted from the
original smallest-substrate thesis" (see day-one prelude in
`web/let-rs.html`) flagged macros as the single largest drift.
The `defmacro` + quasiquote-with-macros + procedural expansion
machinery in `crates/lisp/src/lib.rs` was roughly half the size
of the engine itself: the `Macro` struct, the `macros: Rc<RefCell<
HashMap<…>>>` field on `Vm`, `try_register_defmacro`,
`expand_all`, `expand_in_qq`, `expand_macro_call`, `val_to_datum`,
plus the macro-aware branch inside `eval_str_inner`. Every host
paid for macros even if it never registered one. The DSL packs
(spells, genes, curves) don't use `defmacro` — only the user-
facing REPL surfaces (web bridge + `examples/repl.rs`) do.

This ADR follows the ADR-016/017/018 sibling-crate pattern: lift
a feature out of the engine when its blast radius is bigger than
its consumer footprint.

**What stays in `lisp`**: parser-level quasiquote (` `` `, `,`,
`,@`). These compile to list-construction expressions
(`parse::compile_quasiquote_form`) and work without macros
installed. The runtime tests `quasiquote_basics`,
`quasiquote_splice`, and `quasiquote_nested_depth_preserved`
verify that.

**What moves to `crates/macros/`**:

- `Macro` struct (closure + variadic flag)
- `Expander` struct holding the `HashMap<String, Macro>` table
- `expand_all`, `expand_in_qq`, `expand_macro_call`,
  `try_register_defmacro` (all now `&mut Expander` methods
  taking `&mut Vm` as a parameter)
- `val_to_datum` (only used to round-trip macro return values)
- New `MacroVm` convenience wrapper bundling `Vm` + `Expander`
  with a macro-aware `eval_str(src)`

**Decision**: Land the extraction. Default the engine to
macro-unaware. Hosts opt in by wrapping a `Vm` in
`macros::MacroVm` (or threading an `Expander` manually).

**Migration sketch (concrete)**:

1. New `crates/macros/` crate (depends on `lisp` only).
2. `Vm::call_value` made `pub` (so the macros crate can invoke
   macro closures); `Vm::env()` accessor added (so macros can
   capture the root env for closures they register).
3. `Vm` loses its `macros` field and all expansion methods.
4. `eval_str_inner` drops the `try_register_defmacro` + `expand_all`
   steps and just compiles each form directly. `eval_str`'s
   atomic snapshot drops `saved_macros` (only globals now).
5. `MacroVm::eval_str` parses, splits out `(defmacro …)` forms
   (registers in the Expander) from the rest (expands them, then
   serializes back through `Vm::eval_str`). The round-trip
   through the reader is the price of going through the public
   entry point; lossless for our Datum set (Num/Ratio/Bool/Sym/
   List). MacroVm wraps both Vm-level and macros-table
   atomicity.
6. Existing macro tests (7) moved to `crates/macros/tests/`. A
   new test pins the engine's macro-unawareness:
   `defmacro_unknown_to_raw_vm` asserts `lisp::Vm::eval_str(
   "(defmacro foo () 1)")` is an error.
7. Consumers updated:
   - **`wasm`** (web bridge / user-facing REPL): adds `macros`
     dep, swaps `inner: lisp::Vm` → `inner: macros::MacroVm`.
     Casts pass through MacroVm with macro expansion as a no-op
     when no macros are present.
   - **`examples/repl.rs`** (CLI REPL): swaps to MacroVm so
     interactive `(defmacro …)` still works. Added as a dev-dep
     loop (lisp dev-deps on macros, macros deps on lisp — Cargo
     allows the loop because dev-deps don't participate in the
     library's dep graph).
   - **CLI examples** (`spells.rs`, `genes.rs`, `curves.rs`,
     `world.rs`): unchanged. Their preludes are pure defines, and
     they don't take user input that could contain `defmacro`.

**Alternatives considered**:

1. **Feature flag in lisp** (`#[cfg(feature = "macros")]`). Code
   still lives in `lisp/`; just gated. Doesn't actually reduce
   the engine's surface area or align with the sibling-crate
   pattern. Rejected.
2. **Trait/hook on `Vm`** so external code can inject an
   expander. Adds extension points the engine doesn't otherwise
   need. Heavier than the wrapping pattern that already works
   for `world`/`spells`/`genes`/`curves`.
3. **Status quo (keep macros in lisp)**. Concrete cost: ~250 net
   lines in the engine for a feature half the consumers don't
   use. The audit specifically called this out as drift.

**Consequences**:
- **+** `lisp` crate drops ~250 lines of macro-expansion
  machinery; the engine's surface re-aligns with the "smallest
  interesting substrate" thesis.
- **+** Hosts that don't want macros stay on `lisp::Vm` with no
  macro tax (no expander field, no per-`eval_str` walk to look
  for macro calls).
- **+** Future macro-system experiments (hygienic macros,
  reader macros, a different expansion strategy) can live in
  `crates/macros/` (or a sibling) without touching the engine.
- **+** Pinned by the new `defmacro_unknown_to_raw_vm` test —
  any regression that pulls macros back into `lisp` flips it.
- **−** `MacroVm::eval_str` round-trips expanded datums through
  the reader (serialize → parse → compile). Cheap for REPL
  scale; could be replaced by exposing a `Vm::eval_datums`
  entry point if it ever measures hot.
- **−** Hosts that want macros + a `Vm` field need to reach the
  inner engine via `macro_vm.vm` (`pub vm: Vm` on MacroVm).
  Minor verbosity in `crates/wasm/src/lib.rs` (e.g.
  `spells::install_with_world(&mut inner.vm, …)`).
- **−** Loop in dev-deps: `lisp` dev-deps on `macros` for the
  REPL example, and `macros` deps on `lisp`. Cargo handles
  this because dev-deps don't enter the library's dep graph.
  Documented in `crates/lisp/Cargo.toml`.

**Deferred**:
- A `Vm::eval_datums(forms)` entry point that would let
  `MacroVm::eval_str` skip the serialize round-trip. Worth
  doing only if a benchmark shows the round-trip is measurable.
- Hygienic macros (gensym + renaming pass). Listed in the
  let-rs.html "what comes after"; would live in `crates/macros/`
  when picked up, with the choice between "hygienic by default"
  and "opt-in via a separate macro form" still open.
- Reader macros / custom dispatch. Out of scope; the parser is
  in `lisp` and would need a hook for that.

## ADR-025: Spells prelude adopts `defspell`/`defparam` macros (2026-06-05)

**Context**: After ADR-024 lifted macros to a sibling crate, the
macros stdlib (`install_stdlib` → `begin`/`when`/`unless`/`and`/
`or`) was sitting ready for a concrete consumer. The DSL packs
(spells/genes/curves) still targeted raw `lisp::Vm` with hand-
rolled `(define …)` preludes. The rune prelude in particular was
nine repetitions of the same shape:

```
(define fire (lambda (ctx) (assoc-set 'element 'fire ctx)))
(define ice  (lambda (ctx) (assoc-set 'element 'ice  ctx)))
...
(define area  (lambda (n) (lambda (ctx) (assoc-set 'area  n ctx))))
(define power (lambda (n) (lambda (ctx) (assoc-set 'power n ctx))))
```

Two clear shapes: constant ctx setters and parametric (closes
over a number). Perfect macro fodder, and the ADR-024 narrative
needed a real consumer to prove the extraction earns its keep.

**Decision**: Adopt `MacroVm` as the spells host. Register two
local macros at the head of the prelude:

```
(defmacro defspell (name key val)
  `(define ,name (lambda (ctx) (assoc-set ',key ',val ctx))))

(defmacro defparam (name key)
  `(define ,name (lambda (n) (lambda (ctx) (assoc-set ',key n ctx)))))
```

Then expand the rune vocabulary into nine one-liners
(`(defspell fire element fire)` etc). The two macros live inside
the spells prelude rather than the macros stdlib because their
shape is spell-DSL-specific (the assoc-set call shape is the spell
ctx convention, not a general lisp pattern).

`spells::install` and `spells::install_with_world` now take
`&mut MacroVm` instead of `&mut Vm`. Consumers (`examples/
spells.rs`, `crates/lisp/tests/world.rs`, `crates/lisp/tests/
eval.rs`'s leak test, `crates/bench/benches/demos.rs`, the WASM
bridge) all switched from `Vm::new()` to `MacroVm::new()`.

**Alternatives considered**:

1. **Host-side spell macros, keep prelude as raw defines.** Add
   a separate `spells::install_macros(mvm)` and keep
   `spells::install(vm)` for raw Vm. Hosts call both. Lighter
   coupling but defeats the demonstration — the *prelude itself*
   has to use defspell for the win to be visible in the source.
2. **Compile-time macro expansion (codegen).** A build script
   that runs the macros over a small DSL and emits expanded
   defines as Rust string constants. Bypasses MacroVm entirely
   but adds a build-time dependency and means the runtime
   doesn't actually exercise the macros.
3. **Status quo (hand-roll defines).** Cheap, but the macros
   stdlib stays "potential energy" forever. ADR-024 doesn't
   pay rent.

**Migration sketch (concrete)**:

1. `crates/spells/Cargo.toml` gains `macros = { path = "../
   macros" }` dep.
2. `crates/spells/src/lib.rs` PRELUDE_DEFINES rewritten with
   defspell/defparam; `install(&mut MacroVm)` and
   `install_with_world(&mut MacroVm, world)`. The world prims
   install still goes through the inner `mvm.vm`.
3. WASM bridge: change `spells::install_with_world(&mut inner.vm,
   …)` to `spells::install_with_world(&mut inner, …)`. Single
   token edit; the bridge already used `MacroVm::with_stdlib()`.
4. `examples/spells.rs`: `Vm::new()` → `MacroVm::new()`; `cast`
   signature updated.
5. `crates/lisp/tests/world.rs`, `crates/lisp/tests/eval.rs`
   (leak test), `crates/bench/benches/demos.rs`: same treatment.
   Bench Cargo gains `macros` dep.
6. New tests in `crates/spells/tests/prelude.rs` pin the
   defspell/defparam expansion shape end-to-end (`fire`/`ice`/
   `area` semantics, install_with_world wires world-apply).

**Latent bug surfaced** (and fixed): `Expander::expand_all`
unconditionally rejected `(define …)` at any list head. This was
correct for nested positions but wrong at top level — and the
recursive expansion of a top-level macro that produced `(define
…)` (which is exactly what defspell does) also hit it. Fix: split
into `expand_top_level` (allows define, re-enters at top level
after macro expansion) and `expand_all` (continues to forbid
define). Two new tests in `crates/macros/tests/macros.rs` pin
both top-level `(define …)` and macro-produced top-level
`(define …)`, plus the inverse — `(let ((x 1)) (define y 2))`
still errors.

**Consequences**:
- **+** ADR-024's extraction now has a real consumer: the spells
  prelude is the first DSL pack to actually use the macros
  stdlib pattern. The interlude has content.
- **+** Adding a new constant rune is a one-liner: `(defspell
  NAME KEY VAL)`. Parametric: `(defparam NAME KEY)`. Anything
  fancier still wants a hand-written `(define …)`.
- **+** The expander's top-level handling is now correct in
  general — any macro that expands to `(define …)` works.
  Opens the door for similar `defcodon`/`defgene` patterns in
  the genes prelude.
- **−** `spells` crate now depends on `macros`. Was lisp+world
  only; now adds the third edge. Acceptable — the dependency
  matches the actual runtime requirement.
- **−** Host code that wanted a raw `Vm` + spells prelude no
  longer compiles unchanged; must wrap in `MacroVm`. One-line
  fix at each call site (five total in this repo).
- **−** Spells prelude install cost goes up slightly (MacroVm's
  per-form expand_top_level pass runs across the prelude). At
  install-once-per-session scale this is irrelevant; the bench
  hot path (`bench_cast_spell`) doesn't change because casts
  run through `eval_str` on the body, not the prelude.

**Deferred**:
- `defcodon` / `defgene` analogues for the genes prelude. Same
  shape; would let the genes crate drop the `Rc<RefCell<i64>>`
  seed plumbing's outer scaffolding (the inner mutate closures
  still need lexical scope). 1-2 hours; mostly mechanical.
- A `defstroke` for curves. Less obvious payoff because strokes
  are quoted symbols that `draw!` dispatches on — no closure
  to abstract. Probably stays as the stroke→symbol table.
- Hygiene. The `__or-val__` caveat in `macros::STDLIB` is the
  baseline concern; defspell/defparam shadow `n` and `ctx`
  inside their lambdas, which collides with user-bound `n` /
  `ctx` only inside the lambda body — vanishingly unlikely but
  worth noting.

## ADR-026: `set!` — first-class mutation (2026-06-05)

**Context**: Stage 1 of the "dynamic spells" arc. The CESK store
landed in ADR-023 specifically so frame slots could be mutated
without rewinding the env shape; what was missing was a syntactic
form to do it. ADR-007 (numeric tower), ADR-014 (installable
preludes), and ADR-019 (the standing `(let ((_ a)) b)` sequencing
workaround) all assumed a purely-functional surface. With CESK in
place and a stages plan for a more game-like spell demo (decay,
mana, caster-side state), the engine needed `set!` to make
caster-side state expressible in lisp rather than threaded
through every call.

**Decision**: Add `(set! name val)` as a parse-level special
form. The form evaluates `val` in the current env, then writes
the result into the slot `name` resolves to — frame slot (via the
store) or globals cell. Returns the new value. Unbound names
error. Lexical scoping rules apply: an inner `let` shadowing the
outer binding receives the mutation; the outer slot stays intact.

**Implementation sketch**:

- `Expr::SetBang(Sym, Rc<Expr>)` in `expr.rs`.
- `K::SetBang { name: Sym, env: Env, k: Rc<K> }` in `k.rs`. We
  capture the *env* at the set! site rather than resolving the
  addr up front so set! behaves like `Var` for forward
  references — the same rules that let mutual top-level defines
  work apply here.
- `parse.rs::compile_set_bang` — recognized via the existing
  special-form dispatch alongside `lambda`/`if`/`let`/etc.
- `Env::set(&self, name, val) -> Result<(), String>` — walks the
  frame chain, writing through `store.set(addr, val)` on hit;
  falls through to the globals `Rc<RefCell<Val>>` cell; errors
  on miss with `unbound variable: NAME`.
- Two new arms in `step.rs`: `Expr::SetBang` pushes
  `K::SetBang`, evals the val; `K::SetBang` writes via
  `env.set`, returns the just-evaluated value.
- `macros::Expander::expand_all` learns about set! the same way
  it knows about lambda: leave items[1] (the name) alone, expand
  items[2] (the value) normally. Without this, a macro with the
  same name as the target binding would silently rewrite the
  reference.

About 60 lines across the engine; tests double that.

**Return-value choice**: returns the new value (Common Lisp
style), not unspecified (R7RS style). Two reasons: (1) it's
useful in tail position where the caller wants the value anyway
— `(lambda () (set! n (+ n 1)) n)` reduces to `(lambda () (set!
n (+ n 1)))`; (2) it surfaces the value as a first-class result,
which the test suite leans on (the engine has no `begin` — that's
a macro in the sibling crate — so `(let ((_ side)) body)` is the
sequencing pattern, and set!'s return value is what makes the
underscore-bind read naturally).

**Globals via `Rc<RefCell<Val>>`, not via the store**: the store
is for frame slots. Globals stayed in `Rc<RefCell<Val>>` across
the CESK migration (ADR-023) so the ADR-015 Weak back-edge
pattern still worked, and that decision pays off here — `set!`'s
globals write path is one `borrow_mut`, no store routing.

**Alternatives considered**:

1. **Box mutation in user code** (`(define box (lambda () (let
   ((cell '()))(lambda (op v) (if (eq? op 'get) cell (set!
   cell v)))))`)). Doesn't work — needs `set!` already. The base
   case has to live in the engine.
2. **`(set-cell! cell v)` prim taking a manually-Rc'd cell type
   as a new `Val` variant.** Hides mutation in a host type;
   pushes complexity into both the engine (new variant) and the
   host (`Cell` constructor prim). Worse than just adding the
   form.
3. **Defer until a host needs it.** Two hosts named it
   simultaneously: the planned mana meter (stage 3) needs
   caster-side state, and the dev log's Act VII coda listed
   "set! is now five lines" as a substrate property. Stage 1 of
   the staged plan landed here.

**Consequences**:
- **+** Closure-over-let-binding counters now compose: the
  classic `(let ((n 0)) (lambda () (set! n (+ n 1)) n))` pattern
  works. Tests pin three repeated calls returning `1`/`2`/`3`.
- **+** The CESK store proves its weight beyond just dissolving
  the letrec leak — the mutation path is short specifically
  because `Addr` indices into a mutable `Vec<Val>` were always
  the right shape.
- **+** Unblocks stages 2-4 of the dynamic-spells arc: tile
  decay (host-mutable, doesn't need set! but coexists with it),
  mana meter (does need set!), UI wiring.
- **+** Parser-level discipline: `set!` joins the same special-
  form list as `lambda`/`if`/`let`/`letrec`. The form is rare
  enough in idiomatic lisp that this isn't a syntax sprawl.
- **−** Reasoning about effects now requires tracking what's
  mutable. Functional purity by convention; the substrate no
  longer enforces it. The DSL packs (spells/genes/curves) don't
  use `set!` today and probably shouldn't — their preludes are
  meant to be pure pipelines.
- **−** Hygiene exposure widens. A macro that expands to
  `(set! x …)` mutates the caller's `x`, with no rename pass to
  prevent collision. Documented in the macros crate; not fixed.
- **−** The `(let ((_ x)) y)` sequencing pattern persists in
  engine tests because lisp/tests/* can't use the macros crate's
  `begin`. Acceptable — these tests already used the pattern.

**Deferred**:
- `set-car!` / `set-cdr!` for in-place cons mutation. The store
  doesn't reach into Val structure; this would be a separate
  decision (and probably needs a new addressing scheme inside
  Cons). Not pulled by any consumer yet.
- An `unset!` / `unbind` form. Not requested; the store has no
  reclamation today (ADR-023), so a frame-slot unbind would be
  cheap to express but doesn't free anything.
- A `parameterize` / dynamic binding form. Distinct from `set!`
  (block-scoped, push/pop on entry/exit); CESK makes both
  expressible. Pull when a consumer wants thread-locals or
  per-cast overrides.

## ADR-027: Tile decay — finite-lifetime painted tiles (2026-06-05)

**Context**: Stage 2 of the dynamic-spells arc. After ADR-026
shipped `set!`, the next visible step toward "game" was making
the world feel temporal — tiles that fade rather than persist
forever. The spells demo's central interaction (paint tiles, see
them stay) is a static snapshot. Decay turns it into a small
loop: cast, watch, recast.

**Decision**: Every tile carries a `u8` lifetime stored in a
parallel `lifetimes: Vec<u8>` on `World`. Lifetime `0` means
permanent (the legacy `set_tile` path stays unchanged).
`world-apply!` writes lifetime from ctx `power` (default 5 when
absent). A new `(world-tick!)` prim decrements every positive
lifetime by 1 and reverts tiles to `Floor` when their lifetime
hits zero, returning the count of reverted tiles.

The decay model is a host concern, not an engine concern — it
lives entirely in `crates/world/`. The lisp engine doesn't know
about lifetimes; from its perspective, `world-apply!` and
`world-tick!` are opaque effecting prims like every other host
prim.

**Implementation sketch**:

- `World.lifetimes: Vec<u8>` parallel to `tiles`.
- `World::set_tile_with_lifetime(x, y, t, lifetime)` paints with
  a finite life; the legacy `set_tile` writes `lifetime = 0`
  (permanent), preserving behavior for `world-set-tile!` callers
  that paint walls etc.
- `World::tick(&mut self) -> u32` decrements each positive
  lifetime; tiles that hit zero this tick revert to Floor and
  add to the returned count.
- `World::lifetime_at(x, y) -> Option<u8>` for tests and any
  future renderer that wants to colorize by remaining life.
- `world-apply!` in `world_prim.rs` reads ctx `power`:
  - `power > 0` → clamp to `u8::MAX`, use as lifetime
  - `power <= 0` → 0 (permanent — opt-out)
  - missing → `DEFAULT_LIFETIME` (currently `5`)
- `world-tick!`: zero-arg prim, returns `Val::Num(reverted)`,
  logs `tick → N reverted` when N > 0.

**Choice of u8**: lifetimes top out at 255 ticks — well within
the demo's needs (a 500ms tick gives ~2 minutes at max). Keeps
the parallel `Vec<u8>` small and `Copy`, and bounds the cast at
the lisp/Rust boundary cleanly. If a host wants longer-lived
tiles, the cap moves to u16 — straightforward but premature.

**Permanent-as-zero**: lifetime `0` doubles as "permanent" so
the existing `set_tile` path (used by `world-set-tile!` for
direct wall painting) doesn't need to change. Alternative was
`Option<u8>` which is the same byte-count but adds a None branch
to every iteration of `tick`. The `0 = permanent` convention is
fine because Floor itself doesn't decay to anything visible, so
"a Floor tile with lifetime 5" would be a no-op anyway.

**Default lifetime (5)**: arbitrary but pinned by a test
(`world_apply_without_power_uses_default_lifetime`). Five ticks
× 500ms tick interval = 2.5s of visible fire — long enough to
read, short enough that a recast resets it. Tuneable later;
flagged as a magic number in the prim doc-comment.

**Alternatives considered**:

1. **`Tile` enum carries lifetime.** `Tile::Fire { lifetime: u8 }`.
   Every match site grows a destructure; the `Copy` enum stays
   small but no longer trivially equatable; `from_sym` and
   `glyph` need to thread lifetimes that are irrelevant for
   their purposes. Lifetime is a cell property, not a tile-kind
   property — separating them in storage matches that.
2. **Wall-clock decay** (`Instant`-stamped tiles, decay = now -
   stamp). Web hosts don't easily reach `Instant`; the demo's
   "tick" feels more like a turn anyway. A clock-driven decay is
   fine for an action game; turn/tick fits the spell-cast model.
3. **Defer until a use case shows up.** That use case is the
   labs UI calling `world-tick!` on an interval. Same answer.

**Consequences**:
- **+** The spell lab becomes a loop: cast → watch decay →
  cast again. First time the demo has temporal behavior.
- **+** Sets up stage 3 cleanly: a mana meter that regens on
  `world-tick!` reuses the same tick the decay model fires on.
  One global tick, two effects.
- **+** Decay is testable without touching lisp at all
  (`crates/world/tests/decay.rs` — 7 unit tests).
  Integration through lisp covered by 5 more in
  `crates/lisp/tests/world.rs`.
- **+** `Vec<u8>` parallel to `Vec<Tile>` has negligible memory
  cost (8x8 grid = 64 bytes); `tick` walks the slab linearly
  with no branching past the `> 0` check.
- **−** The world log format changed: `cast fire at (1,1) area=0
  → 1 tiles` is now `cast fire at (1,1) area=0 life=5 → 1
  tiles`. Anything parsing log strings would break; the only
  consumer parsing them today is the eyeball.
- **−** `world-apply!` now writes both tile + lifetime; the
  effect surface grew. Still one prim from the lisp side.
- **−** Adding decay slightly complicates the "the substrate is
  pure" framing — the world struct now has time, in a sense.
  Honest tradeoff: the demo needs time to feel alive.

**Deferred**:
- Tile-kind-specific decay rates (fire decays faster than ice,
  walls don't decay at all). Trivial extension; not pulled.
- Visual decay (lighter glyph as lifetime drops). The current
  `glyph()` is a single char per tile kind. A `glyph_with_life`
  variant or a small ASCII gradient is a UI move, not a model
  move.
- Tile interactions (fire on ice → steam → floor; water on
  fire → extinguish). The interesting "spell composition over
  time" game move. Distinct from decay; layers on top.
- Auto-tick. Hosts call `world-tick!` directly today (the lab
  UI will set up a JS `setInterval`). A Rust-side tick loop
  would let CLI demos animate, but the CLI demos are
  snapshot-oriented — no use case yet.

**Postscript (2026-06-05)**: shortly after stage 4 shipped, the
`ᛃ` (JERA) rune was added for explicit `duration` control.
Lifetime selection in `world-apply!` is now `duration > power >
DEFAULT_LIFETIME` — duration is the explicit knob, power keeps
its pre-rune behavior as a fallback so every previous cast site
works unchanged. The cost formula in ADR-028 picked up duration
as another knob.

## ADR-028: Mana model — caster-side resource via `set!` (2026-06-05)

**Context**: Stage 3 of the dynamic-spells arc. ADR-026 shipped
`set!`; ADR-027 added tile decay + `(world-tick!)`. Both pieces
were inert without a consumer that uses persistent state to
constrain casting. The mana model is the consumer: a small
budget the caster spends to cast and regenerates on tick. It
closes the loop — cast → spend → wait → recast — that turns the
spell lab from a one-shot painter into a small game.

**Decision**: The mana model lives in the spells prelude (not
the engine, not the world). The spell DSL owns its resource
model the same way it owns the rune vocabulary. Three globals
plus three wrappers:

- `max-mana = 10` — the cap
- `mana = max-mana` — the current value
- `assoc-or` — helper: `(assoc-or k ctx default)` returns the
  value at key `k` in `ctx` or `default` if missing
- `spell-cost` — `(+ 1 (assoc-or 'power ctx 0) (assoc-or 'area ctx 0))`
- `cast!` — mana-gated wrapper around `world-apply!`: refuses
  on shortfall (logs `mana-short`, returns 0), decrements mana
  + paints on success
- `tick!` — wrapper around `world-tick!`: advances world decay,
  then regens one point of mana (capped at `max-mana`)
- `reset-mana!` — restore mana to `max-mana`; called by the
  WASM bridge's `reset_world`

The WASM bridge swaps its `cast` from `world-apply!` to `cast!`,
adds `tick()`, `mana()`, `max_mana()` accessors, and calls
`(reset-mana!)` inside `reset_world`. CLI examples and the
`crates/lisp/tests/world.rs` integration tests continue to call
`world-apply!` directly — they're testing the world prim, not
the mana flow, and the bypass keeps those concerns separable.

**Implementation sketch**:

```
(define cast!
  (lambda (ctx)
    (let ((cost (spell-cost ctx)))
      (if (< mana cost)
          (let ((_ (world-log! 'mana-short cost mana))) 0)
          (let ((_ (set! mana (- mana cost))))
            (world-apply! ctx))))))

(define tick!
  (lambda ()
    (let ((reverted (world-tick!)))
      (let ((_ (if (< mana max-mana)
                   (set! mana (+ mana 1))
                   #f)))
        reverted))))
```

About 30 lines added to `crates/spells/src/lib.rs::PRELUDE_DEFINES`;
~40 lines added to the WASM bridge (mostly accessors); 10 tests
in `crates/spells/tests/prelude.rs`.

**Alternatives considered**:

1. **Mana lives in a Rust-side `Caster` struct + prims.** Mirrors
   the world: a host-mutable resource exposed as `(caster-mana)`,
   `(caster-spend! n)`. Pushes complexity into the host; doesn't
   exercise `set!`. Worse: makes the mana cap a Rust constant
   instead of a configurable global a host or user can rebind.
2. **Mana embedded in `ctx`.** Thread mana through the spell
   pipeline as an alist key. Pure functional, no `set!` needed.
   But then every cast site has to read-write the value, and the
   bridge can't expose a stable mana state — the value lives
   inside the threaded ctx that gets thrown away after each cast.
   The "small mutable globals" shape matches what mana actually
   is.
3. **Per-spell cooldowns instead of a unified budget.** More
   game-like, but a much larger design space (which spells share
   cooldowns? how long? do they decay?). Mana is one number; the
   demo doesn't yet earn the complexity.

**Why a fresh `assoc-or` helper instead of just `assoc-get`**:
`assoc-get` returns `Val::Nil` on miss, and `Val::Nil` is truthy
in the engine (only `Val::Bool(false)` is falsy). `(if (assoc-get
…) …)` wouldn't catch the "key absent" case. `assoc-or` makes the
default explicit and keeps the cost formula readable. Worth
exposing in the prelude because power/area are both naturally
absent in many ctx shapes.

**Cost formula choice (`1 + power + area`)**: every cast costs at
least 1 mana, so even a bare `(fire)` draws down the budget.
Power and area add to it linearly so heavier spells feel
heavier. Not tuned beyond "playable" — the constants are pinned
by tests, easy to retune.

**Consequences**:
- **+** First place in the codebase where `set!` does real work
  beyond a counter test. ADR-026 has a consumer.
- **+** First place where `world-tick!` does real work beyond
  tile decay. ADR-027 has a consumer.
- **+** The spell DSL grew a resource model without the engine
  growing a `Resource` concept. The pattern generalizes — a
  health pool, a turn counter, anything caster-side can layer in
  the same way.
- **+** Mana is a regular lisp global. A user can rebind
  `max-mana` from the REPL (`(set! max-mana 25)`) and the next
  `reset-mana!` picks it up. Live retuning.
- **−** The `cast!` wrapper sits between every cast site and the
  world prim. CLI demos that called `world-apply!` directly
  (examples/world.rs) bypass mana. Acceptable — they're
  demonstrating the world layer, not the DSL flow.
- **−** Log format now includes `mana-short` entries on
  refused casts. Anything parsing the log would need to handle
  three event shapes (cast, mana-short, tick-revert).
- **−** Two ways to mutate the world from the bridge:
  `world-apply!` (raw, no mana) and `cast!` (mana-gated). The
  bridge uses cast!; tests use the raw prim where appropriate.
  Mostly a docs concern.

**Deferred**:
- Mana regen rate other than 1-per-tick. Trivial: change the
  `(+ mana 1)` constant or expose a `mana-regen` global.
- Spell-specific costs (fire cheap, lightning expensive). Needs
  a cost table keyed by element. A natural follow-on to
  defspell — `(defspell fire element fire 1)` could pin the
  element AND the cost in one form. Pulls when a real spell
  library distinguishes elements by cost.
- A "channel-cost" pattern where holding the spell drains mana
  over time. Needs a per-frame hook and a separate state model.
  Way past the demo's current scope.
- UI for the meter, including a visible "wait for mana" cue.
  That's stage 4 of the dynamic-spells arc.

**Postscript (2026-06-05)**: stage 4 shipped, then `ᛃ` was
added for explicit `duration`. Cost formula became
`1 + power + area + duration` — every knob the user dials up
on the tape costs them mana. This is the first cost knob that
*doesn't* affect the cast radius (area widens the paint;
power/duration both extend it in time). The deferred
"spell-specific costs" bullet is one ADR-029 away.

