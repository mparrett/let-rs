# Host state in letrs — what "world" actually is

Reading guide for anyone (new contributor, future Claude session,
archaeologist) trying to make sense of why the lisp engine has a
fixed `World` type baked in, and what the path forward looks like.

This isn't an ADR — no decision is being recorded. It's the design
rationale behind the current shape, captured from a conversation on
2026-05-26 so the reasoning doesn't have to be re-derived from
scratch next time someone asks "what's World, exactly?"

## What World is, concretely

`crates/lisp/src/world.rs` defines `World`: a `width × height` tile
grid (`Floor | Wall | Fire | Ice`) and an append-only `log:
Vec<String>`. ~100 LOC of struct + helpers.
`crates/lisp/src/world_prim.rs` adds five lisp-callable prims that
read or mutate it: `world-tile`, `world-set-tile!`, `world-log!`,
`world-size`, `world-apply!`.

That's the whole "world." The name is aspirational — it sounds
general — but the implementation is specifically "a paintable tile
grid with an event log," built around what the spell demo needs.
`world-apply!` is the giveaway: it's a resolver that reads
`'element`, `'tx`, `'ty`, `'area` out of a spell ctx and paints a
square of fire/ice tiles around `(tx, ty)`. That's not a general
primitive; it's the spell DSL's punchline.

## How state lives in letrs

The lisp itself is purely functional. No `set!`, no mutable
variables. Lists, closures, recursion, all over immutable `Val`s.
The only way any state change happens at all is through prims —
Rust functions exposed as lisp-callable values. There are two
flavors (ADR-005):

- **Pure prims** (`Val::Prim`). Rust functions that take `Val`s in
  and return `Val`s out. The "non-lispy" part is just implementation
  detail; conceptually they're functions on values. Arithmetic, list
  ops, predicates, `eq?`, `assoc-get`. Genes' `express!`, `mutate!`,
  `breed!` are also pure prims — they compute on `Val`s without ever
  touching persistent Rust state. Pure prims are already
  host-agnostic.

- **Host-state prims** (`Val::WorldPrim`). Rust functions that read
  or write persistent Rust state hidden in the Vm. State lives
  between calls, in a typed Rust struct the lisp can never see
  directly. Today the typed struct is fixed: `World`.

The two-flavor split is the seam between "language" and
"demo-specific." Pure prims are general; host-state prims are where
a specific host gets a foothold.

## Why World isn't generic

The natural follow-up question: why didn't we write `World` as
something like `Grid<T> + EventLog` from the start, so a roguelike
could use `Grid<Cell>`, Conway's Life `Grid<bool>`, a spreadsheet
`Grid<Formula>`? `EventLog` is even more reusable — any host
wanting observable side effects could use it.

Two reasons.

**The project follows a "promote on second consumer" rule**
(formalized in ADR-010, ADR-011). When the spell demo was the only
consumer of host state, there was no data on what the right generic
shape was. Building `Grid<T>` off one user would be guessing at the
next user's needs. Write the concrete thing first; let the next
consumer reveal the abstraction.

**Then genes came along and taught the opposite lesson.** The
second consumer of `Vm` didn't want a differently-shaped grid — it
wanted *no host state at all*. Pure prims over `Val`s were enough;
the engine still carries a 0×0 `World::empty()` because the type
requires one, but the genes prims and prelude never touch it. That
data point shifted the proposed direction from "generalize World"
(still bakes host state into the engine, just with type parameters)
to "remove host state from the engine entirely."

## What a non-game would use World for

Nothing. A non-game letrs use would do one of three things today:

1. Ignore it (the way genes does — accept a dummy 0×0 grid sitting
   in memory).
2. Force-fit its state into the `Tile` enum + grid shape (only
   works for tile-grid demos).
3. Fork the lisp crate.

Kinds of hosts that would want different state: a config
interpreter (host state = the config object being built), a
build-script DSL (host state = the build context), a music
sequencer (host state = the playing sequence), an embedded REPL
(host state = whatever the outer app is). None want a tile grid;
all want their own typed state, with their own prims.

## What the endgame could look like

Not decided. But the two generalizations on the table are
complementary, not competing:

1. **Make the Vm host-agnostic.** Drop `Val::WorldPrim`. Add
   closure-capable prims (`Rc<dyn Fn(&[Val]) -> Result<Val,
   String>>`) so a host registers state-aware prims as closures
   that captured a handle at registration time. The Vm doesn't know
   any specific host type exists. The host owns its state; the lisp
   talks to it through closures. ~150 LOC of churn, no behavior
   change. Subsumes the `Vm::register_world_prim` helper added in
   the 2026-05-25 cleanup commit.

2. **Add a `crates/world/` micro-crate** with `Grid<T> + EventLog`
   as a reusable building block. Sibling to `runes/` and `codons/`.
   The spell demo would import it; the genes demo wouldn't; a
   roguelike demo would. The lisp engine has zero awareness it
   exists.

Doing both is the natural endgame. The first makes the engine
host-agnostic; the second offers a generic building block for
hosts that happen to need a grid. Neither forces the other.
Together: no host state baked into the engine, with opt-in
building blocks for hosts whose state shape lines up with common
patterns.

## TL;DR

- "World" is the spell demo's paintable tile grid + event log, not
  a general concept.
- The lisp is pure-functional; prims are the only bridge to Rust
  state.
- Two prim flavors today: pure (host-agnostic) and host-state
  (hardcoded to `World`).
- We didn't generalize `World` because we only had one consumer;
  the second consumer (genes) revealed that "no host state in the
  engine" is cleaner than "generic host state in the engine."
- Future direction: drop `WorldPrim`, let hosts register
  closure-captured prims; optionally ship `Grid<T> + EventLog` as a
  sibling building block.

## Resolution (2026-05-29)

Both bullets in §What the endgame could look like are now closed:

- **ADR-017** dropped `Val::WorldPrim`. `Val::Prim` is the single host-prim
  variant; it carries an `Rc<dyn Fn(&[Val]) -> Result<Val, String>>` that
  may capture any host handle. The engine has no privileged host type.
- **ADR-018** moved `world.rs` + `world_prim.rs` into `crates/world/` as
  a sibling micro-crate. The grid is still concrete `World` (not yet
  `Grid<T>`), but it's an opt-in building block now — `lisp` ships zero
  host types. The "generalize to `Grid<T>`" follow-up is deferred until a
  second grid-shaped host appears (still "promote on second consumer").

