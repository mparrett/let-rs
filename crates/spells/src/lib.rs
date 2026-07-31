//! Spell-DSL host support: the shared rune prelude.
//!
//! Pulled out of `examples/spells.rs` and `crates/wasm/src/lib.rs` once
//! both consumers were keeping nearly-identical copies in sync (ADR-010's
//! "two prelude copies will eventually consolidate" clause firing —
//! mirrors the genes refactor in ADR-011). Subsequently extracted from
//! the `lisp` crate into its own sibling crate (ADR-016).
//!
//! As of ADR-025 the prelude registers two local macros (`defspell` for
//! constant ctx-setters, `defparam` for parametric ones) and uses them
//! to define the rune vocabulary. `install` therefore takes a
//! `&mut MacroVm` rather than a raw `&mut Vm`; consumers that previously
//! threaded a raw `Vm` wrap it in `macros::MacroVm` first. This is the
//! first DSL pack to adopt the macros stdlib pattern — the proof that
//! ADR-024's extraction pulls its weight in real prelude code, not just
//! at the REPL.
//!
//! The `start` closure is intentionally zero-arg. Coord seeding for the
//! WASM bridge happens at the call site via `(assoc-set 'tx … (assoc-set
//! 'ty … (thread (start) …)))`. Keeping coord data out of the prelude
//! means there's exactly one prelude string to keep in sync.

use std::cell::RefCell;
use std::rc::Rc;

use lisp::NsHandle;
use macros::MacroVm;
use world::World;

/// The spell prelude as a sequence of top-level forms. `defspell`/
/// `defparam` are defined first, then used to expand the rune
/// vocabulary into nine one-liners. The mana model (ADR-028) sits
/// at the end — a caster-side resource, drawn down by `cast!`,
/// regen'd by `tick!`.
///
/// Install once with `spells::install(mvm)` and every subsequent
/// `mvm.eval_str(body)` sees the vocabulary.
///
/// Adding a new rune that maps to a constant ctx setter: append
/// `(defspell NAME KEY VAL)`. Parametric (closes over a number arg):
/// `(defparam NAME KEY)`. Anything fancier (multi-key, conditional)
/// still wants a hand-written `(define …)`.
pub const PRELUDE_DEFINES: &str = r#"
(define assoc-set (lambda (k v ctx) (cons (cons k v) ctx)))
(define assoc-or
  (lambda (k ctx default)
    (let ((v (assoc-get k ctx)))
      (if (null? v) default v))))
(define thread    (lambda (ctx fs)
                    (if (null? fs) ctx
                        (thread ((car fs) ctx) (cdr fs)))))
