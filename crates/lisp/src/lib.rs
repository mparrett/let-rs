//! A tiny functional lisp built on a CEK abstract machine.
//!
//! - `expr`        — AST: literals, variables, lambdas, applications, `if`, `letrec`, quoted data
//! - `val`         — runtime values: numbers, booleans, symbols, nil, cons, closures, primitives
//! - `env`         — `Rc`-linked environment frames (immutable, structurally shared cells)
//! - `k`           — first-class continuations (the "stack" reified as data)
//! - `step`        — `step(State) -> Step` and the driver loop
//! - `prim`        — pure built-in primitives and the initial environment
//! - `parse`       — s-expression reader + special-form compiler
//!
//! Host state is the host's problem: register state-capturing primitives
//! via [`Vm::register_prim`] (the closure captures whatever handle the
//! host owns). The engine ships no `World` type or world prims; the spell
//! demo's tile grid lives in the sibling `world` crate (ADR-017, ADR-018).
//!
//! Macros: `defmacro`, procedural macro expansion, and quasiquote-with-
//! macros live in the sibling `macros` crate (ADR-024). Hosts that want
//! macros wrap a [`Vm`] in `macros::MacroVm`. Parser-level quasiquote
//! (`\``, `,`, `,@`) stays here because it's list-construction syntax;
//! it works without macros installed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub mod env;
pub mod expr;
pub mod k;
pub mod parse;
pub mod prim;
pub mod step;
pub mod store;
pub mod val;

pub use env::{Env, Globals};
pub use expr::{Expr, Sym};
pub use parse::Datum;
pub use step::{Step, run, run_bounded};
pub use store::{Addr, Store};
pub use val::Val;

// Re-export so hosts wiring up state-capturing prims can write
// `lisp::PrimFn` (an `Rc<dyn Fn(&[Val]) -> Result<Val, String>>`) without
// reaching into `lisp::val` for the alias.
pub use val::PrimFn;

pub struct Vm {
    env: Env,
    /// Top-level bindings (every `(define …)` ever evaluated in this
    /// Vm). Held by `Vm` as the sole strong reference; closures that
    /// reach back to it via `env.globals` use a `Weak`, so dropping
    /// this Vm collapses every closure stored here without leaks. See
    /// ADR-015.
    pub globals: Globals,
    /// Lexical-binding heap (frame slots; `let`/`letrec`/lambda
    /// params). The fourth CESK register from ADR-023. Owned by `Vm`
    /// strong; reached by closures via `Env::store` as a `Weak`, so a
    /// closure can't keep the store alive past its Vm.
    pub store: Rc<Store>,
    /// CEK step budget, applied *per top-level form* — not per
    /// `eval_str` call. A source of N forms can therefore run up to
    /// N × `step_budget` steps in total; the budget bounds any single
    /// nonterminating expression, not the aggregate work of a batch.
    /// `u64::MAX` (the default) is effectively unbounded — preserves
    /// day-one behavior. Hosts that can't otherwise interrupt
    /// evaluation (the WASM bridge, the REPL) should lower this via
    /// `set_step_budget` so a nonterminating expression surfaces as an
    /// error instead of a hung page.
    step_budget: u64,
}

impl Vm {
    /// Construct a Vm with no host state. Hosts that need state-aware
    /// primitives register them via [`Vm::register_prim`] with closures
    /// that capture whatever handle they own. ADR-017 removed the
    /// engine's awareness of a privileged `World` type; hosts wanting a
    /// tile grid call `world::world_prim::install(&mut vm, world)` on
    /// top (ADR-018).
    pub fn new() -> Self {
        let globals: Globals = Rc::new(RefCell::new(HashMap::new()));
        let store: Rc<Store> = Rc::new(Store::new());
        prim::install_builtins(&globals);
        let env = Env::with_globals(&globals, &store);
        Vm {
            env,
            globals,
            store,
            step_budget: u64::MAX,
        }
    }

