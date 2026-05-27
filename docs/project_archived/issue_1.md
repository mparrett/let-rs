# issue 1 — execution step budget (and/or worker)

**Status:** resolved 2026-05-27 by commit `78291ec` (step budget only;
worker option deferred). Archived from `project_incoming/`.

**Source:** codex review 2026-05-26, finding #1c. Priority: P1 deferred from
the correctness batch (handled #1a/b but not #1c).

## Problem

`crates/lisp/src/step.rs::run` has no fuel / cancellation. A
nonterminating lisp expression (e.g. `(letrec ((f (lambda () (f)))) (f))`)
runs forever. In the browser this is worse than annoying: `web/common.js`
evaluates on the main thread, so the page locks.

`just spells` / `just genes` are also vulnerable but those are CLI demos
the user can Ctrl-C.

## Suggested directions

Two cuts, increasing in cost.

1. **Step budget (small).** Thread a counter through `step()` / `run()`.
   On each iteration, decrement; at zero, return
   `Err("execution exceeded step budget")`. Budget is a `Vm` field with
   a per-call override on `eval_str`. Tests would assert the bound
   triggers on a known infinite loop without flagging the existing
   100k-step `tail_calls_dont_grow_the_stack` test. Default budget
   high enough for current tests; surfaced as a `Vm` setter for hosts.

2. **Worker (larger).** Move WASM eval off the main thread. Bigger
   change — needs a JS-side worker bootstrap and message channel — but
   gives real cancellation, not just a budget. Worth keeping the step
   budget regardless, since the worker can still hang on a tight
   non-yielding loop unless we periodically check a flag.

Recommended sequence: ship the step budget first (one PR, well
contained); evaluate whether the worker is necessary based on what
real cast tapes can produce.

## Out of scope

- Async / cooperative yielding inside `step()`. The CEK shape would
  support it (state is reified) but it's a bigger redesign.

## Where to start

- `crates/lisp/src/step.rs:run` — the loop.
- `crates/lisp/src/lib.rs::Vm` — add `step_budget: u64` field.
- `crates/lisp/tests/eval.rs` — add a divergent expression that should
  hit the budget.
- `crates/wasm/src/lib.rs` — pass budget through; surface to JS if
  useful for the lab pages.
