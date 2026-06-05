use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::expr::Sym;
use crate::store::{Addr, Store};
use crate::val::Val;

/// Shared table of top-level bindings, owned by `Vm`. Closures see it
/// via `Env::globals` as a `Weak` back-edge so the cycle that would
/// otherwise form — globals slot → closure → captured env → globals —
/// stays open. See ADR-015 / issue_2.
///
/// Note: top-level bindings stayed as their own region across the
/// ADR-023 CESK migration. Frame slots moved to the `Store`; globals
/// kept their `Rc<RefCell<Val>>` cells so the ADR-015 Weak back-edge
/// pattern still holds end-to-end without re-derivation.
pub type Globals = Rc<RefCell<HashMap<Sym, Rc<RefCell<Val>>>>>;

/// Immutable, structurally-shared linked frames for lexical bindings
/// (`let`, `letrec`, closure params). Each slot is an `Addr` into the
/// Vm's `Store` (ADR-023). The store holds the value; frames just hold
/// the index. `Addr` is `Copy`, so a closure that captures an env can
/// no longer Rc-reach back to its own letrec cell — the cycle from
/// ADR-021 dissolves by construction.
///
/// `globals` is a `Weak` reference to the Vm's top-level table; any
/// name not found in the frame chain falls back to it. `store` is the
/// matching `Weak` for frame-slot resolution. Both `Weak` keeps any
/// closure from rooting its own Vm; dropping the Vm releases the
/// globals table and the store, which together release every closure.
#[derive(Clone)]
pub struct Env {
    frame: Option<Rc<Frame>>,
    globals: Weak<RefCell<HashMap<Sym, Rc<RefCell<Val>>>>>,
    store: Weak<Store>,
}

struct Frame {
    name: Sym,
    addr: Addr,
    parent: Option<Rc<Frame>>,
}

impl Env {
    /// Env with no frames but a live back-edge to a globals table and
    /// a store. The `Weak`s are upgraded on lookup; while the Vm holds
    /// the strong refs they always succeed.
    pub fn with_globals(globals: &Globals, store: &Rc<Store>) -> Self {
        Env {
            frame: None,
            globals: Rc::downgrade(globals),
            store: Rc::downgrade(store),
        }
    }

    /// Allocate `val` in the store and bind `name` to its addr.
    pub fn extend(&self, name: Sym, val: Val) -> Env {
        let store = self.store.upgrade().expect("store dropped before env");
        let addr = store.alloc(val);
        self.extend_addr(name, addr)
    }

    fn extend_addr(&self, name: Sym, addr: Addr) -> Env {
        Env {
            frame: Some(Rc::new(Frame {
                name,
                addr,
                parent: self.frame.clone(),
            })),
            globals: self.globals.clone(),
            store: self.store.clone(),
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

    /// Allocate a placeholder slot in the store and bind `name` to its
    /// addr. Returns the addr so the caller (letrec setup, then
    /// `K::Letrec` apply) can patch the store slot once the init
    /// expression has been evaluated. The placeholder value should
    /// never be read before patching — if it is, it leaks out as `#f`,
    /// which makes the bug observable rather than UB.
    pub fn extend_placeholder(&self, name: Sym) -> (Env, Addr) {
        let store = self.store.upgrade().expect("store dropped before env");
        let addr = store.alloc(Val::Bool(false));
        (self.extend_addr(name, addr), addr)
    }

    pub fn lookup(&self, name: &str) -> Option<Val> {
        let mut cur = self.frame.as_deref();
        while let Some(f) = cur {
            if &*f.name == name {
                let store = self.store.upgrade()?;
                return Some(store.get(f.addr));
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

    /// Return the store handle this env is anchored to, if the owning
    /// Vm is still alive. Used by `K::Letrec`'s patch step to write
    /// the just-evaluated init into the placeholder slot.
    pub fn store_handle(&self) -> Option<Rc<Store>> {
        self.store.upgrade()
    }

    /// Return the frame-allocated `Addr` for `name`, if any. Walks
    /// frames only — does *not* fall through to globals. Exposed for
    /// diagnostic tests that need to observe slot identity without
    /// holding the store value alive.
    pub fn lookup_addr(&self, name: &str) -> Option<Addr> {
        let mut cur = self.frame.as_deref();
        while let Some(f) = cur {
            if &*f.name == name {
                return Some(f.addr);
            }
            cur = f.parent.as_deref();
        }
        None
    }

    /// Mutate the binding for `name`. Walks frames first (writing into
    /// the store slot via `Addr`), then falls through to the globals
    /// table on miss. Errors if `name` is unbound. Used by
    /// `(set! name val)`.
    pub fn set(&self, name: &str, val: Val) -> Result<(), String> {
        let mut cur = self.frame.as_deref();
        while let Some(f) = cur {
            if &*f.name == name {
                let store = self
                    .store
                    .upgrade()
                    .ok_or_else(|| "set!: store dropped before assignment".to_string())?;
                store.set(f.addr, val);
                return Ok(());
            }
            cur = f.parent.as_deref();
        }
        let globals = self
            .globals
            .upgrade()
            .ok_or_else(|| "set!: globals dropped before assignment".to_string())?;
        let table = globals.borrow();
        match table.get(name) {
            Some(slot) => {
                *slot.borrow_mut() = val;
                Ok(())
            }
            None => Err(format!("set!: unbound variable: {name}")),
        }
    }
}
