# issue 2 — Rc cycles in top-level defines and letrec

**Source:** codex review 2026-05-26, finding #4. Priority: P2 — slow
leak, not a correctness bug.

## Problem

A top-level `define` of a lambda forms a permanent `Rc` cycle:

```
env-frame slot → closure → captured env → frame → slot
```

Every function installed by `spells::PRELUDE_DEFINES` and
`genes::PRELUDE_DEFINES` allocates one such cycle on `install(vm)`.
`letrec` has the same shape per bound closure. This isn't a
correctness issue at our current scale (REPL sessions are short, the
Criterion harness rebuilds the Vm per iteration), but:

- Long-lived REPL / lab sessions in the browser leak prelude
  closures every reset that doesn't drop the whole Vm.
- Criterion runs that build a fresh `Vm` per iter currently leak ~all
  closures from each iteration — measurements aren't *wrong*, but
  they're noisier than they should be.

The current `env::Env` slot type is `Rc<RefCell<Val>>`. Closures
capture the env by `Rc<Frame>`. Both ends of the cycle are strong.

## Suggested directions

1. **`Weak` for the back-edge (closure → env).** Pros: minimal
   structural change. Cons: a `Weak::upgrade` failure at lookup time
   means the closure outlives its env, which is exactly what would
   happen if the surrounding env went out of scope. The semantics
   here need a design choice — should a closure called after its
   defining env has been dropped error, or do we guarantee envs
   outlive their closures by some other invariant?
2. **A small cycle collector.** Overkill for current scale, but
   technically correct.
3. **Drop the cycle by representing top-level defines differently.**
   Top-level bindings could live in a flat `Vm`-owned `HashMap`
   rather than as env frames; only nested `letrec` would need the
   placeholder pattern. This is the most invasive but eliminates the
   common case.

Recommended: prototype (3) for top-level defines (the dominant cycle
source), measure with Criterion, then revisit `letrec`.

## Out of scope

- Switching `Rc` → `Arc` (different concern; only matters if we ever
  go multithreaded inside one Vm).

## Where to start

- `crates/lisp/src/env.rs` — current Env shape.
- `crates/lisp/src/lib.rs::Vm::eval_str` — where top-level defines
  extend `self.env`.
- `crates/lisp/src/step.rs` — how closures capture env.

A leak repro for measurement: in a test, call
`Vm::new()` + `spells::install` in a loop, watch heap usage via
`jemalloc-ctl` or RSS.
