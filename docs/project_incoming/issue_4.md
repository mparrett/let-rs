# issue 4 — shared VM across REPL + labs

**Source:** codex review 2026-05-26, finding #7. Priority: P3 — UX,
not correctness.

## Problem

`web/common.js` exports one `Vm` shared by both the REPL panel and
the page-specific lab (Spell Lab on `spells.html`, Gene Lab on
`genes.html`). The REPL can redefine `thread`, `fire`, `assoc-set`,
etc. — and because they're regular lisp bindings, the labs use
whatever's in scope at cast time. So a curious typer in the REPL can
silently change what the lab buttons do.

This is fine — even kind of fun — for an expert playground. For a
demo on a public link, it's surprising and irrecoverable without a
page reload.

## Suggested directions

1. **Reset button (smallest).** A "reset VM" button in the lab UI
   that does `new Vm(width, height)` and reinstalls the preludes.
   No code change to the bridge — the constructor already does the
   work.
2. **Separate VMs for REPL and lab.** Two `Vm` instances in JS, one
   per panel. Slightly more memory, but matches user mental model.
3. **Read-only REPL mode toggle.** Lab keeps the canonical VM; REPL
   gets a checkbox to mark its evaluations sandboxed (eval against
   a forked Vm, discard after).

Recommended: ship (1) first — one button, no architecture change.
(2)/(3) are larger calls that can wait until someone reports the
breakage.

## Out of scope

- A full sandbox model (capabilities, restricted prims). Far beyond
  the scope of a demo; the lab is meant to be played with.
- Persistence of REPL history across reloads.

## Where to start

- `web/common.js` — exports `vm`, the shared instance.
- `web/spells.html` + `web/genes.html` — add a reset button.
- `web/spells.js` + `web/genes.js` — wire the button to
  `vm.reset_world()` (already exists) plus a fresh `new Vm(...)` if
  we want to wipe defines/macros too. Note `reset_world` only resets
  tiles, not env — for a full reset, replace the `vm` reference.
