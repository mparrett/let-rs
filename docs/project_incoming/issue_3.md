# issue 3 — snapshot world for transactional spell evaluation

**Source:** codex review 2026-05-26, finding #6b (the underlying
capability; the narrative claim was rephrased in
`docs/letrs.html` already).

## Problem

`WorldPrim` primitives mutate the host `World` immediately. A spell
pipeline that runs `(world-apply! ...)` and then later errors has
already committed its tile changes — no rollback. The narrative had
said "the world rolls back for free"; it now honestly says snapshots
are not built.

There are real use cases that *want* this — the most obvious is
precognition: "show me where this spell would land if I cast it
here," without actually casting. Tentative evaluation against a
cloned world, render, discard.

## Suggested directions

1. **Clone-and-replace (simple).** `World: Clone`. Wrap a tentative
   `eval_str` in a "save world; run; on error or on caller request,
   restore." A new `Vm::eval_str_tentative(src)` returns the result
   *and* leaves the world untouched. Cheap because `World` is small
   (a few KB for our grid sizes).
2. **Copy-on-write tiles.** Tiles are `Vec<Tile>`; an `Rc<[Tile]>`
   with mutation triggering a clone gets you snapshot semantics
   without the up-front copy. More complex but better for larger
   worlds.
3. **Transactional layer.** A `TileOverlay` that records writes
   without applying them, plus a `commit()` / `discard()`. Pure
   functional model; aligns with the "ctx is pure, host applies"
   narrative. Most invasive.

Recommended: start with (1) — it's small and unblocks precognition
demos. (2) and (3) are optimizations / generalizations.

## Out of scope

- Snapshotting macros / env (`eval_str` already rolls those back on
  error; see commit `47560d0`).
- General undo/redo. This is a single tentative-eval primitive, not a
  history mechanism.

## Where to start

- `crates/lisp/src/world.rs::World` — derive `Clone` (Tile is Copy,
  so straightforward).
- `crates/lisp/src/lib.rs::Vm` — add `eval_str_tentative`.
- `crates/wasm/src/lib.rs::WasmVm` — expose as `eval_tentative` /
  `cast_tentative` for the lab page if useful.
- Update `docs/letrs.html` once shipped — the rephrased paragraph at
  line ~764 should swing back to "rolls back for free" with a link
  to this issue's resolution.
