# letrs core · next-slice follow-ups

Engine-level work surfaced while building the genes demo (ADR-011,
ADR-012, ADR-013). The demos validated the "vocabulary on top"
architecture cleanly — zero engine changes were needed for either
spells or genes — but they made a few concrete places where the
engine could give meaningfully better ergonomics or push past real
expressiveness limits.

This is a prioritized queue. Each item has enough context that a
future session can pick it up cold.

---

## High-impact (engine-level)

### 1. Installable preludes  +  top-level `define`

**Problem.** Today the only way to install bindings into a `Vm`'s
env is to send a full source string through
`parse → expand_all → compile → run` on every call. Both demos do
this — every `cast()` re-parses and re-evaluates the entire prelude
(~25 lines for spells, ~30 for genes) just to evaluate a one-line
body. Identical work, repeated per call. With WASM in the loop this
is the dominant cost.

Symptom in the codebase:

```rust
// crates/wasm/src/lib.rs (cast)
let src = format!(
    "{}  (world-apply! (assoc-set 'tx {x} ...))",
    spells::PRELUDE_BINDINGS,  // ~25 lines, re-parsed every cast
);
self.inner.eval_str(&src)
```

The architectural cause is related: the lisp has **no top-level
`define`**. Everything must live inside `letrec`, which is why both
preludes are giant single letrecs. The two issues collapse into one
design: a way to extend the `Vm` env with new bindings *in place*,
without wrapping subsequent code.

**Approach.** Two paths, possibly converging:

- **`Vm::install_prelude(src: &str)`** — evaluate a letrec-shaped
  source once, walk the resulting bindings, merge each into
  `self.env` via `Env::extend`. Future `eval_str` calls see those
  bindings. The CEK rule for letrec already builds the right env
  extension; this would expose that as a public capability.
- **Top-level `(define name value)`** as a parser-recognized form
  (mirroring `try_register_defmacro` at `crates/lisp/src/lib.rs:90`).
  Each `define` extends the Vm's env in place. Then preludes can
  be written as `.scm` files of top-level defines and loaded via
  `vm.eval_str(file_contents)`.

The second subsumes the first if `eval_str` recognizes the
top-level shape (a sequence of defines + body) and treats it
correctly. That's likely the right end state: drop "implicit
letrec wrapping" entirely, replace with "load a file of
top-level forms, each one either a `define`/`defmacro` (env
extension) or an expression (value)."

**Files to touch.**
- `crates/lisp/src/lib.rs` — split `eval_str` into "compile to
  Expr" and "run against env"; add `install_prelude` and/or
  recognize top-level `define`.
- `crates/lisp/src/parse.rs` — if going the `define` route, add it
  as a special form in `compile` (parallel to `lambda`, `let`,
  `letrec` in `expand_all` at lib.rs:152).
- `crates/lisp/src/genes.rs` and `crates/lisp/src/spells.rs` —
  prelude strings move from "letrec body" shape to "sequence of
  top-level defines" shape (or stay as letrec, depending on
  approach).
- `crates/wasm/src/lib.rs` constructor — call
  `install_prelude(spells::...)` + `install_prelude(genes::...)`
  once instead of inlining the prelude into every cast.
- Both `examples/spells.rs` and `examples/genes.rs` similarly.

**Open design questions.**
- Does `install_prelude` *replace* existing bindings or shadow
  them? (Today `Env::extend` shadows. Probably keep that.)
- Does top-level `define` allow forward references between
  defines? (Real Scheme distinguishes `define` from `letrec` here.
  Letrec-style — all defines visible to each other — matches
  expected behavior, but requires either two-pass eval or
  forward-reference cell allocation like `letrec` already does.)
