use std::cell::RefCell;
use std::rc::Rc;

use crate::expr::Sym;
use crate::val::Val;

/// Immutable, structurally-shared linked frames.
/// Each slot is an `Rc<RefCell<Val>>` so `letrec` can allocate a placeholder
/// frame, evaluate inits in it (closures capture the cell), and patch later.
/// For non-recursive bindings the mutability is invisible — `extend` creates
/// a fresh cell and nothing ever writes to it.
#[derive(Clone)]
pub struct Env(Option<Rc<Frame>>);

struct Frame {
    name: Sym,
    slot: Rc<RefCell<Val>>,
    parent: Option<Rc<Frame>>,
}

impl Env {
    pub fn empty() -> Self {
        Env(None)
    }

    pub fn extend(&self, name: Sym, val: Val) -> Env {
        self.extend_slot(name, Rc::new(RefCell::new(val)))
    }

    pub fn extend_slot(&self, name: Sym, slot: Rc<RefCell<Val>>) -> Env {
        Env(Some(Rc::new(Frame {
            name,
            slot,
            parent: self.0.clone(),
        })))
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
        let mut cur = self.0.as_deref();
        while let Some(f) = cur {
            if &*f.name == name {
                return Some(f.slot.borrow().clone());
            }
            cur = f.parent.as_deref();
        }
        None
    }
}
