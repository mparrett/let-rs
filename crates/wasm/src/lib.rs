//! JS-facing bridge: wraps `lisp::Vm` for the browser via wasm-bindgen.
//!
//! Surfaces:
//!
//! - **`eval(src)`** — arbitrary lisp evaluation. Returns the formatted Val
//!   on success; throws (rejected `Result` → JS exception) on error.
//! - **`eval_start(src)` / `eval_resume(slice)` / `eval_cancel()`** — the
//!   same evaluation in slices, so the page stays responsive and the user
//!   can cancel. `eval_resume` returns `null` while there's more to do.
//!   See ADR-040; this is why the pausable machine exists.
//! - **`cast(tape, x, y)`** — rune-tape translation + spell prelude + the
//!   `world-apply!` resolver in one call. Reuses `runes::tape_to_sexpr`
//!   and `spells::install` so the CLI and the bridge stay
//!   bit-identical (ADR-010, ADR-016).
//! - **`cast_genome(tape, seed)`** — codon-tape translation + genome prelude +
//!   the `express!` resolver. Returns a rendered creature card. Prelude,
//!   prim, and renderer all come from `genes` (ADR-011, ADR-016).
//! - **`cast_breed(tape_a, tape_b, seed)`** — two parent strands → breed
//!   via `breed!` → resolve via `express!`. Same shape as `cast_genome`.
//! - **`cast_curve(axiom, rules_sexpr, iters)`** — stroke-tape translation +
//!   pure-lisp `grow` rewrite + side-effecting `draw!` + `render!`. Returns
//!   the rendered ASCII canvas. Rules arrive pre-built as a lisp form
//!   (the page module's job — see `web/curves.js`) so the bridge stays a
//!   thin wrapper. See ADR-019.
//!
//! Plus read-only `grid()` / `log()` accessors and a `reset_world()` that
//! replaces the world tiles in place while preserving dimensions.
//!
//! The whole thing is intentionally thin. No bundler, no npm —
//! `wasm-bindgen --target web` and `python3 -m http.server` are the
//! entire toolchain (ADR-009).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use curves::Turtle;
use lisp::{LispErr, Namespace, Session};
use macros::MacroVm;
use world::World;

/// Error from source *this bridge* assembled — a cast pipeline, a
/// `genes::seeded` wrapper, the curve `begin` form. Drops the span
/// deliberately: it names a line and column in generated text the user
/// never saw, so reporting it would be worse than reporting nothing.
/// User-authored source goes through `LispErr::render` instead; see
/// `eval` below.
fn generated_err(e: LispErr) -> JsValue {
    JsValue::from_str(&e.msg)
}

#[wasm_bindgen(js_name = "Vm")]
pub struct WasmVm {
    /// Macro-aware Vm: bundles `lisp::Vm` + `macros::Expander`. Hosts
    /// reach the inner lisp engine via `inner.vm` (e.g. for prim
    /// registration, prelude installs, the step-budget setter).
    inner: MacroVm,
    /// Host-owned world handle. The lisp engine no longer carries a
    /// `World` field (ADR-017); the bridge owns this Rc and shares a
    /// clone with the world prims via closure capture in
    /// `world::world_prim::install`.
    world: Rc<RefCell<World>>,
    /// Host-owned turtle handle for the curves DSL. Same pattern as
    /// `world`: the bridge owns the Rc, `curves::install` captures a
    /// clone in the `draw!`/`render!`/`reset!` prims. See ADR-019.
    #[allow(dead_code)] // held to keep the prim closures alive
    turtle: Rc<RefCell<Turtle>>,
    /// Each pack's namespace (ADR-042). Cast source references helpers
    /// the packs keep private — `thread` above all, which spells and
    /// genes both define — so every generated cast evaluates *inside*
    /// the pack it belongs to rather than at the root.
    spells_ns: Rc<Namespace>,
    genes_ns: Rc<Namespace>,
    curves_ns: Rc<Namespace>,
    /// An in-flight resumable evaluation, if any (ADR-040), plus the
    /// source it came from so errors can still be rendered with a caret
    /// after `eval_start` has returned.
    ///
    /// This exists because a browser host cannot block. Before it, the
    /// only defense against a nonterminating expression was the step
    /// budget: the page froze for however long 10M steps took and then
    /// reported an error, and there was no way to show progress or let the
    /// user cancel. A `Session` holds no borrow of the `Vm`, so it parks
    /// here between animation frames.
    pending: Option<(Session, String)>,
    width: u32,
    height: u32,
}

