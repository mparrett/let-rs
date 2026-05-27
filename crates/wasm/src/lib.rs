//! JS-facing bridge: wraps `lisp::Vm` for the browser via wasm-bindgen.
//!
//! Surfaces:
//!
//! - **`eval(src)`** — arbitrary lisp evaluation. Returns the formatted Val
//!   on success; throws (rejected `Result` → JS exception) on error.
//! - **`cast(tape, x, y)`** — rune-tape translation + spell prelude + the
//!   `world-apply!` resolver in one call. Reuses `runes::tape_to_sexpr`
//!   and `lisp::spells::install` so the CLI and the bridge stay
//!   bit-identical (ADR-010).
//! - **`cast_genome(tape, seed)`** — codon-tape translation + genome prelude +
//!   the `express!` resolver. Returns a rendered creature card. Prelude,
//!   prim, and renderer all come from `lisp::genes` (ADR-011).
//! - **`cast_breed(tape_a, tape_b, seed)`** — two parent strands → breed
//!   via `breed!` → resolve via `express!`. Same shape as `cast_genome`.
//!
//! Plus read-only `grid()` / `log()` accessors and a `reset_world()` that
//! replaces the world tiles in place while preserving dimensions.
//!
//! The whole thing is intentionally thin. No bundler, no npm —
//! `wasm-bindgen --target web` and `python3 -m http.server` are the
//! entire toolchain (ADR-009).

use wasm_bindgen::prelude::*;

use lisp::{Vm as LispVm, World, genes, spells};

#[wasm_bindgen(js_name = "Vm")]
pub struct WasmVm {
    inner: LispVm,
    width: u32,
    height: u32,
}

#[wasm_bindgen(js_class = "Vm")]
impl WasmVm {
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32) -> Result<WasmVm, JsValue> {
        console_error_panic_hook::set_once();
        let world = World::new(width, height).map_err(|e| JsValue::from_str(&e))?;
        let mut inner = LispVm::with_world(world);
        spells::install(&mut inner);
        genes::install(&mut inner);
        // Default budget for browser hosts: 10M CEK steps. Tail-call test
        // currently uses ~1M; spells/genes runs are well under 100k. The
        // browser eval runs on the main thread, so an unbounded loop
        // hangs the page — this is the backstop.
        inner.set_step_budget(10_000_000);
        Ok(WasmVm { inner, width, height })
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
        let log = &self.inner.world.borrow().log;
        Ok(log.last().cloned().unwrap_or_default())
    }

    /// Newline-joined ASCII render of the world grid.
    pub fn grid(&self) -> String {
        format!("{}", self.inner.world.borrow())
    }

    /// All log entries, newline-joined.
    pub fn log(&self) -> String {
        self.inner.world.borrow().log.join("\n")
    }

    /// Replace the world with a fresh empty one of the same dimensions.
    /// Does *not* reset the interpreter env (the preludes were installed
    /// at construction and stay valid).
    pub fn reset_world(&mut self) {
        // Dims were validated at construction, so this can't fail.
        *self.inner.world.borrow_mut() =
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
}
