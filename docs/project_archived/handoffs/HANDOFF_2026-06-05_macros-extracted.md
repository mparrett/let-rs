# Session Handoff

**Created:** 2026-06-05T22:30:00-0700
**Session ID:** 81f755b6-61fd-470b-962b-ba9f9ca8b4f8
**Working Directory:** /Users/matt/projects-new/3p/letrs

## What to read first

The CESK migration (ADR-023) and the macros-to-sibling-crate
extraction (ADR-024) both landed this session — two real
architectural moves in a row. The user explicitly chose **not**
to write a dev-log interlude or Act VIII yet; they want the next
session to build toward something concrete enough to make an
interlude worth writing. Read this handoff before reaching for
the dev log or proposing more abstract architectural changes.

## Summary

Long session anchored by two architectural moves: ADR-023 (CESK
store, design + implementation, dissolves the letrec leak ADR-021
pinned) and ADR-024 (macros lifted to a sibling crate so the
engine reverts to macro-unaware, with an opt-in `macros::MacroVm`
wrapper). Plus: dev-log Act VII closing the CEK chapter, an
accordion UI for the dev log with leading caret + flush-left
controls + collapsibles for prelude/coda, a lab-nav strip in the
masthead, the per-machine dev-port scheme pinned (let-rs = 7670),
and the first stdlib payoff via `install_stdlib` shipping `begin`
+ tier-1 (`when`/`unless`/`and`/`or`). 140 tests pass.

## Current State

