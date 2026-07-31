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
//!
//! ## The ownership invariant, and what enforces it
//!
//! A `Vm` is the sole strong owner of its globals table and its store, so
//! dropping it releases every binding, closure and frame slot. That has
//! been the rule since ADR-036 — and it was reopened twice in a row, by
//! ADR-042's `root()` and ADR-043's `env_in()`, because privacy was
//! enforced at the *field* while the invariant is a claim about every
//! line that returns.
//!
//! Two things enforce it now (ADR-044), because one wasn't enough:
//!
//! 1. **Accessors.** `Namespace` and `Store` are crate-private, so no
//!    public signature can mention them and a new accessor handing one
//!    out does not compile. The `deny` below is what makes that bite:
//!    leaking a private type through a public signature is only a
//!    *warning* by default, and a warning is not an invariant.
//! 2. **Everything else.** The lint checks *signatures*. It cannot see a
//!    public type that stores a container in a private field, which is
//!    what [`Session`] did — parking one kept a whole globals table
//!    alive past its `Vm`, and no lint was ever going to notice. So
//!    `Vm`'s `Drop` checks the property itself, at the one moment it is
//!    decidable: if a strong reference to the store or any namespace
//!    outlives the `Vm`, the counts are wrong and a `debug_assert`
//!    fires. `mod ownership_guard` pins both directions.
#![deny(private_interfaces, private_bounds)]

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

pub use env::Env;
pub use error::{LispErr, Span, render_span};
pub use expr::{Expr, Sym};
pub use ns::NsHandle;

// Crate-internal, and deliberately *not* re-exported (ADR-044).
// `Namespace` and `Store` are the two containers the `Vm` must be sole
// strong owner of. Keeping them unnameable from outside means the
// compiler rejects any public signature that mentions them — so an
// accessor that would hand one out cannot be written, rather than being
// caught after the fact. `Globals` is `Rc<Namespace>` and goes with it.
use env::Globals;
use ns::Namespace;
use store::Store;

pub use parse::{Datum, DatumKind};
pub use step::{Machine, Progress, Step, run, run_bounded};
pub use store::Addr;
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
    /// [`Vm::store_probe`].
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
    /// Rollback snapshots for in-flight [`Session`]s, keyed by session id.
    ///
    /// These live here rather than in the `Session` because the snapshot
    /// is a clone of a whole namespace table — every binding cell in it —
    /// and a `Session` deliberately holds no borrow of the `Vm`
    /// (ADR-040), so a host can park one indefinitely. Owning them in the
    /// `Session` meant a parked session kept every binding and closure
    /// alive after its `Vm` dropped: the ADR-036 invariant, escaping
    /// through a private field of a public type where no lint could see
    /// it (ADR-044 amendment).
    rollbacks: HashMap<u64, Rollback>,
    next_session: u64,
}

impl Drop for Vm {
    /// The catch-all half of ADR-044's enforcement, and the half that
    /// would have caught the `Session` escape.
    ///
    /// Making `Namespace` and `Store` crate-private stops any *signature*
    /// from handing one out, which is the failure mode that happened
    /// twice. It cannot see a public type that stores one in a private
    /// field — `Session` did exactly that, and no lint was ever going to
    /// notice. This checks the property itself, at the one moment it is
    /// decidable: if anything still holds a strong reference when the
    /// `Vm` goes, the counts are wrong.
    ///
    /// `debug_assert` on purpose. It runs in every test and every debug
    /// build and costs nothing shipped; a retained container is a
    /// development mistake, not a condition to handle at runtime.
    fn drop(&mut self) {
        // Never turn someone else's failure into an abort: a panicking
        // test drops its `Vm` mid-unwind, and a second panic from here
        // would replace the real diagnostic with `panic in a destructor`.
        if std::thread::panicking() {
            return;
        }
        debug_assert_eq!(
            Rc::strong_count(&self.store),
            1,
            "the store outlived its Vm: something holds a strong reference. \
             Env and Frame reach it through a Weak, so this is a new escape \
             — see ADR-044."
        );
        // Each pack namespace holds a strong reference to its parent, so
        // the root's expected count is one for this `Vm` plus one per pack.
        debug_assert_eq!(
            Rc::strong_count(&self.globals),
            1 + self.packs.len(),
            "the root namespace outlived its Vm: something holds a strong \
             reference beyond this Vm and its {} pack(s) — see ADR-044.",
            self.packs.len()
        );
        for (name, ns) in &self.packs {
            debug_assert_eq!(
                Rc::strong_count(ns),
                1,
                "pack namespace `{name}` outlived its Vm: something holds a \
                 strong reference — see ADR-044."
            );
        }
    }
}