- What does `vm.install_prelude` do if the source has side
  effects? (Spell prelude is pure closures; gene prelude is too.
  But the door's open.)

**Scope estimate.** ~100 LOC of engine changes (lib.rs + parse.rs)
plus prelude reshape across genes/spells. Two integration tests
(prelude installs, subsequent eval sees the bindings; defines are
mutually recursive). Demos and bundle should shrink slightly
since the prelude string isn't carried into every call.

**Why it matters beyond perf.** This is the architectural shift
that makes "Vm + DSL pack" a real concept. Once preludes are
installable, a DSL becomes "a const string + a Rust prim
installer." Two lines to add a new DSL to a Vm.

---

### 2. Numeric tower (rationals or floats)

**Problem.** `Val::Num(i64)` is the only numeric type
(`crates/lisp/src/val.rs:10`). The mutation rate wanted to be
`0.05` but had to become `5` (integer percent). I worked around
it by passing percentages everywhere, but this was the first
place a non-toy demo wanted a fractional value and the language
couldn't say it.

Symptom: `genes::PRELUDE_BINDINGS` uses
`(mutate (lambda (ctx) (mutate! 25 seed ctx)))` — the `25` reads
as a magic number. With floats or rationals it could be `0.25`,
and the prim could accept rates in their natural form.

**Approach.** Two real choices:

- **Rationals** — `Val::Ratio(i64, u64)` (numerator + non-zero
  denom). Schemey, exact, no float weirdness. Roughly 80 LOC for
  arithmetic dispatch (`prim.rs` add/sub/mul/div/cmp need to
  promote between `Num` and `Ratio`). Parser learns `1/20` syntax.
- **Floats** — `Val::Flt(f64)`. Pragmatic, expected behavior, ~40
  LOC. Parser learns `0.05` syntax. Loses exactness and brings
  the usual float comparison hazards.

**Recommendation.** Rationals fit the project's tone (exact CEK
semantics, deterministic everything, zero-dep). The cost
difference is small. Float syntax could be added later via a
`#i` reader (Scheme `(exact->inexact)` style), but probably
unnecessary for the kinds of demos this lisp hosts.

**Files to touch.**
- `crates/lisp/src/val.rs` — new `Val` variant, `is_truthy`,
  `Display`, `eq_shallow` updates.
- `crates/lisp/src/prim.rs` — arithmetic and comparison prims
  get rational-aware dispatch.
- `crates/lisp/src/parse.rs` — tokenizer reads `1/20` or `0.05`.
- `crates/lisp/tests/eval.rs` — arithmetic tests.

**Open design questions.**
- Do `=`, `<`, `>` cross-type-compare `Num` and `Ratio`? (Should:
  `(= 1 2/2)` should be `#t`.)
- Does division of two `Num`s produce a `Ratio` automatically?
  (`(/ 1 4)` → `1/4`?) — pretty Schemey. Currently `/` is
  integer-only and rounds.
- Does `(quotient n m)` / `(remainder n m)` stay integer-only?
  (Yes — those are explicit integer ops.)

**Scope estimate.** ~100 LOC if rationals, ~50 if floats. Pure
addition; existing tests should stay green.

---

## Medium-impact (val / parse ergonomics)

### 3. `Val` constructor helpers

**Problem.** Demos and Rust prims build `Val` cons-structures
repeatedly, always with the same pattern:

```rust
Val::Cons(Rc::new(k), Rc::new(v))
```

Genes added two ad-hoc helpers (`to_pair_list`,
`traits_to_genome_ctx`) for "list of cons-pairs" shapes that the
existing `Val::list_from` (`val.rs:62`) doesn't cover.

**Approach.** Pure additions to `val.rs`:

```rust
impl Val {
    pub fn cons(head: Val, tail: Val) -> Val {
        Val::Cons(Rc::new(head), Rc::new(tail))
    }

    pub fn pair(k: Val, v: Val) -> Val {
        Val::Cons(Rc::new(k), Rc::new(v))
    }

    pub fn alist_from(pairs: &[(Val, Val)]) -> Val {
        let mut acc = Val::Nil;
        for (k, v) in pairs.iter().rev() {
            acc = Val::cons(Val::pair(k.clone(), v.clone()), acc);
        }
        acc
    }
}
```

Then `genes::to_pair_list` and `traits_to_genome_ctx` collapse to
one-line calls. `world_prim::world_size` (`world_prim.rs:46`)
stops writing `Val::Cons(Rc::new(...), Rc::new(...))` long-hand.

**Files to touch.**
- `crates/lisp/src/val.rs` — three new helpers.
- `crates/lisp/src/genes.rs` — drop `to_pair_list`,
  `traits_to_genome_ctx`; use the new helpers.
- `crates/lisp/src/world_prim.rs` — clean up `world_size` and any
  other manual cons-building.

**Scope estimate.** ~20 LOC additions, ~30 LOC deletions. Five
minutes of work; improves readability everywhere val-construction
happens.

---

### 4. Shared `assoc-get` prim

**Problem.** `assoc-get` exists twice:

- In Rust: `world_prim::assoc_get` (`world_prim.rs:88`) — used
  by `world-apply!` to read `element`, `tx`, `ty`, `area` from
  the spell ctx.
- In lisp: `genes::PRELUDE_BINDINGS` defines its own `assoc-get`
  closure (used by `add-allele`).

Both walk an alist by key. Identical semantics.