Branch `main`, clean worktree, pushed through `2f45a94`. Pre-
session baseline was the prior handoff's `9ff810a docs: handoff
2026-05-31` (today's tip is 15 commits ahead of that).

Today's commits, oldest first:

```
6484a1b  docs: archive HANDOFF_2026-05-31
af1d593  docs: ADR-023 draft — CESK migration, designed, deferred
b6673d3  lisp: implement ADR-023 — CESK store dissolves letrec cycle
fadbc09  docs: ADR-023 postscript; mark ADR-021 fix done via CESK
8a97ec9  web: dev-log Act VII — CEK becomes CESK
a4ea9d0  web: collapsible acts in let-rs dev log
c061d68  web: accordion polish — leading caret, flush-left controls
3cb1ecb  web: hang accordion chevron in negative-margin space
7986f6b  web: prelude and coda are collapsible too
faaa69c  web: lab nav strip in let-rs masthead
80cd6fc  ops: pin let-rs dev port to 7670; document port scheme
dfa0924  macros: extract to sibling crate (ADR-024)
4a4ae2e  docs: ADR-024 + CLAUDE.md + key_facts for macros extraction
9b0991b  macros: install_stdlib + begin; wire WASM cast_curve to use it
2f45a94  macros: tier-1 stdlib — when, unless, and, or
```

## Uncommitted State / Untouched

- *Uncommitted:* none. Worktree clean.
- *Untouched (deliberate):*
  - **Spells / genes / curves preludes** still target raw `lisp::Vm`,
    not `MacroVm` (`crates/spells/src/lib.rs` etc.). They use plain
    `define`s — no macros. This is the natural extension surface for
    the next session (see Next Steps).
  - **ADR-022 (structured parse errors)** still a design draft, not
    implemented. User explicitly paused on it 2026-05-31; still on
    the shelf.
  - **CLI examples** (`spells.rs`, `world.rs`, `genes.rs`, `curves.rs`)
    stay on raw `lisp::Vm`. Only the user-facing REPLs (`examples/
    repl.rs`, `crates/wasm/src/lib.rs`) wrap in `MacroVm`.
  - **Dev-port server (PID 87065 at session end)** was left running
    on port 7670 via `just wasm-serve`. May or may not still be up
    when the next session starts; `lsof -iTCP:7670 -sTCP:LISTEN`
    checks.
  - **Local repo dir** still `/Users/matt/projects-new/3p/letrs`.
    User said they'd rename to `let-rs` themselves; don't auto-rename.

## In Progress

Nothing strictly mid-implementation. The macros stdlib is sitting
ready for a concrete consumer — see Next Steps for what would
make it earn a writeup.

## Gotchas

- **Don't write a dev-log Act VIII or interlude unprompted.** The
  user explicitly said "let's hold on the interlude and instead
  write a handoff." They want momentum + a concrete consumer
  *first*, then the writeup. If they ask "is it time yet?", the
  honest answer is: when a real DSL adopts the macros stdlib
  end-to-end OR when a hygiene bug surfaces and is fixed, the
  interlude has content. Until then, the macros work is mostly
  potential energy from a narrative perspective.
- **The macros stdlib is opt-in.** `MacroVm::new()` does NOT
  install the stdlib; `MacroVm::with_stdlib()` does, or call
  `macros::install_stdlib(&mut vm)` explicitly. There's a pinning
  test (`stdlib_not_present_without_install`) that asserts this.
  Don't change the default — the opt-in discipline is part of
  ADR-024's "minimal engine" stance.
- **`or` macro has a known hygiene caveat.** It binds to
  `__or-val__` to avoid double-evaluating side-effecting args.
  If user code binds `__or-val__` and references it inside a
  later `or` arg, you'd get a surprising shadow. Documented in
  the STDLIB doc-comment. Fix when/if needed is gensym → its own
  ADR.
- **Spells/genes/curves preludes target `lisp::Vm`, not `MacroVm`.**
  If you write a `defspell` macro that the spells prelude wants to
  use, the spells crate's install API needs to change OR you
  install the stdlib + the spell macros at the host level (WASM
  bridge + examples/spells.rs), keeping spells::install pure
  defines. Either choice has consequences — flag for the user.
- **Dev port is 7670 across the project now.** Hardcoded in
  `justfile:65`, documented in `docs/project_notes/key_facts.md`'s
  URLs/ports section with the per-machine scheme (`6900 +
  first-two-letters-as-base-36`). Tailscale URL pattern: `http://
  100.126.31.103:7670/<page>.html`.
- **CESK leak fix is real.** The `letrec_cycle_persists_after_drop`
  diagnostic was renamed `letrec_does_not_leak` and *flipped*; if
  anything later re-introduces the leak, this test fires loudly.
- **Accordion state persists in localStorage** under key
  `let-rs.collapsible-state.v2`. If the dev log gains a new section
  in a way that shifts numeric indices, bump to v3 so returning
  readers don't see stale state.
- **`begin` came in via macro, not engine special form.** ADR-019
  explicitly rejected the engine-side path; the macro path
  honored that. If anyone proposes adding `begin` to `parse.rs`
  as a special form, push back — the strike-through note on
  ADR-019's Deferred section explains why.
- **The dev log's "Five rules" naming is now slightly aspirational.**
  Post-CESK there are still five CEK *transitions* but the engine
  has a fourth register (the store). Act VII addresses this
  honestly; don't "fix" the Act I "Five rules" title — it's
  historical and correct in its chapter.

## Next Steps

User goal: build toward something interlude-worthy. The interlude
needs concrete proof that ADR-024's extraction was the right call.
Options, ranked by how close they are to "interlude content":

1. **`defspell` macro for the spells crate.** The most direct
   payoff. Today `crates/spells/src/lib.rs:21-32` is a manual
   wall of `(define fire (lambda (ctx) (assoc-set 'element
   'fire ctx)))` repetition. A `(defspell fire (element fire))`
   macro could collapse those nine defines into nine one-liners
   and prove the macros stdlib pulls its weight in production
   code, not just at the REPL. Requires deciding where the
   `defspell` macro lives (probably a new `crates/spells/`
   stdlib install function that calls into `macros::MacroVm`),
   and re-pointing the spells consumers (wasm bridge,
   examples/spells.rs) at MacroVm. **1-3 hours, lots of design
   space to choose.** Once this lands, the interlude has a clean
   narrative: audit → extract → adopt in a real DSL → write.
2. **`defcodon` / `defgene` analogues for genes.** Same shape.
   Less urgent than spells because the gene prelude is more
   varied (mutate/breed/express prims), but if `defspell` works,
   `defcodon` is mostly mechanical.
3. **A second stdlib tier — `case` / `cond` / `if-let`.** Pure
   sugar; not as interlude-worthy as a DSL adoption.
4. **Hygienic macros (gensym).** Would only be worth doing if a
   real bug from the `__or-val__` style hits us. Probably
   premature.
5. **`issue_4` (REPL + labs share Vm).** Listed in last handoff,
   still relevant, still P3. 1-3 hours. Not interlude-worthy
   on its own.

**Recommended path for the next session:** open with "let's wire
spells to MacroVm and write `defspell`." Spend an hour on the
implementation, an hour on tests + the WASM bridge update, then
draft the interlude with the concrete diff in hand. That's the
shortest path to a satisfying writeup.

If the user wants something smaller, **issue_4** or **the
`->` thread-first macro added to STDLIB** are both quick wins
that don't pull the interlude forward but keep momentum.

## Open Tickets

`docs/project_incoming/` holds two tickets, neither
`status: in_progress`:

- `issue_3.md` — deferred (world snapshots; waiting on a concrete
  consumer)
- `issue_4.md` — open P3 (REPL+labs share VM clobber risk)

Plus deferred designs on disk:
- ADR-022 — structured parse errors with source spans (drafted,
  not implemented; user explicitly paused on it)
