//! JS-facing bridge: wraps `lisp::Vm` for the browser via wasm-bindgen.
//!
//! Three surfaces:
//!
//! - **`eval(src)`** — arbitrary lisp evaluation. Returns the formatted Val
//!   on success; throws (rejected `Result` → JS exception) on error.
//! - **`cast(tape, x, y)`** — rune-tape translation + spell prelude + the
//!   `world-apply!` resolver in one call. Reuses the rune translator from
//!   the `runes` crate (ADR-010) so it stays in sync with the CLI demo.
//! - **`cast_genome(tape)`** — codon-tape translation + genome prelude +
//!   the `express!` resolver. Returns a rendered creature card. The
//!   prelude, prim, and renderer all come from `lisp::genes` so the CLI
//!   demo and this bridge share one source of truth (ADR-011).
//!
//! Plus read-only `grid()` / `log()` accessors and a `reset_world()` that
//! replaces the world tiles in place while preserving dimensions.
//!
//! The whole thing is intentionally thin. No bundler, no npm —
//! `wasm-bindgen --target web` and `python3 -m http.server` are the
//! entire toolchain (ADR-009).

use wasm_bindgen::prelude::*;

use lisp::{Vm as LispVm, World, genes};

/// The spell prelude — the user-level closures that turn rune symbols into
/// pipeline primitives. Closes the letrec bindings list but leaves letrec
/// itself open: `cast()` appends the body and a closing paren.
const SPELL_PRELUDE_BINDINGS: &str = r#"
(letrec ((assoc-set (lambda (k v ctx) (cons (cons k v) ctx)))
         (thread    (lambda (ctx fs)
                      (if (null? fs) ctx
                          (thread ((car fs) ctx) (cdr fs)))))
         (start     (lambda (x y) (assoc-set 'ty y (assoc-set 'tx x '()))))
         (fire      (lambda (ctx) (assoc-set 'element 'fire ctx)))
         (ice       (lambda (ctx) (assoc-set 'element 'ice ctx)))
         (bolt      (lambda (ctx) (assoc-set 'shape   'bolt ctx)))
         (self      (lambda (ctx) (assoc-set 'target  'self ctx)))
         (area      (lambda (n)   (lambda (ctx) (assoc-set 'area  n ctx))))
         (power     (lambda (n)   (lambda (ctx) (assoc-set 'power n ctx)))))
"#;

#[wasm_bindgen(js_name = "Vm")]
pub struct WasmVm {
    inner: LispVm,
    width: u32,
    height: u32,
}

#[wasm_bindgen(js_class = "Vm")]
impl WasmVm {
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32) -> WasmVm {
        console_error_panic_hook::set_once();
        let mut inner = LispVm::with_world(World::new(width, height));
        genes::install(&mut inner);
        WasmVm { inner, width, height }
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
        let src = format!(
            "{SPELL_PRELUDE_BINDINGS}  (world-apply! (thread (start {x} {y}) {list_expr})))"
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
    /// Does *not* reset the interpreter env (the lisp prelude is rebuilt per
    /// cast, so there's nothing persistent to clear).
    pub fn reset_world(&mut self) {
        *self.inner.world.borrow_mut() = World::new(self.width, self.height);
    }

    /// Translate a codon tape, thread it through the genome prelude, and
    /// resolve via `express!`. Returns the rendered ASCII creature card.
    /// Errors (unknown codon, lex failure, eval failure) throw to JS.
    pub fn cast_genome(&mut self, tape: &str) -> Result<String, JsValue> {
        let list_expr = codons::tape_to_sexpr(tape)
            .map_err(|e| JsValue::from_str(&format!("codon: {e}")))?;
        let src = format!(
            "{}  (express! (thread '() {list_expr})))",
            genes::PRELUDE_BINDINGS
        );
        let phenotype = self
            .inner
            .eval_str(&src)
            .map_err(|e| JsValue::from_str(&e))?;
        Ok(genes::render_creature(&phenotype))
    }
}
