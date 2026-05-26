//! Spell-DSL host support: the shared rune prelude.
//!
//! Pulled out of `examples/spells.rs` and `crates/wasm/src/lib.rs` once
//! both consumers were keeping nearly-identical copies in sync (ADR-010's
//! "two prelude copies will eventually consolidate" clause firing —
//! mirrors the genes refactor in ADR-011).
//!
//! The `start` closure is intentionally zero-arg. Coord seeding for the
//! WASM bridge happens at the call site via `(assoc-set 'tx … (assoc-set
//! 'ty … (thread (start) …)))`. Keeping coord data out of the prelude
//! means there's exactly one prelude string to keep in sync.

/// The spell prelude: user-level closures that turn rune symbols into
/// pipeline primitives. Closes the `letrec` *bindings* but leaves
/// `letrec` itself open — consumers append the body and a closing paren.
pub const PRELUDE_BINDINGS: &str = r#"
(letrec ((assoc-set (lambda (k v ctx) (cons (cons k v) ctx)))
         (thread    (lambda (ctx fs)
                      (if (null? fs) ctx
                          (thread ((car fs) ctx) (cdr fs)))))
         (start     (lambda () '()))
         (fire      (lambda (ctx) (assoc-set 'element 'fire ctx)))
         (ice       (lambda (ctx) (assoc-set 'element 'ice ctx)))
         (bolt      (lambda (ctx) (assoc-set 'shape   'bolt ctx)))
         (self      (lambda (ctx) (assoc-set 'target  'self ctx)))
         (area      (lambda (n)   (lambda (ctx) (assoc-set 'area  n ctx))))
         (power     (lambda (n)   (lambda (ctx) (assoc-set 'power n ctx)))))
"#;
