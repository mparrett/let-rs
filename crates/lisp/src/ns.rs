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

/// Reserved name of the root namespace. A pack cannot claim it.
pub(crate) const ROOT: &str = "root";

/// A binding cell. Shared rather than copied when a name is exported, so
/// `set!` through either name writes the same slot — which is what lets
/// the mana counter live in the spells pack and still be read from root.
pub type Cell = Rc<RefCell<Val>>;

/// A reference to a namespace, safe to hand outside the engine.
///
/// Deliberately *not* an `Rc<Namespace>`. Handing out the `Rc` let a
/// caller keep the entire globals table — and every closure and binding
/// cell in it — alive after its `Vm` was dropped, which is exactly the
/// sole-strong-owner invariant ADR-036 made `Vm::globals` private to
/// protect. A handle is just a name; the `Vm` owns every `Rc` and
/// resolves handles internally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NsHandle(Rc<str>);

impl NsHandle {
    pub(crate) fn new(name: &str) -> NsHandle {
        NsHandle(name.into())
    }

    /// The root namespace: builtins, user top-level `define`s, and
    /// whatever the packs have exported.
    pub fn root() -> NsHandle {
        NsHandle(ROOT.into())
    }

    pub fn name(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_root(&self) -> bool {
        &*self.0 == ROOT
    }
}

impl std::fmt::Display for NsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct Namespace {
    name: Rc<str>,
    table: RefCell<HashMap<Sym, Cell>>,
    /// `None` for the root. A pack's namespace parents to the root, so
    /// builtins resolve without every pack re-registering them.
    parent: Option<Rc<Namespace>>,
    /// For a target of `export`: which pack published each name here.
    ///
    /// Provenance rather than cell identity is what distinguishes the two
    /// cases that look alike. Re-running a pack's prelude allocates fresh
    /// cells for all of its defines, so a *reinstall* presents a
    /// different cell for a name this same pack already exported — which
    /// a cell-identity check reports as a collision, making every
    /// installer panic on its second call. A genuine collision is a
    /// different *pack* claiming the name, and that is what this records.
    exported_by: RefCell<HashMap<Sym, Rc<str>>>,
}

impl Namespace {
    pub(crate) fn root() -> Rc<Namespace> {
        Rc::new(Namespace {
            name: ROOT.into(),
            table: RefCell::new(HashMap::new()),
            parent: None,
            exported_by: RefCell::new(HashMap::new()),
        })
    }

    /// A child of `parent`, for one DSL pack.
    pub(crate) fn child(name: &str, parent: &Rc<Namespace>) -> Rc<Namespace> {
        Rc::new(Namespace {
            name: name.into(),
            table: RefCell::new(HashMap::new()),
            parent: Some(Rc::clone(parent)),
            exported_by: RefCell::new(HashMap::new()),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The cell bound to `name`, searching this table then outward.
    pub(crate) fn cell(&self, name: &str) -> Option<Cell> {
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
    pub(crate) fn define(&self, name: Sym, val: Val) {
        self.table
            .borrow_mut()
            .insert(name, Rc::new(RefCell::new(val)));
    }

    /// Bind `name` to an existing cell — the aliasing that `export` uses.
    pub(crate) fn bind_cell(&self, name: Sym, cell: Cell) {
        self.table.borrow_mut().insert(name, cell);
    }

    /// Whether *this* table binds `name`, ignoring the parent chain.
    pub fn defines_locally(&self, name: &str) -> bool {
        self.table.borrow().contains_key(name)
    }

    /// Write through the cell bound to `name`, wherever in the chain it
    /// lives. `set!` mutates an existing binding rather than creating
    /// one, so it walks outward exactly like lookup does.
    pub(crate) fn set(&self, name: &str, val: Val) -> Result<(), String> {
        match self.cell(name) {
            Some(c) => {
                *c.borrow_mut() = val;
                Ok(())
            }
            None => Err(format!("set!: unbound variable: {name}")),
        }
    }

    /// Would exporting `name` into `target` succeed? Separated from the
    /// doing so a multi-name export can check the whole list before
    /// mutating anything — otherwise a collision on the last name leaves
    /// the earlier ones published even though the call reports failure.
    pub(crate) fn can_export(&self, target: &Namespace, name: &str) -> Result<(), String> {
        if !self.table.borrow().contains_key(name) {
            return Err(format!(
                "{}: cannot export `{name}`, which it does not define",
                self.name
            ));
        }
        match target.exported_by.borrow().get(name) {
            // Same pack re-exporting: a reinstall. Rebinding is the
            // point, not an error.
            Some(owner) if **owner == *self.name => Ok(()),
            Some(owner) => Err(format!(
                "`{name}` is already exported into `{}` by `{owner}`; \
                 `{}` cannot export it too",
                target.name, self.name
            )),
            // Bound in the target but not by an export — a builtin, or a
            // user `define`. Refuse rather than silently replace it.
            None if target.table.borrow().contains_key(name) => Err(format!(
                "`{name}` is already bound in `{}`; `{}` cannot export over it",
                target.name, self.name
            )),
            None => Ok(()),
        }
    }

    /// Publish `name` into `target`, sharing the cell rather than copying
    /// the value, and recording this namespace as its owner.
    ///
    /// Call [`Namespace::can_export`] for every name first; this assumes
    /// the check passed.
    pub(crate) fn export(&self, target: &Namespace, name: &str) -> Result<(), String> {
        self.can_export(target, name)?;
        let cell = self
            .table
            .borrow()
            .get(name)
            .map(Rc::clone)
            .expect("can_export confirmed the binding exists");
        target.bind_cell(name.into(), cell);
        target
            .exported_by
            .borrow_mut()
            .insert(name.into(), Rc::clone(&self.name));
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
