**Findings**

1. **[P1] Browser-facing execution is unbounded and can freeze or panic on ordinary input.**  
[`world_prim.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/world_prim.rs:51) iterates the entire square implied by `area`, even though the grid is only 7x5 in the web app. A valid tape such as `ᚠ ᛊ 1000000000` enters effectively unbounded work through [`wasm/src/lib.rs`](/Users/matt/projects-new/3p/letrs/crates/wasm/src/lib.rs:56). Independently, [`runes/src/lib.rs`](/Users/matt/projects-new/3p/letrs/crates/runes/src/lib.rs:44) parses an arbitrary digit run with `unwrap()`, so an oversized numeric rune parameter panics. The REPL has the same availability problem: [`step.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/step.rs:216) has no fuel/cancellation, and [`web/common.js`](/Users/matt/projects-new/3p/letrs/web/common.js:34) evaluates on the main thread. A nonterminating Lisp expression locks the page.

Recommended direction: clamp world painting to the grid intersection, make rune numeric overflow return `Err`, and run arbitrary evaluation with a step budget or in a worker that can be terminated.

2. **[P1] Exact rational arithmetic panics on valid, representable values.**  
The folds in [`prim.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/prim.rs:27) multiply unreduced denominators in `i128` and only normalize after the whole operation. I reproduced:

```lisp
(+ 1/18446744073709551615 1/18446744073709551615)
```

This should return `2/18446744073709551615`; in the debug REPL it panics at `prim.rs:30` with integer overflow. In release/WASM, unchecked overflow risks incorrect results or later failures. The rational benchmark in [`crates/bench/benches/core.rs`](/Users/matt/projects-new/3p/letrs/crates/bench/benches/core.rs:118) exercises exactly this growth pattern.

Reduce after each binary operation and use checked arithmetic; add boundary and cancellation tests.

3. **[P1] A failed top-level `define` corrupts the persistent VM environment.**  
[`Vm::eval_str`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/lib.rs:127) publishes placeholder cells into `self.env` before definition bodies succeed. Reproduced in the REPL:

```lisp
(+ 1 2)                 ; 3
(define + (/ 1 0))      ; error: division by zero
(+ 1 2)                 ; error: not callable: #f
```

The failed redefinition has already shadowed the builtin with its placeholder. This is particularly damaging in the browser, where one REPL typo can break the labs until reload.

Build defines in a candidate environment and commit them only after successful initialization, or explicitly roll back failed placeholders. An uninitialized sentinel should also error rather than masquerade as `#f`.

4. **[P2] Top-level function definitions and `letrec` closure bindings form permanent `Rc` cycles.**  
A top-level define pre-allocates a slot in [`lib.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/lib.rs:135); evaluating a lambda captures that environment in [`step.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/step.rs:50); storing the closure back into its own slot completes `slot -> closure -> env -> frame -> slot`. This happens for every function installed by [`spells.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/spells.rs:18) and [`genes.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/genes.rs:24), even functions that do not recurse. `letrec` has the same lifetime problem.

This will distort Criterion runs that repeatedly create VMs, and it matters for long-lived REPL sessions or repeated web VM creation. The current `Rc<RefCell<Val>>` model needs a deliberate cycle strategy before expanding runtime usage.

5. **[P2] Several advertised Lisp features have correctness holes.**  
[`val_to_datum`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/lib.rs:387) omits `Val::Ratio`, so a macro expanding to `1/2` fails with `can't convert 1/2 back to a datum`.  
[`compile_qq`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/parse.rs:346) does not track quasiquote nesting depth; I observed ``(let ((x 7)) `(outer `(inner ,x)))`` produce `(outer (quasiquote (inner 7)))`, prematurely evaluating the inner unquote.  
[`compile_cond`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/parse.rs:321) permits `else` before later clauses; `(cond (else 'wrong) (#t 'right))` returns `wrong` instead of rejecting malformed syntax.

These are small fixes, but they should be pinned with regression tests because they affect the language claim directly.

6. **[P2] World input conversion silently wraps, and host mutation is not transactional.**  
[`coord`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/world_prim.rs:6) casts any nonnegative `i64` to `u32`; values above `u32::MAX` wrap to unrelated tiles. [`World::new`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/world.rs:50) similarly multiplies dimensions as `u32` before allocation. Use checked conversions and checked size calculation.

The narrative currently says “The world rolls back for free” in [`docs/letrs.html`](/Users/matt/projects-new/3p/letrs/docs/letrs.html:754), but `WorldPrim` mutates the shared `World` immediately. Evaluation that performs one mutation and subsequently errors does not roll back. Either implement snapshots for tentative evaluation or revise that claim.

7. **[P3] The web REPL and labs deliberately share one mutable VM, which makes experimentation destructive.**  
[`web/common.js`](/Users/matt/projects-new/3p/letrs/web/common.js:14) exports one VM used by both REPL and page-specific lab operations. A user can redefine `thread`, `fire`, or invoke world primitives from the REPL and break or alter the lab state. That may be useful for an expert playground, but for a demo it needs an explicit reset-VM action or separate VMs.

**Design Direction**

The CEK core is well chosen for this project. [`step.rs`](/Users/matt/projects-new/3p/letrs/crates/lisp/src/step.rs:169) makes proper tail calls straightforward, and the 100,000-call test gives that claim real backing. Splitting rune/codon translation into zero-dependency crates is also clean, and the seeded pure genetics primitives are substantially easier to reason about than hidden RNG state.

The documented direction in [`host-state.md`](/Users/matt/projects-new/3p/letrs/docs/project_notes/host-state.md:102) is sound: remove fixed `World` knowledge from the evaluator and register host closures instead. I would extend that principle further: over time, `spells`, `genes`, and `world` should become sibling vocabulary/host crates depending on `lisp`, rather than domain modules inside the language crate. The language crate can then honestly remain both dependency-free and domain-agnostic.

Do the correctness work before broader abstraction work: rational overflow, definition failure atomicity, execution limits, and `Rc` cycles affect the substrate every future DSL depends upon.

**Coverage And Maintenance**

`just test` and `just check` pass. The current workspace runs 78 tests: 45 evaluator tests, 19 genes tests, 8 rune tests, and 6 codon tests.

Important missing coverage is concentrated exactly where the risks are: there are no substantive tests for `world_prim`, spell/world integration, the WASM bridge, failed-define recovery, nested quasiquote, rational macro expansion, adversarial numeric limits, or browser workload limits.

Documentation has drifted: [`CLAUDE.md`](/Users/matt/projects-new/3p/letrs/CLAUDE.md:31) says 58 tests and later says 40; [`key_facts.md`](/Users/matt/projects-new/3p/letrs/docs/project_notes/key_facts.md:23) says 34; [`wasm/src/lib.rs`](/Users/matt/projects-new/3p/letrs/crates/wasm/src/lib.rs:7) still describes older prelude/API names. The ADR practice is a strength, but stale orientation docs will quickly negate it.