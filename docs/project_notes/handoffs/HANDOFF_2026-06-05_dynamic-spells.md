# Session Handoff

**Created:** 2026-06-05T17:11:57-0700
**Session ID:** 271e804c-83ef-49c6-86b2-af51cba41b32
**Working Directory:** /Users/matt/projects-new/3p/letrs

## What to read first

This was the *second* session today. The morning session (archived
as `HANDOFF_2026-06-05_macros-extracted`) shipped ADR-025 (defspell)
and dev-log Act VIII. This session ran the dynamic-spells arc end-
to-end (ADR-026 through ADR-029) plus a course-correction the user
flagged mid-stream. The lab is now genuinely temporal. **Do not
draft Act IX yet** — same posture as the prior handoff: build
something concrete enough to be worth narrating before the writeup.

## Summary

Eight commits implementing a four-stage "make the spells demo
temporal" arc plus one stretch + reframe. The engine grew `set!`
(ADR-026, ~60 lines), the world grew tile decay + `world-tick!`
(ADR-027), the spells prelude grew a caster-side mana model with
cost-gated `cast!` and regen-via-`tick!` (ADR-028), and the spell
lab UI rewired to drop its JS dissipation animation in favor of
real engine-driven decay with a pip-row mana meter (stage 4). A
"stretch" added the ᛃ rune with a duration meaning; the user
caught the implicit power→lifetime coupling and pushed back, so
ᛃ got reframed as `aftershock` — scheduled re-cast via a new
World `pending: Vec<PendingCast>` (ADR-029). Verified end-to-end
with Playwright. 184 workspace tests pass; clean working tree.

## Current State

Branch `main`, clean, 15 commits ahead of origin (none pushed).

Today's session commits, oldest first (the morning session also
added commits before these; see "Background" below):

```
a962074  lisp: set! — first-class mutation (ADR-026)
818584e  docs: ADR-026 + CLAUDE.md for set!
3701550  world: tile decay + world-tick! (ADR-027)
55a9d59  docs: ADR-027 + CLAUDE.md + key_facts for tile decay
7f57bf8  spells: mana model — cast!/tick!/reset-mana! (ADR-028)
95f668c  docs: ADR-028 + CLAUDE.md for mana model
35cdb97  web: spell lab tick loop + mana meter (stage 4)
c063e62  runes/spells/world: ᛃ JERA — explicit duration knob
c2bb7ef  runes/spells/world: ᛃ JERA reframed as aftershock (ADR-029)
cbfbcf2  web: drop seed cast; fix aftershock cheatsheet examples
```

**Background commits this morning** (already archived in the
prior handoff):

```
9d1a835  macros: expand_top_level — allow define at top level
be4475d  spells: defspell/defparam macros in prelude (ADR-025)
cac9b53  docs: ADR-025 + CLAUDE.md + key_facts for defspell adoption
d4a11a2  web: dev-log Act VIII — the vocabulary becomes a library
```

## Uncommitted State / Untouched