**Approach.** Add a pure prim `assoc-get` to `prim::initial_env`
(`crates/lisp/src/prim.rs`) so both demos can use it without
defining their own. The genes prelude drops its `assoc-get`
binding; the world resolver keeps its Rust helper (it's calling
from Rust, can't easily call into the lisp).

Actually, the cleaner move is to **leave the Rust helper alone**
(it's an internal implementation detail of the world resolver)
and just **add the lisp-callable prim** so future preludes don't
each redefine it.

**Files to touch.**
- `crates/lisp/src/prim.rs` — add `assoc-get` as a pure prim.
- `crates/lisp/src/genes.rs` — drop the `(assoc-get …)` binding
  from `PRELUDE_BINDINGS`.

**Scope estimate.** ~15 LOC. Trivial. Touches the existing test
in `crates/lisp/tests/express.rs` only insofar as the prelude
shrinks (no behavioral change).

---

### 5. Symmetric prim registration

**Problem.** I added `Vm::register_prim` (ADR-011) for pure
prims, but `Val::WorldPrim` has no public registration path —
it's only installed by `Vm::with_world` via the `WORLD_PRIMS`
const slice. If a future demo wanted to add a world-aware prim
from an example without modifying the lisp crate, it couldn't.

**Approach.** Add the obvious sibling:

```rust
impl Vm {
    pub fn register_world_prim(
        &mut self,
        name: &'static str,
        arity: Arity,
        f: fn(&[Val], &mut World) -> Result<Val, String>,
    ) {
        self.env = self.env.extend(
            name.into(),
            Val::WorldPrim { name, arity, f },
        );
    }
}
```

Same shape as `register_prim` (`lib.rs:79`). No behavioral
change for anyone who doesn't call it.

**Files to touch.**
- `crates/lisp/src/lib.rs` — one method, ~10 LOC.

**Scope estimate.** Trivial. Just admits an asymmetry that
already exists.

---

## Validated patterns (no work; worth knowing)

These came up during demo work and confirmed the engine's design
is load-bearing in real usage, not just in tests:

- **Tail calls.** Both `thread` (spells) and the gene prelude's
  thread are tail-recursive lisp closures. They worked unbounded
  for any tape length without growing the Rust stack. The
  `tail_calls_dont_grow_the_stack` test isn't theoretical — the
  demos lean on it constantly.
- **Lexical scope in `Val::Clo`.** ADR-012's seed-via-lexical-
  scope pattern (`(let ((seed N)) ...prelude with mutate
  referring to seed...)`) only works because `Val::Clo { env,
  ... }` captures the env at closure creation. This is the
  substrate underneath the seeded mutation design.
- **Zero engine changes absorb structurally different DSLs.** The
  hypothesis from ADR-011 held: spells (last-write-wins flat
  alist) and genes (list-per-key diploid accumulation with
  Mendelian resolution) are very different vocabularies, and the
  engine learned nothing for either. Only addition across the
  whole session was `Vm::register_prim` (15 lines) — and even
  that just exposes what `with_world` already does internally.

These don't suggest work; they suggest the design's good and we
shouldn't accidentally break them.

---

## Out of scope here (already on the docs/letrs.html coda)

These are real but pre-existing follow-ups, not surfaced by the
gene work. Listed for context so we don't duplicate them:

- **Hygienic macros, or not.** The genes demo didn't use macros,
  but a future demo that wants `(defspell …)` or `(defgene …)`
  will trip on the unhygienic procedural macro system.
- **Persistent maps (`Val::Map`).** Both demos use alist ctxs,
  which is fine at <30 keys. A real game would want better
  scaling.
- **Structured errors.** `eval_str` returns `Result<Val,
  String>`. The web REPL and gene/spell labs surface those
  strings raw. For a real REPL, line/column info would be nice.
- **The play loop.** Listed in the docs/letrs.html "what comes
  after" — turn-based render/input/spell/world tick. The engine
  is ready; this is host-side work.

---

## Suggested order

If the next session does ~one of these:

1. **Installable preludes + top-level `define`** (#1) — biggest
   win, most architectural reach. Unlocks "load a `.scm`
   prelude file" as a real concept. Touches `Vm`, `eval_str`,
   `parse.rs`. ~100 LOC + reshape both prelude strings.
2. **Numeric tower** (#2) — clears a real expressiveness wall.
   Rationals if we care about exactness; floats if we just want
   the syntax. Open design call.
3. **`Val` helpers + shared `assoc-get` + `register_world_prim`**
   (#3 + #4 + #5) — bundle the three small ones as a single
   cleanup commit. Maybe 90 minutes total, no design
   controversy.

`#1` is the most interesting; `#3+#4+#5` is the easiest warm-up.
