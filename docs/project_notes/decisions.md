# Decisions

Architectural decision records, backfilled at the end of day one. Each entry
captures the choice as it was made and the alternatives that were on the table
at that moment. If a decision is later reversed, mark it superseded but leave
the original entry — the reasoning is part of the history.

## ADR-001: CEK abstract machine as the evaluator (2026-05-25)

**Context**: Letrs needs an interpreter for a small functional lisp. The
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
