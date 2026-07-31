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
pub mod error;
pub mod expr;
pub mod k;
pub mod ns;
pub mod parse;
pub mod prim;
pub mod step;
pub mod store;
pub mod val;

pub use env::{Env, Globals};
pub use error::{LispErr, Span, render_span};
pub use expr::{Expr, Sym};
pub use ns::Namespace;
pub use parse::{Datum, DatumKind};
pub use step::{Machine, Progress, Step, run, run_bounded};
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
    ///
    /// Private (ADR-036): the sole-strong-reference property above is
    /// only true if nothing outside can clone the `Rc`. Hosts install
    /// bindings through [`Vm::register_prim`] and [`Vm::eval_str`];
    /// tests observe cell lifetime through [`Vm::global_cell_weak`].
    globals: Globals,
    /// Lexical-binding heap (frame slots; `let`/`letrec`/lambda
    /// params). The fourth CESK register from ADR-023. Owned by `Vm`
    /// strong; reached by closures via `Env::store` as a `Weak`, so a
    /// closure can't keep the store alive past its Vm.
    ///
    /// Private (ADR-036). `Store::set` writes any `Addr` without
    /// checking that a live `Frame` owns it, which is sound only
    /// because the engine never lets an `Addr` escape the `Env` that
    /// keeps its frame alive (ADR-033). A `pub` handle here would have
    /// been the way to break exactly that. Diagnostics go through
    /// [`Vm::store_weak`].
    store: Rc<Store>,
    /// Namespaces created by [`Vm::namespace`], keyed by name (ADR-042).
    /// The `Vm` holds the sole strong references, matching how it holds
    /// the root: a pack's closures reach their namespace through
    /// `Env::globals` as a `Weak`, so dropping the Vm collapses every
    /// pack without leaks, exactly as ADR-015 arranged for the root.
    packs: HashMap<String, Rc<Namespace>>,
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
        let globals: Globals = Namespace::root();
        let store: Rc<Store> = Rc::new(Store::new());
        prim::install_builtins(&globals);
        let env = Env::with_globals(&globals, &store);
        Vm {
            env,
            globals,
            packs: HashMap::new(),
            store,
            step_budget: u64::MAX,
        }
    }

    /// The root namespace: builtins, user top-level `define`s, and
    /// whatever the installed packs have exported (ADR-042).
    pub fn root(&self) -> &Rc<Namespace> {
        &self.globals
    }

    /// Get or create the namespace named `name`, a child of the root.
    ///
    /// A DSL pack calls this once at install time and evaluates its
    /// prelude there. Its internal helpers stay private to it, so two
    /// packs can both define `thread` without either noticing — which
    /// they already both did, identically, until this existed.
    pub fn namespace(&mut self, name: &str) -> Rc<Namespace> {
        if let Some(ns) = self.packs.get(name) {
            return Rc::clone(ns);
        }
        let ns = Namespace::child(name, &self.globals);
        self.packs.insert(name.to_string(), Rc::clone(&ns));
        ns
    }

    /// The namespace named `name`, if it has been created.
    pub fn find_namespace(&self, name: &str) -> Option<Rc<Namespace>> {
        self.packs.get(name).map(Rc::clone)
    }

    /// Publish `names` from `ns` into the root, so unqualified code —
    /// the REPL, a host's generated source — can reach a pack's public
    /// vocabulary.
    ///
    /// Exported names share the *cell*, not the value, so `set!` through
    /// either path writes the same slot; that's what lets the mana
    /// counter live in the spells pack and still be read from root.
    /// Fails, naming both packs, if another pack already exported the
    /// same name — the diagnostic whose absence made the old flat table
    /// dangerous.
    pub fn export(&mut self, ns: &Rc<Namespace>, names: &[&str]) -> Result<(), LispErr> {
        for name in names {
            ns.export(&self.globals, name).map_err(LispErr::new)?;
        }
        Ok(())
    }

    /// `Weak` handle to the Vm's store. Exposed for ADR-023's
    /// `letrec_does_not_leak` diagnostic, which asserts the store
    /// drops with the Vm — proof that no closure rooted it.
    pub fn store_weak(&self) -> Weak<Store> {
        Rc::downgrade(&self.store)
    }

    /// Read the current value of top-level binding `name`, or `None` if
    /// it isn't bound.
    ///
    /// This is the supported way for a host to read a lisp-side value.
    /// Before it existed the WASM bridge read its mana counter with
    /// `eval_str("mana")` — tokenize, compile to `Expr::Var`, run the
    /// CEK machine — to do a hashmap lookup. Hosts that render
    /// lisp-owned state (see the state-placement rule in ADR-037)
    /// should reach for this rather than evaluating source.
    ///
    /// Takes `&self`: reading a binding is not evaluation and shouldn't
    /// require a mutable borrow of the Vm.
    pub fn global(&self, name: &str) -> Option<Val> {
        self.globals.get(name)
    }

    /// [`Vm::global`] against a specific namespace, for reading a value a
    /// pack keeps private.
    pub fn global_in(&self, ns: &Rc<Namespace>, name: &str) -> Option<Val> {
        ns.get(name)
    }

    /// `Weak` handle to the cell backing top-level binding `name`, or
    /// `None` if it isn't bound. Diagnostic sibling of
    /// [`Vm::store_weak`]: it lets a caller observe whether a binding's
    /// cell outlives this Vm — the ADR-015 property — without holding
    /// the cell (or the table) alive and thereby changing the answer.
    pub fn global_cell_weak(&self, name: &str) -> Option<Weak<RefCell<Val>>> {
        self.globals.cell(name).map(|c| Rc::downgrade(&c))
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
        let root = Rc::clone(&self.globals);
        self.register_prim_in(&root, name, arity, f);
    }

    /// [`Vm::register_prim`] into a specific namespace, so a pack's prims
    /// are private to it unless exported.
    pub fn register_prim_in<F>(
        &mut self,
        ns: &Rc<Namespace>,
        name: &'static str,
        arity: val::Arity,
        f: F,
    ) where
        F: Fn(&[Val]) -> Result<Val, String> + 'static,
    {
        ns.define(
            name.into(),
            Val::Prim {
                name,
                arity,
                f: Rc::new(f),
            },
        );
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
    pub fn eval_str(&mut self, src: &str) -> Result<Val, LispErr> {
        let forms = parse::read_many(src)?;
        self.eval_datums(&forms)
    }

    /// Evaluate already-read top-level forms. Same semantics as
    /// [`Vm::eval_str`] — which is just `read_many` plus this — for
    /// callers that hold `Datum`s rather than source text.
    ///
    /// This is the entry point for `macros::MacroVm`, whose whole job
    /// is to hand the engine datums it has already expanded (ADR-034).
    /// Without it, a macro host's only route back into the Vm was to
    /// re-serialize its expansion to source and make the reader parse
    /// it a second time.
    pub fn eval_datums(&mut self, forms: &[Datum]) -> Result<Val, LispErr> {
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
        // Driving a `Session` rather than a private copy of the same loop
        // keeps one implementation of define pre-passes, per-form budgets
        // and rollback. `u64::MAX` as the slice means it never pauses, so
        // the `Paused` arm is unreachable — but `unreachable!` here would
        // be a panic reachable from a host, so it degrades to the budget
        // error instead.
        let mut session = self.start_datums(forms)?;
        match self.resume(&mut session, u64::MAX)? {
            Progress::Done(v) => Ok(v),
            Progress::Paused => Err(LispErr::new("execution exceeded step budget")),
        }
    }

    /// Begin a resumable top-level evaluation of `src` without running
    /// any of it. See [`Vm::resume`].
    pub fn start(&mut self, src: &str) -> Result<Session, LispErr> {
        let forms = parse::read_many(src)?;
        self.start_datums(&forms)
    }

    /// [`Vm::eval_str`] evaluated inside `ns` (ADR-042). `define`s land
    /// there and names resolve from there outward to the root, so a pack
    /// sees its own private vocabulary.
    pub fn eval_str_in(&mut self, ns: &Rc<Namespace>, src: &str) -> Result<Val, LispErr> {
        let forms = parse::read_many(src)?;
        self.eval_datums_in(ns, &forms)
    }

    /// [`Vm::eval_datums`] evaluated inside `ns`.
    pub fn eval_datums_in(&mut self, ns: &Rc<Namespace>, forms: &[Datum]) -> Result<Val, LispErr> {
        let mut session = self.start_datums_in(ns, forms)?;
        match self.resume(&mut session, u64::MAX)? {
            Progress::Done(v) => Ok(v),
            Progress::Paused => Err(LispErr::new("execution exceeded step budget")),
        }
    }

    /// [`Vm::start`] for callers that already hold read forms — the
    /// `eval_datums` counterpart of `start`.
    ///
    /// Reading and the define pre-pass both happen here, before any
    /// evaluation, so a syntax error surfaces from `start` rather than
    /// from the first `resume`.
    pub fn start_datums(&mut self, forms: &[Datum]) -> Result<Session, LispErr> {
        let root = Rc::clone(&self.globals);
        self.start_datums_in(&root, forms)
    }

    /// [`Vm::start_datums`] targeting `ns`: `define`s land there, and
    /// top-level expressions resolve names from there outward.
    pub fn start_datums_in(
        &mut self,
        ns: &Rc<Namespace>,
        forms: &[Datum],
    ) -> Result<Session, LispErr> {
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
        let saved_globals = ns.snapshot();

        // Pre-pass: allocate placeholder cells in `globals` for every
        // top-level `(define name body)` in this batch. Bodies that
        // reference any sibling-define's name (or their own) resolve
        // through `Env::lookup`'s globals fallback to these cells.
        // Same name defined twice in one batch: the second
        // pre-allocation overwrites the first cell; both bodies then
        // write to the second cell. The first cell becomes garbage.
        let mut define_cells: HashMap<String, Rc<RefCell<Val>>> = HashMap::new();
        let mut pre_pass = || -> Result<(), LispErr> {
            for datum in forms {
                if let Some(name) = extract_define_name(datum)? {
                    let cell = Rc::new(RefCell::new(Val::Bool(false)));
                    ns.bind_cell(name.clone(), cell.clone());
                    define_cells.insert(name.to_string(), cell);
                }
            }
            Ok(())
        };
        if let Err(e) = pre_pass() {
            // A malformed define in the batch has to undo the cells the
            // pre-pass already installed for its well-formed siblings.
            ns.restore(saved_globals);
            return Err(e);
        }

        Ok(Session {
            forms: forms.to_vec(),
            next: 0,
            define_cells,
            machine: None,
            pending_define: None,
            form_steps: 0,
            last: Val::Bool(true),
            // The env every form in this batch evaluates against. A
            // pack's prelude therefore resolves its own helpers, and the
            // closures it creates capture this env and keep doing so
            // forever after (ADR-042).
            env: self.env.with_namespace(ns),
            ns: Rc::clone(ns),
            saved_globals: Some(saved_globals),
        })
    }

    /// Advance `session` by at most `slice` CEK steps, returning
    /// [`Progress::Paused`] if it runs out with work left.
    ///
    /// Two independent limits apply, and conflating them is the mistake to
    /// avoid: `slice` is *cooperative* — how much work the host is willing
    /// to do before it wants control back — while the Vm's
    /// [`step_budget`](Vm::set_step_budget) is a *safety net* against a
    /// form that never terminates, and is still enforced per form. A host
    /// pumping 50k-step slices to keep a frame budget will pause many
    /// times over one form without ever tripping the budget.
    ///
    /// On error the globals table is rolled back exactly as
    /// [`Vm::eval_datums`] does. A session the host simply stops resuming
    /// is *not* rolled back — abandoning is a decision, not a failure, and
    /// the effects already applied stand the same way a failed batch's
    /// prim effects do.
    pub fn resume(&mut self, session: &mut Session, slice: u64) -> Result<Progress, LispErr> {
        let result = self.resume_inner(session, slice);
        if result.is_err()
            && let Some(saved) = session.saved_globals.take()
        {
            session.ns.restore(saved);
        }
        result
    }

    fn resume_inner(&mut self, session: &mut Session, slice: u64) -> Result<Progress, LispErr> {
        let mut left = slice;
        loop {
            // Start the next form if we're between forms.
            if session.machine.is_none() {
                let Some(datum) = session.forms.get(session.next) else {
                    // Every form done: the batch's value is the last
                    // expression's, or `#t` if it was all defines.
                    session.saved_globals = None;
                    return Ok(Progress::Done(session.last.clone()));
                };
                session.next += 1;
                session.form_steps = 0;
                let (expr, define) = match extract_define_name(datum)? {
                    Some(name) => {
                        let items = datum
                            .as_list()
                            .expect("extract_define_name returned Some, so datum is a List");
                        (parse::compile(&items[2])?, Some(name))
                    }
                    None => (parse::compile(datum)?, None),
                };
                session.pending_define = define;
                session.machine = Some(Machine::new(expr, session.env.clone()));
            }

            let machine = session
                .machine
                .as_mut()
                .expect("just ensured a machine is present");

            // The form's own budget and the caller's slice are both
            // ceilings; run to whichever comes first and tell them apart
            // by which one we hit.
            let form_left = self.step_budget.saturating_sub(session.form_steps);
            if form_left == 0 {
                return Err(LispErr::new("execution exceeded step budget"));
            }
            let chunk = left.min(form_left);
            let before = machine.steps();
            let progress = machine.run(chunk);
            let taken = machine.steps() - before;
            session.form_steps += taken;
            left -= taken;

            match progress? {
                Progress::Done(v) => {
                    session.machine = None;
                    match session.pending_define.take() {
                        // A define's value goes to the cell the pre-pass
                        // allocated, and doesn't become the batch value.
                        Some(name) => session
                            .define_cells
                            .get(name.as_ref())
                            .expect("pre-pass should have allocated a cell for this define")
                            .borrow_mut()
                            .clone_from(&v),
                        None => session.last = v,
                    }
                }
                Progress::Paused => {
                    // Distinguish "the host's slice ran out" from "this
                    // form blew its budget": only the latter is an error.
                    if session.form_steps >= self.step_budget {
                        return Err(LispErr::new("execution exceeded step budget"));
                    }
                    return Ok(Progress::Paused);
                }
            }

            if left == 0 && session.next < session.forms.len() {
                return Ok(Progress::Paused);
            }
        }
    }

    /// Apply a callable `Val` (closure or prim) to `args`. Exposed for
    /// the `macros` crate to call macro closures; hosts generally don't
    /// need this — top-level evaluation goes through [`Vm::eval_str`].
    pub fn call_value(&self, f: &Val, args: Vec<Val>) -> Result<Val, LispErr> {
        let mut app: Vec<Rc<Expr>> = vec![Rc::new(Expr::Quote(Rc::new(f.clone())))];
        for a in args {
            app.push(Rc::new(Expr::Quote(Rc::new(a))));
        }
        // No span: this application has no source text. Errors from it
        // report unpositioned, which is correct — pointing at a caller's
        // line for a form the caller never wrote would be worse than
        // saying nothing.
        run_bounded(
            Expr::App(app.into(), None),
            self.env.clone(),
            self.step_budget,
        )
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// A top-level evaluation in progress: the batch of forms, which one is
/// running, and the [`Machine`] running it.
///
/// Created by [`Vm::start`] and advanced by [`Vm::resume`]. It holds no
/// borrow of the `Vm`, so a host can park one in a struct field between
/// event-loop turns — which is the entire point, since that's how you
/// evaluate on a thread you aren't allowed to block.
///
/// A `Session` owns its forms and the define cells the pre-pass allocated,
/// but not the bindings themselves: those live in the `Vm`, so effects
/// from completed forms are visible immediately, mid-session.
pub struct Session {
    forms: Vec<Datum>,
    /// Index of the next form to start; `forms.len()` once all are begun.
    next: usize,
    define_cells: HashMap<String, Rc<RefCell<Val>>>,
    /// The form currently in flight, or `None` between forms.
    machine: Option<Machine>,
    /// Set when the in-flight form is a `define`, naming the cell its
    /// value belongs in.
    pending_define: Option<Sym>,
    /// Steps spent on the in-flight form, for the per-form budget. Reset
    /// per form, unlike `Machine::steps`.
    form_steps: u64,
    last: Val,
    /// The environment this batch evaluates against — the Vm's root env
    /// rebased onto the target namespace (ADR-042).
    env: Env,
    /// The namespace `define`s land in, and the one rolled back on
    /// failure. Held strong: a session outlives no Vm, and the `Vm` keeps
    /// the authoritative reference either way.
    ns: Rc<Namespace>,
    /// Pre-call table of `ns`, dropped once the batch completes. `Some`
    /// means a rollback is still owed if something fails.
    saved_globals: Option<HashMap<Sym, Rc<RefCell<Val>>>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("forms_done", &self.forms_done())
            .field("forms_total", &self.forms_total())
            .field("machine", &self.machine)
            .finish()
    }
}

impl Session {
    /// Forms in the batch that have finished.
    pub fn forms_done(&self) -> usize {
        if self.machine.is_some() {
            self.next - 1
        } else {
            self.next
        }
    }

    pub fn forms_total(&self) -> usize {
        self.forms.len()
    }

    /// The machine running the current form, for introspection while
    /// paused — depth, position, backtrace. `None` between forms.
    pub fn machine(&self) -> Option<&Machine> {
        self.machine.as_ref()
    }

    /// Value of the last expression to finish. Meaningful before the
    /// batch completes: a host can show intermediate results.
    pub fn last_value(&self) -> &Val {
        &self.last
    }
}

/// Inspect a datum and, if it's a well-formed `(define name body)`,
/// return the bound name. Used by `eval_str`'s pre-pass (to allocate
/// placeholder cells) and `try_register_define` (to validate the
/// form structure and find its cell).
fn extract_define_name(d: &Datum) -> Result<Option<Sym>, LispErr> {
    let items = match d.as_list() {
        Some(items) => items,
        None => return Ok(None),
    };
    match items.first().and_then(Datum::as_sym) {
        Some(s) if &**s == "define" => {}
        _ => return Ok(None),
    }
    if items.len() != 3 {
        return Err(LispErr::maybe_at(
            "define: expected (define name value)",
            d.span,
        ));
    }
    match items[1].as_sym() {
        Some(s) => Ok(Some(s.clone())),
        None => Err(LispErr::maybe_at(
            "define: name must be a symbol",
            items[1].span,
        )),
    }
}
