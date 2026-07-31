//! Namespaces: a chain of top-level binding tables (ADR-042).
//!
//! Before this, every `(define …)` in the process landed in one table.
//! Two DSL packs both defining `thread` meant whichever installed second
//! won, silently — and because a closure resolves globals at *call* time
//! (ADR-015), even code that captured the earlier binding jumped into the
//! later one on its first recursive call. Shadowing wasn't containable by
//! discipline at the call site.
//!
//! A `Namespace` is a table plus an optional parent. Each pack installs
//! into its own, chained to a shared root that holds the builtins and
//! whatever the packs choose to export. Lookup walks the chain outward;
//! definition always writes to the table it started in.
//!
//! What makes this *lexical* rather than dynamic is that `Env` holds the
//! namespace, and closures capture their `Env`. A spells closure calling
//! `thread` gets spells' `thread` even when a genes closure invoked it,
//! because the resolution follows the closure's own environment rather
//! than whatever is "current" at the moment of the call.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::expr::Sym;
use crate::val::Val;

/// A binding cell. Shared rather than copied when a name is exported, so
/// `set!` through either name writes the same slot — which is what lets
/// the mana counter live in the spells pack and still be read from root.
pub type Cell = Rc<RefCell<Val>>;

pub struct Namespace {
    name: Rc<str>,
    table: RefCell<HashMap<Sym, Cell>>,
    /// `None` for the root. A pack's namespace parents to the root, so
    /// builtins resolve without every pack re-registering them.
    parent: Option<Rc<Namespace>>,
}

impl Namespace {
    pub fn root() -> Rc<Namespace> {
        Rc::new(Namespace {
            name: "root".into(),
            table: RefCell::new(HashMap::new()),
            parent: None,
        })
    }

    /// A child of `parent`, for one DSL pack.
    pub fn child(name: &str, parent: &Rc<Namespace>) -> Rc<Namespace> {
        Rc::new(Namespace {
            name: name.into(),
            table: RefCell::new(HashMap::new()),
            parent: Some(Rc::clone(parent)),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The cell bound to `name`, searching this table then outward.
    pub fn cell(&self, name: &str) -> Option<Cell> {
        if let Some(c) = self.table.borrow().get(name) {
            return Some(Rc::clone(c));
        }
        self.parent.as_ref()?.cell(name)
    }

    pub fn get(&self, name: &str) -> Option<Val> {
        self.cell(name).map(|c| c.borrow().clone())
    }

    /// Bind `name` in *this* table, replacing any binding it already
    /// held. Definition never walks outward: a pack defining a name the
    /// root also has gets its own, which is the whole point.
    pub fn define(&self, name: Sym, val: Val) {
        self.table
            .borrow_mut()
            .insert(name, Rc::new(RefCell::new(val)));
    }

    /// Bind `name` to an existing cell — the aliasing that `export` uses.
    pub fn bind_cell(&self, name: Sym, cell: Cell) {
        self.table.borrow_mut().insert(name, cell);
    }

    /// Whether *this* table binds `name`, ignoring the parent chain.
    pub fn defines_locally(&self, name: &str) -> bool {
        self.table.borrow().contains_key(name)
    }

    /// Write through the cell bound to `name`, wherever in the chain it
    /// lives. `set!` mutates an existing binding rather than creating
    /// one, so it walks outward exactly like lookup does.
    pub fn set(&self, name: &str, val: Val) -> Result<(), String> {
        match self.cell(name) {
            Some(c) => {
                *c.borrow_mut() = val;
                Ok(())
            }
            None => Err(format!("set!: unbound variable: {name}")),
        }
    }

    /// Publish `name` from this namespace into `target`, sharing the
    /// cell rather than copying the value.
    ///
    /// Fails if `target` already binds `name` to a *different* cell —
    /// that's two packs claiming one public name, and the whole reason
    /// this module exists is that the old behavior was to let the second
    /// one win in silence. Re-exporting the same cell is a no-op, so
    /// installing a pack twice is harmless.
    pub fn export(&self, target: &Namespace, name: &str) -> Result<(), String> {
        let Some(cell) = self.table.borrow().get(name).map(Rc::clone) else {
            return Err(format!(
                "{}: cannot export `{name}`, which it does not define",
                self.name
            ));
        };
        if let Some(existing) = target.table.borrow().get(name)
            && !Rc::ptr_eq(existing, &cell)
        {
            return Err(format!(
                "`{name}` is already bound in `{}` by another pack; \
                 `{}` cannot export it too",
                target.name, self.name
            ));
        }
        target.bind_cell(name.into(), cell);
        Ok(())
    }

    /// Snapshot of this table alone, for `eval_datums`' rollback.
    pub(crate) fn snapshot(&self) -> HashMap<Sym, Cell> {
        self.table.borrow().clone()
    }

    pub(crate) fn restore(&self, snap: HashMap<Sym, Cell>) {
        *self.table.borrow_mut() = snap;
    }

    /// Names bound in this table alone, sorted. Diagnostic, and how a
    /// pack's install can report what it is about to export.
    pub fn names(&self) -> Vec<Sym> {
        let mut v: Vec<Sym> = self.table.borrow().keys().cloned().collect();
        v.sort();
        v
    }

    /// Number of names bound in this table alone. Diagnostic.
    pub fn len(&self) -> usize {
        self.table.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