/// A namespace table as it stood before a batch began, owed back if the
/// batch fails. Held by the `Vm`, not the `Session` — see [`Vm::rollbacks`].
struct Rollback {
    ns: NsHandle,
    table: HashMap<Sym, Rc<RefCell<Val>>>,
    /// Liveness signal for the owning `Session`. A host that abandons a
    /// session rather than resuming it to completion leaves its entry
    /// behind; dead tokens are pruned when the next session starts, so
    /// the garbage is bounded by "sessions abandoned since the last
    /// `start`" rather than growing forever.
    token: Weak<()>,
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
            rollbacks: HashMap::new(),
            next_session: 0,
        }
    }

    /// Handle to the root namespace: builtins, user top-level `define`s,
    /// and whatever the installed packs have exported (ADR-042).
    ///
    /// A handle, not the `Rc`. Returning the `Rc` would let a caller keep
    /// the whole globals table alive past this `Vm` — the invariant
    /// ADR-036 made the field private to protect.
    pub fn root(&self) -> NsHandle {
        NsHandle::root()
    }

    /// Resolve a handle to the table it names. `None` if the handle came
    /// from a different `Vm`, or names a pack this one never created.
    fn resolve(&self, h: &NsHandle) -> Option<Rc<Namespace>> {
        if h.is_root() {
            return Some(Rc::clone(&self.globals));
        }
        self.packs.get(h.name()).map(Rc::clone)
    }

    fn resolve_or_err(&self, h: &NsHandle) -> Result<Rc<Namespace>, LispErr> {
        self.resolve(h)
            .ok_or_else(|| LispErr::new(format!("unknown namespace: `{h}`")))
    }

    /// Get or create the namespace named `name`, a child of the root.
    ///
    /// A DSL pack calls this once at install time and evaluates its
    /// prelude there. Its internal helpers stay private to it, so two
    /// packs can both define `thread` without either noticing — which
    /// they already both did, identically, until this existed.
    pub fn namespace(&mut self, name: &str) -> NsHandle {
        if name == ns::ROOT {
            return NsHandle::root();
        }
        if !self.packs.contains_key(name) {
            let ns = Namespace::child(name, &self.globals);
            self.packs.insert(name.to_string(), ns);
        }
        NsHandle::new(name)
    }

    /// Handle to the namespace named `name`, if it has been created.
    pub fn find_namespace(&self, name: &str) -> Option<NsHandle> {
        if name == ns::ROOT {
            return Some(NsHandle::root());
        }
        self.packs.contains_key(name).then(|| NsHandle::new(name))
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
    pub fn export(&mut self, ns: &NsHandle, names: &[&str]) -> Result<(), LispErr> {
        let source = self.resolve_or_err(ns)?;
        // Check the whole list before publishing any of it. Exporting
        // name-by-name meant a collision on the last name still left the
        // earlier ones visible in the root, with the call reporting
        // failure — a half-installed pack.
        for name in names {
            source
                .can_export(&self.globals, name)
                .map_err(LispErr::new)?;
        }
        for name in names {
            source.export(&self.globals, name).map_err(LispErr::new)?;
        }
        Ok(())
    }

    /// A non-owning probe of the Vm's store, for ADR-023's
    /// `letrec_does_not_leak` diagnostic and the ADR-033 reclamation
    /// tests: it answers whether the arena is still alive and how full
    /// it is, and it cannot be turned into a reference that keeps it so.
    ///
    /// This used to hand out the `Weak<Store>` itself, which a caller
    /// could upgrade and hold — rooting the whole arena past its `Vm`.
    /// The name said `weak` and the type was honest about it; that
    /// still left the escape one `.upgrade()` away (ADR-044).
    pub fn store_probe(&self) -> StoreProbe {
        StoreProbe(Rc::downgrade(&self.store))
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
    pub fn global_in(&self, ns: &NsHandle, name: &str) -> Option<Val> {
        self.resolve(ns)?.get(name)
    }

    /// `Weak` handle to the cell backing top-level binding `name`, or
    /// `None` if it isn't bound. Diagnostic sibling of
    /// [`Vm::store_probe`]: it lets a caller observe whether a binding's
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

    /// The environment for `ns` — what a form evaluated there resolves
    /// against, and what a closure created there captures.
    ///
    /// [`Vm::env`] is the *root* env, which is the wrong one for anything
    /// belonging to a pack: a closure capturing it resolves names from
    /// root outward and never sees the pack's private vocabulary. The
    /// `macros` crate needs this for macro closures, which are created at
    /// `defmacro` time rather than by the evaluator and so don't get the
    /// batch env `start_datums_in` builds (ADR-043).
    pub fn env_in(&self, ns: &NsHandle) -> Result<Env, LispErr> {
        Ok(self.env.with_namespace(&self.resolve_or_err(ns)?))
    }

    /// The namespace chain for `ns`, innermost first and ending at the
    /// root — the order a name resolves in.
    ///
    /// Exposed so a caller keeping its own per-namespace table can mirror
    /// the engine's topology rather than assume it. The `macros` crate
    /// keeps exactly such a table; hardcoding "the pack, then root" would
    /// be correct only for as long as every pack is a direct child of the
    /// root, and would fail silently the day one isn't (ADR-043).
    pub fn ns_chain(&self, ns: &NsHandle) -> Result<Vec<NsHandle>, LispErr> {
        let mut out = Vec::new();
        let mut cur = Some(self.resolve_or_err(ns)?);
        while let Some(n) = cur {
            out.push(NsHandle::new(n.name()));
            cur = n.parent();
        }
        Ok(out)
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
        self.register_prim_in(&NsHandle::root(), name, arity, f);
    }

    /// [`Vm::register_prim`] into a specific namespace, so a pack's prims
    /// are private to it unless exported.
    pub fn register_prim_in<F>(
        &mut self,
        ns: &NsHandle,
        name: &'static str,
        arity: val::Arity,
        f: F,
    ) where
        F: Fn(&[Val]) -> Result<Val, String> + 'static,
    {
        let Some(target) = self.resolve(ns) else {
            // A handle from another Vm. Registering into nothing would be
            // a silent no-op, and this is host setup code, so say so.
            panic!("register_prim_in: unknown namespace `{ns}`");
        };
        let ns = target;
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
    pub fn eval_str_in(&mut self, ns: &NsHandle, src: &str) -> Result<Val, LispErr> {
        let forms = parse::read_many(src)?;
        self.eval_datums_in(ns, &forms)
    }

    /// [`Vm::eval_datums`] evaluated inside `ns`.
    pub fn eval_datums_in(&mut self, ns: &NsHandle, forms: &[Datum]) -> Result<Val, LispErr> {
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
        self.start_datums_in(&NsHandle::root(), forms)
    }

    /// [`Vm::start_datums`] targeting `ns`: `define`s land there, and
    /// top-level expressions resolve names from there outward.
    pub fn start_datums_in(
        &mut self,
        ns_handle: &NsHandle,
        forms: &[Datum],
    ) -> Result<Session, LispErr> {
        let ns = &self.resolve_or_err(ns_handle)?;
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

        // Prune snapshots whose sessions were abandoned before handing
        // out a new one. Bounded garbage, collected at the only moment
        // we're guaranteed to be here with `&mut self`.
        self.rollbacks.retain(|_, r| r.token.strong_count() > 0);

        let id = self.next_session;
        self.next_session += 1;
        let token = Rc::new(());
        self.rollbacks.insert(
            id,
            Rollback {
                ns: ns_handle.clone(),
                table: saved_globals,
                token: Rc::downgrade(&token),
            },
        );

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
            id,
            _token: token,
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
            && let Some(rollback) = self.rollbacks.remove(&session.id)
            && let Some(ns) = self.resolve(&rollback.ns)
        {
            ns.restore(rollback.table);
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
                    self.rollbacks.remove(&session.id);
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

/// A non-owning window onto a `Vm`'s store, handed out by
/// [`Vm::store_probe`].
///
/// Every method answers from a momentary upgrade and drops it again, so
/// holding a probe forever still cannot keep the arena alive. `None`
/// means the owning `Vm` is gone — which is itself the thing most of
/// these diagnostics are asserting.
///
/// The type exists because the invariant it protects is otherwise
/// unenforceable: `Store` is crate-private, so this is the only shape in
/// which anything store-related can cross the crate boundary at all
/// (ADR-044).
pub struct StoreProbe(Weak<Store>);

impl StoreProbe {
    /// Whether the store — and therefore the `Vm` that owns it — is
    /// still alive.
    pub fn is_alive(&self) -> bool {
        self.0.upgrade().is_some()
    }

    /// Live slots, or `None` once the `Vm` has dropped. Slots are
    /// recycled (ADR-033), so this is occupancy rather than a total.
    pub fn len(&self) -> Option<usize> {
        self.0.upgrade().map(|s| s.len())
    }

    /// Whether the store holds no live slots. `None` once the `Vm` has
    /// dropped — which is *not* the same as empty, and the reason this
    /// returns an `Option` rather than defaulting to `true`.
    pub fn is_empty(&self) -> Option<bool> {
        self.0.upgrade().map(|s| s.is_empty())
    }

    /// High-water mark: slots ever allocated, including recycled ones.
    pub fn slots(&self) -> Option<usize> {
        self.0.upgrade().map(|s| s.slots())
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
    // The target namespace used to live here as an `Rc<Namespace>`,
    // which is what let a parked session keep an entire globals table
    // alive past its `Vm` (ADR-044 amendment). It moved to the `Vm`'s
    // rollback record — which needs it anyway — rather than being
    // downgraded to a handle nothing read.
    /// Identifies this session's rollback snapshot in [`Vm::rollbacks`].
    /// Presence there means a rollback is still owed.
    id: u64,
    /// Dropped with the session; the `Vm` watches it through a `Weak` to
    /// prune the snapshots of sessions that were abandoned.
    _token: Rc<()>,
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

/// Tests for ADR-044's *enforcement*, rather than for any behavior.
///
/// The compile-time half — `Namespace` and `Store` being crate-private,
/// plus `deny(private_interfaces)` — can't be pinned from here; a test
/// that a signature fails to compile needs `trybuild`, which `lisp` won't
/// take a dependency on (ADR-002). These pin the runtime half, which is
/// the part that catches what the lint structurally cannot see: a public
/// type storing a container in a private field, which is exactly how
/// `Session` escaped.
///
/// They live in the crate because reproducing the escape requires
/// touching the private fields. That is the point: from outside, these
/// leaks are unwritable.
#[cfg(test)]
mod ownership_guard {
    use super::*;

    #[test]
    #[should_panic(expected = "root namespace outlived its Vm")]
    fn a_retained_root_trips_the_guard() {
        let vm = Vm::new();
        let _leaked = Rc::clone(&vm.globals);
        drop(vm);
    }

    #[test]
    #[should_panic(expected = "store outlived its Vm")]
    fn a_retained_store_trips_the_guard() {
        let vm = Vm::new();
        let _leaked = Rc::clone(&vm.store);
        drop(vm);
    }

    #[test]
    #[should_panic(expected = "pack namespace `p` outlived its Vm")]
    fn a_retained_pack_trips_the_guard() {
        let mut vm = Vm::new();
        let h = vm.namespace("p");
        let _leaked = vm.resolve(&h).unwrap();
        drop(vm);
    }

    #[test]
    fn a_clean_vm_with_packs_and_sessions_does_not() {
        // The guard has to be quiet on the ordinary case, or it is just
        // a flaky test. Packs raise the root's expected count by one
        // each, and a completed session must leave nothing behind.
        let mut vm = Vm::new();
        let a = vm.namespace("a");
        let b = vm.namespace("b");
        vm.eval_str_in(&a, "(define x 1)").unwrap();
        vm.eval_str_in(&b, "(define x 2)").unwrap();
        let mut s = vm.start("(+ 1 2)").unwrap();
        vm.resume(&mut s, u64::MAX).unwrap();
        drop(s);
        drop(vm);
    }
}
