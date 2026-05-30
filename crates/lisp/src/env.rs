use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::expr::Sym;
use crate::val::Val;

/// Shared table of top-level bindings, owned by `Vm`. Closures see it
/// via `Env::globals` as a `Weak` back-edge so the cycle that would
/// otherwise form — globals slot → closure → captured env → globals —
/// stays open. See ADR-015 / issue_2.
pub type Globals = Rc<RefCell<HashMap<Sym, Rc<RefCell<Val>>>>>;

/// Immutable, structurally-shared linked frames for lexical bindings
/// (`let`, `letrec`, closure params). Each slot is an `Rc<RefCell<Val>>`
/// so `letrec` can allocate a placeholder frame, evaluate inits in it
/// (closures capture the cell), and patch later. For non-recursive
/// bindings the mutability is invisible — `extend` creates a fresh
/// cell and nothing ever writes to it.
///
/// `globals` is a `Weak` reference to the Vm's top-level table; any
/// name not found in the frame chain falls back to it. Storing it as
/// `Weak` keeps top-level closures from rooting their own globals
/// table — dropping the Vm releases the table, which releases every
/// closure stored in it.
#[derive(Clone)]
pub struct Env {
    frame: Option<Rc<Frame>>,
    globals: Weak<RefCell<HashMap<Sym, Rc<RefCell<Val>>>>>,
}

struct Frame {
    name: Sym,
    slot: Rc<RefCell<Val>>,
    parent: Option<Rc<Frame>>,
}

impl Env {
    /// Env with no frames and no globals. Lookups against it can only
    /// find what's in the (empty) frame chain. Used by tests that build
    /// envs in isolation; production code goes through `with_globals`.
    pub fn empty() -> Self {
        Env {
            frame: None,
            globals: Weak::new(),
        }
    }

    /// Env with no frames but a live back-edge to a globals table. The
    /// `Weak` is upgraded on lookup miss; while the Vm holds the strong
    /// ref it always succeeds.
    pub fn with_globals(globals: &Globals) -> Self {
        Env {
            frame: None,
            globals: Rc::downgrade(globals),
        }
    }

    pub fn extend(&self, name: Sym, val: Val) -> Env {
        self.extend_slot(name, Rc::new(RefCell::new(val)))
    }

    pub fn extend_slot(&self, name: Sym, slot: Rc<RefCell<Val>>) -> Env {
        Env {
            frame: Some(Rc::new(Frame {
                name,
                slot,
                parent: self.frame.clone(),
            })),
            globals: self.globals.clone(),
        }
    }

    pub fn extend_many<I>(&self, bindings: I) -> Env
    where
        I: IntoIterator<Item = (Sym, Val)>,
    {
        let mut env = self.clone();
        for (n, v) in bindings {
            env = env.extend(n, v);
        }
        env
    }

    /// Allocate a placeholder slot and bind `name` to it. Returns the cell so
    /// the caller can patch it once the init expression has been evaluated.
    /// The placeholder value should never be read before patching — if it is,
    /// it leaks out as `#f`, which makes the bug observable rather than UB.
    pub fn extend_placeholder(&self, name: Sym) -> (Env, Rc<RefCell<Val>>) {
        let slot = Rc::new(RefCell::new(Val::Bool(false)));
        (self.extend_slot(name, slot.clone()), slot)
    }

    pub fn lookup(&self, name: &str) -> Option<Val> {
        let mut cur = self.frame.as_deref();
        while let Some(f) = cur {
            if &*f.name == name {
                return Some(f.slot.borrow().clone());
            }
            cur = f.parent.as_deref();
        }
        // Frame miss — fall through to the Vm's globals. The Weak
        // upgrade only fails after the owning Vm has been dropped, in
        // which case any surviving closure was definitionally orphaned.
        let globals = self.globals.upgrade()?;
        let table = globals.borrow();
        table.get(name).map(|slot| slot.borrow().clone())
    }
}