- *Uncommitted:* none. Worktree clean.
- *Untouched (deliberate):*
  - **Dev log Act IX** intentionally not written. Same reasoning
    as the prior handoff — let the narrative chase the work.
    Candidate beat: "the substrate stopped being stateless" arc
    covering ADR-026/027/028/029 together. The duration→aftershock
    course-correction would be an interesting honest-engineering
    detail to include.
  - **CLI examples** (`spells.rs`, `world.rs`, `genes.rs`,
    `curves.rs`) bypass the new mana model and the aftershock
    path. They call `world-apply!` directly, not `(cast! …)`,
    deliberately — they're testing the world prim and the
    pipeline, not the DSL flow. Don't "fix" this unless asked.
  - **`pending_count()`** is exposed on World but no UI surfaces
    it. ADR-029 notes a deferred "3 aftershocks queued" indicator
    in the lab — the data is ready when the UI pulls.
  - **Dev server** (PID 42259) still running on port 7670 via
    plain `python3 -m http.server -d web` (not `just wasm-serve`
    — it was started from this session's bash). `lsof
    -iTCP:7670 -sTCP:LISTEN` confirms.
  - **REPL pages for genes/curves**: untouched. The mana model
    is a *spells* thing — not bolted onto genes or curves. If a
    future arc adds caster-side resources to those DSLs they'd
    follow the same pattern (prelude-level globals + set! +
    tick wrappers).
  - **ADR-022 (structured parse errors)** still parked. The
    next "real UX win" candidate after the dynamic arc.

## In Progress

Nothing strictly mid-implementation. The arc closed at
ADR-029 + ADR-028 postscripts + Playwright verification. The
mana meter is wired, decay fires, aftershocks work end-to-end,
the cheatsheet examples make the mechanic visible. Cleanly at
rest.

## Gotchas

- **The duration→aftershock course-correction is the load-
  bearing story.** I added ᛃ as `duration` in `c063e62` after
  noting "power was double-duty" — but the user caught that the
  implicit `power → lifetime` coupling was *intent*, not
  accident. `c2bb7ef` reverts the split (power reclaims
  lifetime + cost) and gives ᛃ the genuinely new job of
  scheduled re-casts. Both commits are in the log; the second
  ADR-029 narrates the whole sequence honestly. **Do not
  re-litigate**: power means "how long does this linger,"
  duration was a wrong turn, aftershock is the real new thing.
- **Aftershock visibility requires power ≤ aftershock.** The
  default lifetime is 5; if a user casts `ᚦ ᛃ 3` (no explicit
  power), the aftershock fires *while the tile is still
  painted* and just refreshes its lifetime — visually no
  change, only the log shows it. The cheatsheet now leads with
  `ᚦ ᛟ 1 ᛃ 3` to make the re-strike obvious. If a future user
  reports "aftershock not working," check the lifetime arithmetic
  before assuming a bug.
- **`cast!` ≠ `world-apply!`.** The WASM bridge routes through
  `cast!` (mana-gated). The CLI examples and `crates/lisp/
  tests/world.rs` call `world-apply!` directly — they bypass
  mana intentionally to test the world prim in isolation. If
  you add a new bridge surface, route it through `cast!`; if
  you add a world-only test, raw `world-apply!` is correct.
- **PendingCast chains are bounded by construction.** A
  PendingCast does NOT carry an aftershock field of its own, so
  `(ᚦ ᛃ 1)` cannot loop forever. Three lisp-level tests pin
  this (`aftershock_does_not_recursively_schedule` and friends).
  If anyone proposes "chained aftershocks for stacking effects,"
  this is the structural invariant being relaxed; needs an ADR.
- **Aftershocks paint with the original lifetime, NOT a
  refresh.** A `(ᚦ ᛟ 1 ᛃ 5)` cast paints fire(life=1) at t=0,
  then fire(life=1) at t=5. The re-strike inherits the same
  short lifetime; it doesn't carry "fresh" decay. Tests pin
  this; UI examples are written assuming it.
- **Mana cost is paid ONCE at cast time.** The aftershock
  doesn't charge again when it fires. The cost formula
  (`1 + power + area + aftershock`) absorbs the future strike
  up front. This is what makes aftershock a real tactical
  choice rather than a free time-extension.
- **`set!` cost was "60 lines" not the rhetorical "5".** Act
  VII's coda claim was about the *substrate* being ready
  (CESK store), not the form. Real `set!` shipped at ~60 lines
  across `expr.rs`, `k.rs`, `parse.rs`, `step.rs`, `env.rs`,
  plus the macros-expander hook so the name slot isn't macro-
  expanded.
- **The expander hook for set! is structural.** `Expander::
  expand_all` learned to skip `items[1]` (the name) of
  `(set! NAME val)` and expand `items[2]` normally. Same
  pattern as how `lambda` doesn't expand its params. One
  macros test (`set_bang_name_position_not_macro_expanded`)
  pins this.
- **The /tmp/spell_lab_test.py Playwright verification is
  NOT in the repo.** It was a one-shot sanity check; I didn't
  add playwright to the project tooling. If we want this as a
  regression test, it needs justfile + dev-deps + likely a
  conftest.py or similar. Worth doing if/when the lab grows.
- **`reset_world` clears pendings.** `World::new(w, h)`
  constructs a fresh World with empty `pending`, and the
  bridge's `reset_world` calls that. So pendings don't
  survive a reset. ADR-029 notes this as a "minus" because a
  future "reset tiles but not pending events" mode would be
  confusing — flagged for future contributors.
- **Browser caching can hide HTML/JS changes.** Standard
  let-rs issue. If the lab looks stale after a deploy,
  hard-refresh (⌘+Shift+R) before chasing a layout bug.

## Next Steps

The dynamic-spells arc is complete. Plausible next moves,
ordered by what would push the project forward most:

1. **Dev-log Act IX** — covers ADR-026 → ADR-029 as one arc.
   The narrative beats are: "the substrate stopped being
   stateless" (set!), "the world grew time" (decay), "the
   caster grew a budget" (mana), "the substrate grew a notion
   of scheduled effects" (aftershock). The honest detour
   (duration → aftershock) is a satisfying mid-act
   complication that pays off. 1-2 hours; needs no code, just
   prose + structure. **Best next session if you want a
   chapter to come out of today's work.**
2. **A `pending` UI indicator** in the lab (per ADR-029's
   deferred list). `pending_count()` exposes the data; the
   pip-row pattern from the mana meter generalizes. A small
   counter "queued: 3" next to the mana meter would visually
   surface the in-flight aftershocks. 30-60 min.
3. **A `pulse` animation for fired aftershocks.** When an
   aftershock fires, briefly flash the cell with a contrast
   color so the re-strike is unmistakable even if the
   lifetime overlaps. Adds polish without changing the model.
   ~30 min.
4. **ADR-022 (structured parse errors with source spans).**
   Drafted but never implemented. The web REPL's error UX is
   still the weakest part of the labs. Concrete, scoped, and
   would visibly improve the demo. 2-4 hours.
5. **`defcodon` / `defgene` for the genes prelude.** The same
   defspell pattern applied to the second DSL. Mostly
   mechanical; closes the "is the macros stdlib load-bearing
   across DSLs?" question one more pack. 1-2 hours.
6. **`pulse` event in `world-tick!`'s log** (already
   partially done — aftershocks log `aftershock fire at
   (x,y) → N tiles`, tick logs `tick → N reverted`). The
   bridge surfaces these via `vm.log()`. Possibly add a
   "session log" panel separate from the world log if it
   grows noisy.
7. **`pending`-aware `tick!`** — currently the lisp-side
   `tick!` wrapper just calls `world-tick!` and regens 1
   mana. It could read `pending_count` and surface that to
   callers. Tiny; only if a host pulls.

**Recommended path for the next session:** open with Act IX.
The arc is clean and complete; the writeup has clear
boundaries (start at `a962074`, end at `cbfbcf2`); the
duration→aftershock detour is genuinely instructive
("sometimes the wrong design surfaces the right one"). After
Act IX lands, the natural follow-on is ADR-022 (structured
errors) or `defcodon` — both un-flashy but high-leverage.

If the user wants something tactile instead of narrative,
the `pending` UI indicator (option 2) is the smallest
satisfying improvement.

## Open Tickets

`docs/project_incoming/` holds two tickets, neither
`status: in_progress`:

- `issue_3.md` — deferred (world snapshots; waiting on a
  concrete consumer)
- `issue_4.md` — open P3 (REPL+labs share VM clobber risk)

Plus designed-but-unimplemented:
- **ADR-022** — structured parse errors with source spans
  (drafted, not implemented; user explicitly paused on it
  prior session)
