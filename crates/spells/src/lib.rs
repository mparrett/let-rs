//! Spell-DSL host support: the shared rune prelude.
//!
//! Pulled out of `examples/spells.rs` and `crates/wasm/src/lib.rs` once
//! both consumers were keeping nearly-identical copies in sync (ADR-010's
//! "two prelude copies will eventually consolidate" clause firing —
//! mirrors the genes refactor in ADR-011). Subsequently extracted from
//! the `lisp` crate into its own sibling crate (ADR-016).
//!
//! The `start` closure is intentionally zero-arg. Coord seeding for the
//! WASM bridge happens at the call site via `(assoc-set 'tx … (assoc-set
//! 'ty … (thread (start) …)))`. Keeping coord data out of the prelude
//! means there's exactly one prelude string to keep in sync.

use lisp::Vm;

/// The spell prelude as a sequence of top-level `(define …)` forms.
/// Install once with `spells::install(vm)` and every subsequent
/// `vm.eval_str(body)` sees the vocabulary.
pub const PRELUDE_DEFINES: &str = r#"
(define assoc-set (lambda (k v ctx) (cons (cons k v) ctx)))
(define thread    (lambda (ctx fs)
                    (if (null? fs) ctx
                        (thread ((car fs) ctx) (cdr fs)))))
(define start     (lambda () '()))
(define fire      (lambda (ctx) (assoc-set 'element 'fire ctx)))
(define ice       (lambda (ctx) (assoc-set 'element 'ice ctx)))
(define bolt      (lambda (ctx) (assoc-set 'shape   'bolt ctx)))
(define self      (lambda (ctx) (assoc-set 'target  'self ctx)))
(define area      (lambda (n)   (lambda (ctx) (assoc-set 'area  n ctx))))
(define power     (lambda (n)   (lambda (ctx) (assoc-set 'power n ctx))))
"#;

/// Install the spell prelude into `vm`. Idempotent in effect — a later
/// install shadows earlier defines of the same name.
pub fn install(vm: &mut Vm) {
    vm.eval_str(PRELUDE_DEFINES)
        .expect("spells prelude failed to install");
}