(define start     (lambda () '()))

(defmacro defspell (name key val)
  `(define ,name (lambda (ctx) (assoc-set ',key ',val ctx))))

(defmacro defparam (name key)
  `(define ,name (lambda (n) (lambda (ctx) (assoc-set ',key n ctx)))))

;; ── alchemy (ADR-030) ─────────────────────────────────────────────
;; The element runes (fire / ice / earth) don't use defspell — they
;; need to look at the *prior* element in ctx and combine. defspell
;; only knows constant setters; mixing is the whole point of this
;; pack, so the three element runes go hand-written through
;; add-element + mix.
;;
;; Pairs with no explicit rule fall through to last-write-wins (else
;; b), matching the pre-alchemy behavior so unmixed tapes stay
;; predictable. Same-element-twice is idempotent.

(define mix
  (lambda (a b)
    (cond ((eq? a 'none) b)
          ((eq? a b)     a)
          ((or (and (eq? a 'fire)  (eq? b 'ice))
               (and (eq? a 'ice)   (eq? b 'fire)))  'water)
          ((or (and (eq? a 'water) (eq? b 'earth))
               (and (eq? a 'earth) (eq? b 'water))) 'mud)
          ((or (and (eq? a 'fire)  (eq? b 'earth))
               (and (eq? a 'earth) (eq? b 'fire)))  'lava)
          (else b))))

(define add-element
  (lambda (e ctx)
    (assoc-set 'element (mix (assoc-or 'element ctx 'none) e) ctx)))

(define fire  (lambda (ctx) (add-element 'fire  ctx)))
(define ice   (lambda (ctx) (add-element 'ice   ctx)))
(define earth (lambda (ctx) (add-element 'earth ctx)))

(defspell bolt shape   bolt)
(defspell self target  self)
(defparam area       area)
(defparam power      power)
(defparam aftershock aftershock)

;; ── mana model (ADR-028) ──────────────────────────────────────────
;; Caster-side resource: cast! draws it down, tick! regenerates.
;; Lives in the spells prelude (not the engine, not the world) — the
;; spell DSL owns its resource model.
;;
;; `cast!` is the mana-gated entry. Wraps `world-apply!`: computes
;; cost = 1 + power + area + aftershock (the aftershock rune pays
;; its re-strike up front — ADR-029 — so the delayed fire costs
;; nothing when it lands); on shortfall, logs and returns 0 (no
;; paint, no mana spent); on success, decrements mana and delegates
;; to world-apply!.
;;
;; `tick!` is the temporal entry. Wraps `world-tick!`: advances
;; world decay, then regens one point of mana (capped at max-mana).
;; UI hosts on a setInterval call `(tick!)` rather than
;; `(world-tick!)` directly so the two halves stay in sync.

(define max-mana 10)
(define mana     max-mana)

(define spell-cost
  (lambda (ctx)
    (+ 1
       (assoc-or 'power      ctx 0)
       (assoc-or 'area       ctx 0)
       (assoc-or 'aftershock ctx 0))))

(define cast!
  (lambda (ctx)
    (let ((cost (spell-cost ctx)))
      (if (< mana cost)
          (let ((_ (world-log! 'mana-short cost mana))) 0)
          (let ((_ (set! mana (- mana cost))))
            ;; The shortfall above is a *predictable* failure — the cost
            ;; is knowable before trying — so it stays an `if`. This
            ;; guard is for the unpredictable one: `world-apply!` rejects
            ;; a ctx with no 'element, which a tape of modifiers and no
            ;; element rune produces. That used to abort the whole
            ;; evaluation *and* keep the mana, since the charge happens
            ;; above. Refund, log, and report nothing painted. See
            ;; ADR-041.
            (guard (e (let ((_ (set! mana (+ mana cost))))
                        (let ((_ (world-log! 'cast-failed (error-message e))))
                          0)))
              (world-apply! ctx)))))))

(define tick!
  (lambda ()
    (let ((reverted (world-tick!)))
      (let ((_ (if (< mana max-mana)
                   (set! mana (+ mana 1))
                   #f)))
        reverted))))

(define reset-mana!
  (lambda ()
    (set! mana max-mana)))
"#;

/// Install the spell prelude into `mvm`. Idempotent in effect — a later
/// install shadows earlier defines of the same name (and re-registers
/// the local macros, which is a no-op for behavior).
///
/// Pulls in the macros `STDLIB` (begin/when/unless/and/or) first so
/// the prelude's `mix` table can use `and`/`or`. The alchemy logic
/// in ADR-030 made these unavoidable — sequences of `(if a (if b c
/// #f) #f)` would render the table unreadable.
/// Names this pack publishes to the root namespace (ADR-042) — the
/// vocabulary a user types in the REPL, plus the entry points and the
/// mana counter the host renders.
///
/// Everything else stays private, and `thread` / `assoc-set` / `mix` /
/// `add-element` / `spell-cost` / `assoc-or` are deliberately on the
/// private side: `thread` and `assoc-set` are the two names genes also
/// defines, and exporting either would put the collision straight back
/// into the root table. They are internals in both packs, so neither
/// needs to.
pub const EXPORTS: &[&str] = &[
    "cast!",
    "tick!",
    "reset-mana!",
    "mana",
    "max-mana",
    "start",
    "fire",
    "ice",
    "earth",
    "bolt",
    "self",
    "area",
    "power",
    "aftershock",
];

/// Install the spell prelude into its own namespace and publish
/// [`EXPORTS`] to the root. Returns the namespace, which hosts pass to
/// `eval_str_in` when running spell source — casts reference `thread`
/// and the ctx helpers, which are private.
pub fn install(mvm: &mut MacroVm) -> NsHandle {
    macros::install_stdlib(mvm).expect("macros stdlib failed to install");
    let ns = mvm.vm.namespace("spells");
    mvm.eval_str_in(&ns, PRELUDE_DEFINES)
        .expect("spells prelude failed to install");
    mvm.vm
        .export(&ns, EXPORTS)
        .expect("spells exports collided with another pack");
    ns
}

/// Convenience: install the spell prelude AND the world prims that
/// resolve a finished ctx against `world` (`world-apply!` and friends).
/// Both `examples/spells.rs` and the WASM bridge want exactly this
/// wiring; one helper saves the two-line duplication.
pub fn install_with_world(mvm: &mut MacroVm, world: Rc<RefCell<World>>) -> NsHandle {
    // World prims go to the *root*, not to this pack: they're a host
    // capability rather than spell vocabulary, `examples/world.rs` uses
    // them directly, and the spells namespace reaches them by chaining
    // outward anyway (ADR-042).
    world::world_prim::install(&mut mvm.vm, world);
    install(mvm)
}