#[wasm_bindgen(js_class = "Vm")]
impl WasmVm {
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32) -> Result<WasmVm, JsValue> {
        console_error_panic_hook::set_once();
        let world = Rc::new(RefCell::new(
            World::new(width, height).map_err(|e| JsValue::from_str(&e))?,
        ));
        let turtle = Rc::new(RefCell::new(Turtle::new()));
        let mut inner = MacroVm::with_stdlib();
        let spells_ns = spells::install_with_world(&mut inner, world.clone());
        let genes_ns = genes::install(&mut inner.vm);
        let curves_ns = curves::install(&mut inner.vm, turtle.clone());
        // Default budget for browser hosts: 10M CEK steps. Tail-call test
        // currently uses ~1M; spells/genes runs are well under 100k. The
        // browser eval runs on the main thread, so an unbounded loop
        // hangs the page — this is the backstop.
        inner.vm.set_step_budget(10_000_000);
        Ok(WasmVm {
            inner,
            world,
            turtle,
            spells_ns,
            genes_ns,
            curves_ns,
            pending: None,
            width,
            height,
        })
    }

    /// Override the CEK step budget for subsequent evaluations.
    /// `u64::MAX` disables the gate.
    pub fn set_step_budget(&mut self, n: u64) {
        self.inner.vm.set_step_budget(n);
    }

    /// Evaluate arbitrary lisp source. On error, the returned `Result::Err`
    /// becomes a JS exception — JS catches with `try/catch`.
    pub fn eval(&mut self, src: &str) -> Result<String, JsValue> {
        self.inner
            .eval_str(src)
            .map(|v| format!("{v}"))
            // The REPL is the one surface where the user wrote the text
            // we evaluated, so it's the one place a rendered span with a
            // caret under the offending token is meaningful (ADR-039).
            .map_err(|e| JsValue::from_str(&e.render(src)))
    }

    /// Begin evaluating `src` without running any of it, for hosts that
    /// want to evaluate in slices rather than block (ADR-040). Reading and
    /// macro expansion happen here, so a syntax error throws from this
    /// call; drive the rest with [`WasmVm::eval_resume`].
    ///
    /// Replaces any evaluation already in flight — starting a new one is
    /// an implicit cancel, which is what a REPL wants when the user
    /// submits a second line.
    pub fn eval_start(&mut self, src: &str) -> Result<(), JsValue> {
        self.pending = None;
        let session = self
            .inner
            .start(src)
            .map_err(|e| JsValue::from_str(&e.render(src)))?;
        self.pending = Some((session, src.to_string()));
        Ok(())
    }

    /// Advance the in-flight evaluation by at most `slice` CEK steps.
    ///
    /// Returns the formatted value once finished, or `null` while there's
    /// more to do — so the JS side is `while (r === null) await frame()`.
    /// Throws if evaluation fails, or if nothing is in flight.
    ///
    /// Pick `slice` from the frame budget you want to hold: a few tens of
    /// thousands of steps is well under a frame on any machine that can
    /// run this page, and the step budget still catches a runaway
    /// independently (see `Vm::resume`).
    pub fn eval_resume(&mut self, slice: u32) -> Result<Option<String>, JsValue> {
        let Some((mut session, src)) = self.pending.take() else {
            return Err(JsValue::from_str("no evaluation in flight"));
        };
        match self.inner.vm.resume(&mut session, u64::from(slice)) {
            Ok(lisp::Progress::Done(v)) => Ok(Some(format!("{v}"))),
            Ok(lisp::Progress::Paused) => {
                self.pending = Some((session, src));
                Ok(None)
            }
            Err(e) => Err(JsValue::from_str(&e.render(&src))),
        }
    }

    /// Steps spent on the form currently in flight, for a progress
    /// readout. `0` when nothing is running.
    pub fn eval_steps(&self) -> f64 {
        // f64 rather than u64: this crosses into JS, where a u64 becomes a
        // BigInt and can't be compared or formatted alongside plain
        // numbers without ceremony. Step counts stay exact well past any
        // budget a browser host would set.
        self.pending
            .as_ref()
            .and_then(|(s, _)| s.machine())
            .map_or(0.0, |m| m.steps() as f64)
    }

    /// Abandon the in-flight evaluation. Whatever already ran stands —
    /// completed `define`s keep their values and host effects are not
    /// undone (see `Vm::resume`). The Vm stays usable.
    pub fn eval_cancel(&mut self) {
        self.pending = None;
    }

    /// Whether an evaluation is in flight.
    pub fn eval_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Translate a rune tape and cast at `(x, y)`. Routes through
    /// `cast!` (the mana-gated wrapper from the spells prelude, ADR-028)
    /// rather than calling `world-apply!` directly — so a cast that
    /// exceeds the current mana budget logs a `mana-short` event,
    /// returns 0, and doesn't paint. Returns the latest log entry
    /// (mana-short, cast, or empty if none). Errors (unknown rune,
    /// lex failure, eval failure) throw to JS.
    pub fn cast(&mut self, tape: &str, x: i64, y: i64) -> Result<String, JsValue> {
        let list_expr =
            runes::tape_to_sexpr(tape).map_err(|e| JsValue::from_str(&format!("rune: {e}")))?;
        // Coord seeding lives at the call site (assoc-set wrap) rather than
        // inside the shared prelude — keeps `start` zero-arg and identical
        // across CLI + WASM consumers. See ADR-010.
        let src = format!(
            "(cast! \
               (assoc-set 'tx {x} \
                 (assoc-set 'ty {y} \
                   (thread (start) {list_expr}))))"
        );
        let ns = Rc::clone(&self.spells_ns);
        self.inner.eval_str_in(&ns, &src).map_err(generated_err)?;
        // safety: see ADR-005 — no callback primitives, so a JS handler cannot
        // re-enter Vm during this borrow.
        let log = &self.world.borrow().log;
        Ok(log.last().cloned().unwrap_or_default())
    }

    /// Advance the world by one tick via the spells prelude's `tick!`
    /// wrapper (ADR-027 decay + ADR-028 mana regen). The lab UI is
    /// expected to call this on a setInterval. Returns the number of
    /// tiles that decayed this tick (0+ as a string for JS).
    pub fn tick(&mut self) -> Result<String, JsValue> {
        self.inner
            .eval_str_in(&Rc::clone(&self.spells_ns), "(tick!)")
            .map(|v| format!("{v}"))
            .map_err(generated_err)
    }

    /// Read a lisp-side integer global, or 0 if it's unbound or not a
    /// number. Mana lives in the spells prelude rather than in host
    /// state (ADR-028, grandfathered by ADR-037), so the bridge reads
    /// it back out — but via `Vm::global`, a hashmap lookup, rather
    /// than by evaluating source.
    fn global_int(&self, name: &str) -> i32 {
        match self.inner.vm.global(name) {
            Some(lisp::Val::Num(n)) => i32::try_from(n).unwrap_or(0),
            _ => 0,
        }
    }

    /// Current mana value, as an i32 for the UI meter.
    pub fn mana(&self) -> i32 {
        self.global_int("mana")
    }

    /// Mana cap. Read once at startup; doesn't change unless something
    /// rewrites `max-mana` from lisp.
    pub fn max_mana(&self) -> i32 {
        self.global_int("max-mana")
    }

    /// Newline-joined ASCII render of the world grid.
    pub fn grid(&self) -> String {
        format!("{}", self.world.borrow())
    }

    /// All log entries, newline-joined.
    pub fn log(&self) -> String {
        self.world.borrow().log.join("\n")
    }

    /// Replace the world with a fresh empty one of the same dimensions
    /// AND restore mana to its max (ADR-028). The interpreter env is
    /// preserved — preludes were installed at construction.
    pub fn reset_world(&mut self) {
        // Dims were validated at construction, so this can't fail.
        *self.world.borrow_mut() =
            World::new(self.width, self.height).expect("dims validated at construction");
        // Best-effort: reset-mana! is defined by the spells prelude;
        // if a future bridge drops that prelude, `global` returns None
        // and this no-ops cleanly. Looking the closure up and calling
        // it beats `eval_str("(reset-mana!)")` — no reparse, and the
        // "not installed" case is a `None` rather than an error string
        // we'd have to discard blind.
        if let Some(f) = self.inner.vm.global("reset-mana!") {
            let _ = self.inner.vm.call_value(&f, vec![]);
        }
    }

    /// Translate two codon tapes into parent genomes, breed them via
    /// `breed!`, and resolve the child with `express!`. Returns the
    /// rendered child creature card. Same `(tape_a, tape_b, seed)` →
    /// same child (Mendelian gamete pick is seeded).
    pub fn cast_breed(&mut self, tape_a: &str, tape_b: &str, seed: i64) -> Result<String, JsValue> {
        let la = codons::tape_to_sexpr(tape_a)
            .map_err(|e| JsValue::from_str(&format!("codon (parent A): {e}")))?;
        let lb = codons::tape_to_sexpr(tape_b)
            .map_err(|e| JsValue::from_str(&format!("codon (parent B): {e}")))?;
        let body = format!("(express! (breed! seed (thread '() {la}) (thread '() {lb})))");
        let src = genes::seeded(seed, &body);
        let ns = Rc::clone(&self.genes_ns);
        let phenotype = self.inner.eval_str_in(&ns, &src).map_err(generated_err)?;
        Ok(genes::render_creature(&phenotype))
    }

    /// Translate a codon tape, thread it through the genome prelude, and
    /// resolve via `express!`. Returns the rendered ASCII creature card.
    /// `seed` is the lexical RNG seed for any `MUT` codons in the tape;
    /// strands without `MUT` ignore it. Errors throw to JS.
    pub fn cast_genome(&mut self, tape: &str, seed: i64) -> Result<String, JsValue> {
        let list_expr =
            codons::tape_to_sexpr(tape).map_err(|e| JsValue::from_str(&format!("codon: {e}")))?;
        // `genes::seeded` wraps the body in a let chain so MUT's mutate
        // closure captures the caller's seed via lexical scope. See
        // ADR-012.
        let body = format!("(express! (thread '() {list_expr}))");
        let src = genes::seeded(seed, &body);
        let ns = Rc::clone(&self.genes_ns);
        let phenotype = self.inner.eval_str_in(&ns, &src).map_err(generated_err)?;
        Ok(genes::render_creature(&phenotype))
    }

    /// Translate a stroke axiom, optionally rewrite it `iters` times under
    /// `rules_sexpr`, dispatch the result through `draw!`, and return the
    /// rendered ASCII canvas via `render!`. `rules_sexpr` is a *lisp*
    /// rules list as a string (e.g. `"((F F + F - F))"`); the page module
    /// builds it from the per-line `lhs = rhs` UI input so the bridge
    /// stays domain-neutral. Empty string is shorthand for `()` (no
    /// rewrite — useful for axiom-only casts like the octagon).
    pub fn cast_curve(
        &mut self,
        axiom: &str,
        rules_sexpr: &str,
        iters: i32,
    ) -> Result<String, JsValue> {
        let axiom_list = strokes::tape_to_sexpr(axiom)
            .map_err(|e| JsValue::from_str(&format!("stroke: {e}")))?;
        let rules = if rules_sexpr.trim().is_empty() {
            "()"
        } else {
            rules_sexpr
        };
        // Each cast resets the turtle so successive casts don't pile on
        // the same canvas. `begin` comes from `macros::install_stdlib`
        // (registered in `new()` via `MacroVm::with_stdlib`); it
        // expands to the `(let ((_ …)) …)` sequencing chain that was
        // the standing workaround before ADR-024's macros crate gave
        // us a place to put it.
        let src = format!(
            "(begin (reset!) \
                    (draw! (grow {axiom_list} '{rules} {iters})) \
                    (render!))"
        );
        self.inner
            .eval_str_in(&Rc::clone(&self.curves_ns), &src)
            .map(|v| format!("{v}"))
            .map_err(generated_err)
    }
}
