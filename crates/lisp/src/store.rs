//! Append-only heap for lexical-binding values. The fourth register
//! of the CESK state machine (ADR-023). Frame slots in `env.rs` hold
//! `Addr` indices instead of `Rc<RefCell<Val>>` cells; the store owns
//! every value, and closures reach it via an `Env::store` `Weak`
//! back-edge so closures can't root the store themselves.
//!
//! The letrec leak from ADR-021 dissolves here by construction:
//! `Addr` is `Copy`, so a closure → env → frame → addr path has no
//! Rc edge back to the value the closure was created from.
//!
//! Append-only is intentional. There is no reclamation today — the
//! store grows for the lifetime of the owning `Vm` and drops in one
//! shot when the Vm drops. A persistent / HAMT-backed store (for
//! cheap snapshots and undo) is the open follow-on in ADR-023.

use std::cell::RefCell;

use crate::val::Val;

/// Index into a `Store`. `Copy`; no Rc edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Addr(pub u32);

pub struct Store {
    cells: RefCell<Vec<Val>>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            cells: RefCell::new(Vec::new()),
        }
    }

    /// Append `v` and return its address. Panics if the address space
    /// is exhausted (2^32 slots — a Vm leak, not a bug).
    pub fn alloc(&self, v: Val) -> Addr {
        let mut cells = self.cells.borrow_mut();
        let idx = cells.len();
        assert!(
            idx < u32::MAX as usize,
            "store: address space exhausted (this Vm has allocated 2^32 frame slots)"
        );
        cells.push(v);
        Addr(idx as u32)
    }

    pub fn get(&self, addr: Addr) -> Val {
        self.cells.borrow()[addr.0 as usize].clone()
    }

    pub fn set(&self, addr: Addr, v: Val) {
        self.cells.borrow_mut()[addr.0 as usize] = v;
    }

    pub fn len(&self) -> usize {
        self.cells.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.borrow().is_empty()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}