    /// `Weak` handle to the Vm's store. Exposed for ADR-023's
    /// `letrec_does_not_leak` diagnostic, which asserts the store
    /// drops with the Vm — proof that no closure rooted it.
    pub fn store_weak(&self) -> Weak<Store> {
        Rc::downgrade(&self.store)
    }

    /// Borrow the Vm's root environment. Exposed for the `macros`
    /// crate to capture as the lexical env of macro closures.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Cap each top-level form at `n` CEK steps (see [`Vm::step_budget`]
    /// — the cap is per form, so a batch of N forms can spend up to
    /// N × `n`). Forms that exceed the budget return
    /// `Err("execution exceeded step budget")`. Set to `u64::MAX` to
    /// disable.
    pub fn set_step_budget(&mut self, n: u64) {
        self.step_budget = n;
    }

    /// Install a host primitive into the VM's globals table. The
    /// callback is wrapped in an `Rc<dyn Fn>` so it can capture host
    /// state — e.g., `move |args| { /* read/write &mut world.borrow_mut() */ }`.
    /// Replaces the ADR-005 split between pure `register_prim` and
    /// world-aware `register_world_prim`; both shapes collapse here.
    /// Host prims live in the same table as `BUILTINS` and user
    /// `(define …)` bindings (ADR-020): a later `(define name v)`
    /// overwrites the prim slot.
    pub fn register_prim<F>(&mut self, name: &'static str, arity: val::Arity, f: F)
    where
        F: Fn(&[Val]) -> Result<Val, String> + 'static,
    {
        let val = Val::Prim {
            name,
            arity,
            f: Rc::new(f),
        };
        self.globals
            .borrow_mut()
            .insert(name.into(), Rc::new(RefCell::new(val)));
    }

    /// Evaluate a sequence of top-level forms. Each form is one of:
    /// `(define name body)` (writes to the Vm's globals table) or any
    /// expression (compiled + run normally). Returns the value of the
    /// last expression — or `#t` if every form was a `define` (i.e.
    /// nothing produced a value).
    ///
    /// All `(define name body)` forms in a single call have placeholder
    /// cells pre-allocated *before* any body runs, so defines in the
    /// same batch may freely refer to each other (mutual recursion).
    /// And because top-level defines now live in a shared globals
    /// table that every Env points back to, mutual recursion across
    /// separate `eval_str` calls also works — a closure looked up via
    /// globals sees the table's *current* contents, not a snapshot of
    /// its own capture time. See ADR-015.
    ///
    /// If any form in the batch fails, the globals *table* is restored
    /// to its pre-call state, so a failed `(define …)` can't leave a
    /// placeholder shadowing a builtin. Values reached through cells
    /// that already existed are not restored — `set!` and host prim
    /// effects survive a failed batch. See the note in the body.
    ///
    /// `(defmacro …)` is *not* recognized here — macros live in the
    /// sibling `macros` crate (ADR-024). Hosts that want macros wrap
    /// this Vm in `macros::MacroVm`.
    pub fn eval_str(&mut self, src: &str) -> Result<Val, String> {
        // Binding-level rollback: if any form in the batch fails,
        // restore the globals *table* to its pre-call state. Pre-fix, a
        // failed define left a placeholder cell visible in env (e.g.
        // `(define + (/ 1 0))` masking the builtin `+` with `#f`),
        // which then took every subsequent REPL line down.
        //
        // Snapshotting the HashMap clones it but Rc-bumps each cell,
        // so cost is O(globals.len()) and unchanged-cells are shared.
        //
        // This is *not* transactional rollback of effects, and the
        // sharing is exactly why: the snapshot restores which cell each
        // name points at, not what's inside those cells. `set!`
        // (ADR-026, which postdates this rollback) writes through the
        // shared `RefCell`, so `(set! x 99) (car 5)` fails the batch
        // and leaves `x` at 99. Host prim effects — painted tiles,
        // turtle state, log entries — likewise stand. Undoing those
        // would need the persistent store ADR-023 leaves open.
        let saved_globals = self.globals.borrow().clone();
        let result = self.eval_str_inner(src);
        if result.is_err() {
            *self.globals.borrow_mut() = saved_globals;
        }
        result
    }

    fn eval_str_inner(&mut self, src: &str) -> Result<Val, String> {
        let forms = parse::read_many(src)?;

        // Pre-pass: allocate placeholder cells in `globals` for every
        // top-level `(define name body)` in this batch. Bodies that
        // reference any sibling-define's name (or their own) resolve
        // through `Env::lookup`'s globals fallback to these cells.
        // Same name defined twice in one batch: the second
        // pre-allocation overwrites the first cell; both bodies then
        // write to the second cell. The first cell becomes garbage.
        let mut define_cells: HashMap<String, Rc<RefCell<Val>>> = HashMap::new();
        for datum in &forms {
            if let Some(name) = extract_define_name(datum)? {
                let cell = Rc::new(RefCell::new(Val::Bool(false)));
                self.globals.borrow_mut().insert(name.clone(), cell.clone());
                define_cells.insert(name.to_string(), cell);
            }
        }

        let mut last = Val::Bool(true);
        for datum in forms {
            if self.try_register_define(&datum, &define_cells)? {
                continue;
            }
            let expr = parse::compile(&datum)?;
            last = run_bounded(expr, self.env.clone(), self.step_budget)?;
        }
        Ok(last)
    }

    /// Evaluate `(define name body)` against `self.env`, then write
    /// the result into the cell pre-allocated for `name` by the
    /// `eval_str` pre-pass. Returns `Ok(false)` if `d` isn't a
    /// `define` form, propagating non-define forms to the caller.
    fn try_register_define(
        &mut self,
        d: &Datum,
        cells: &HashMap<String, Rc<RefCell<Val>>>,
    ) -> Result<bool, String> {
        let name = match extract_define_name(d)? {
            Some(n) => n,
            None => return Ok(false),
        };
        let items = match d {
            Datum::List(items) => items,
            _ => unreachable!("extract_define_name returned Some, so d is a List"),
        };
        let body_expr = parse::compile(&items[2])?;
        let val = run_bounded(body_expr, self.env.clone(), self.step_budget)?;
        cells
            .get(name.as_ref())
            .expect("pre-pass should have allocated a cell for this define")
            .borrow_mut()
            .clone_from(&val);
        Ok(true)
    }

    /// Apply a callable `Val` (closure or prim) to `args`. Exposed for
    /// the `macros` crate to call macro closures; hosts generally don't
    /// need this — top-level evaluation goes through [`Vm::eval_str`].
    pub fn call_value(&self, f: &Val, args: Vec<Val>) -> Result<Val, String> {
        let mut app: Vec<Rc<Expr>> = vec![Rc::new(Expr::Quote(Rc::new(f.clone())))];
        for a in args {
            app.push(Rc::new(Expr::Quote(Rc::new(a))));
        }
        run_bounded(Expr::App(app), self.env.clone(), self.step_budget)
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// Inspect a datum and, if it's a well-formed `(define name body)`,
/// return the bound name. Used by `eval_str`'s pre-pass (to allocate
/// placeholder cells) and `try_register_define` (to validate the
/// form structure and find its cell).
fn extract_define_name(d: &Datum) -> Result<Option<Sym>, String> {
    let items = match d {
        Datum::List(items) => items,
        _ => return Ok(None),
    };
    match items.first() {
        Some(Datum::Sym(s)) if &**s == "define" => {}
        _ => return Ok(None),
    }
    if items.len() != 3 {
        return Err("define: expected (define name value)".into());
    }
    match &items[1] {
        Datum::Sym(s) => Ok(Some(s.clone())),
        _ => Err("define: name must be a symbol".into()),
    }
}
