//! JS-facing bridge: wraps `lisp::Vm` for the browser via wasm-bindgen.
//!
//! Surfaces:
//!
//! - **`eval(src)`** — arbitrary lisp evaluation. Returns the formatted Val
//!   on success; throws (rejected `Result` → JS exception) on error.
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
use lisp::Vm as LispVm;
use world::World;

#[wasm_bindgen(js_name = "Vm")]
pub struct WasmVm {
    inner: LispVm,
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
    width: u32,
    height: u32,
}

#[wasm_bindgen(js_class = "Vm")]
impl WasmVm {
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32) -> Result<WasmVm, JsValue> {
        console_error_panic_hook::set_once();
        let world =
            Rc::new(RefCell::new(World::new(width, height).map_err(|e| JsValue::from_str(&e))?));
        let turtle = Rc::new(RefCell::new(Turtle::new()));
        let mut inner = LispVm::new();
        spells::install_with_world(&mut inner, world.clone());
        genes::install(&mut inner);
        curves::install(&mut inner, turtle.clone());
        // Default budget for browser hosts: 10M CEK steps. Tail-call test
        // currently uses ~1M; spells/genes runs are well under 100k. The
        // browser eval runs on the main thread, so an unbounded loop
        // hangs the page — this is the backstop.
        inner.set_step_budget(10_000_000);
        Ok(WasmVm { inner, world, turtle, width, height })
    }

    /// Override the CEK step budget for subsequent evaluations.
    /// `u64::MAX` disables the gate.
    pub fn set_step_budget(&mut self, n: u64) {
        self.inner.set_step_budget(n);
    }

    /// Evaluate arbitrary lisp source. On error, the returned `Result::Err`
    /// becomes a JS exception — JS catches with `try/catch`.
    pub fn eval(&mut self, src: &str) -> Result<String, JsValue> {
        self.inner
            .eval_str(src)
            .map(|v| format!("{v}"))
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Translate a rune tape and cast at `(x, y)`. Returns the latest log
    /// entry written by `world-apply!`, or an empty string if none. Errors
    /// (unknown rune, lex failure, eval failure) throw to JS.
    pub fn cast(&mut self, tape: &str, x: i64, y: i64) -> Result<String, JsValue> {
        let list_expr =
            runes::tape_to_sexpr(tape).map_err(|e| JsValue::from_str(&format!("rune: {e}")))?;
        // Coord seeding lives at the call site (assoc-set wrap) rather than
        // inside the shared prelude — keeps `start` zero-arg and identical
        // across CLI + WASM consumers. See ADR-010.
        let src = format!(
            "(world-apply! \
               (assoc-set 'tx {x} \
                 (assoc-set 'ty {y} \
                   (thread (start) {list_expr}))))"
        );
        self.inner
            .eval_str(&src)
            .map_err(|e| JsValue::from_str(&e))?;
        // safety: see ADR-005 — no callback primitives, so a JS handler cannot
        // re-enter Vm during this borrow.
        let log = &self.world.borrow().log;
        Ok(log.last().cloned().unwrap_or_default())
    }

    /// Newline-joined ASCII render of the world grid.
    pub fn grid(&self) -> String {
        format!("{}", self.world.borrow())
    }

    /// All log entries, newline-joined.
    pub fn log(&self) -> String {
        self.world.borrow().log.join("\n")
    }

    /// Replace the world with a fresh empty one of the same dimensions.
    /// Does *not* reset the interpreter env (the preludes were installed
    /// at construction and stay valid).
    pub fn reset_world(&mut self) {
        // Dims were validated at construction, so this can't fail.
        *self.world.borrow_mut() =
            World::new(self.width, self.height).expect("dims validated at construction");
    }

    /// Translate two codon tapes into parent genomes, breed them via
    /// `breed!`, and resolve the child with `express!`. Returns the
    /// rendered child creature card. Same `(tape_a, tape_b, seed)` →
    /// same child (Mendelian gamete pick is seeded).
    pub fn cast_breed(
        &mut self,
        tape_a: &str,
        tape_b: &str,
        seed: i64,
    ) -> Result<String, JsValue> {
        let la = codons::tape_to_sexpr(tape_a)
            .map_err(|e| JsValue::from_str(&format!("codon (parent A): {e}")))?;
        let lb = codons::tape_to_sexpr(tape_b)
            .map_err(|e| JsValue::from_str(&format!("codon (parent B): {e}")))?;
        let body = format!(
            "(express! (breed! seed (thread '() {la}) (thread '() {lb})))"
        );
        let src = genes::seeded(seed, &body);
        let phenotype = self
            .inner
            .eval_str(&src)
            .map_err(|e| JsValue::from_str(&e))?;
        Ok(genes::render_creature(&phenotype))
    }

    /// Translate a codon tape, thread it through the genome prelude, and
    /// resolve via `express!`. Returns the rendered ASCII creature card.
    /// `seed` is the lexical RNG seed for any `MUT` codons in the tape;
    /// strands without `MUT` ignore it. Errors throw to JS.
    pub fn cast_genome(&mut self, tape: &str, seed: i64) -> Result<String, JsValue> {
        let list_expr = codons::tape_to_sexpr(tape)
            .map_err(|e| JsValue::from_str(&format!("codon: {e}")))?;
        // `genes::seeded` wraps the body in a let chain so MUT's mutate
        // closure captures the caller's seed via lexical scope. See
        // ADR-012.
        let body = format!("(express! (thread '() {list_expr}))");
        let src = genes::seeded(seed, &body);
        let phenotype = self
            .inner
            .eval_str(&src)
            .map_err(|e| JsValue::from_str(&e))?;
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
        let rules = if rules_sexpr.trim().is_empty() { "()" } else { rules_sexpr };
        // Each cast resets the turtle so successive casts don't pile on
        // the same canvas. `let ((_ …))` is the project's standing
        // workaround for the missing `begin` (see ADR-019).
        let src = format!(
            "(let ((_ (reset!))) \
               (let ((_ (draw! (grow {axiom_list} '{rules} {iters})))) \
                 (render!)))"
        );
        self.inner
            .eval_str(&src)
            .map(|v| format!("{v}"))
            .map_err(|e| JsValue::from_str(&e))
    }
}
