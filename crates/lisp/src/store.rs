//! Heap for lexical-binding values. The fourth register of the CESK
//! state machine (ADR-023). Frame slots in `env.rs` hold `Addr`
//! indices instead of `Rc<RefCell<Val>>` cells; the store owns every
//! value, and closures reach it via an `Env::store` `Weak` back-edge
//! so closures can't root the store themselves.
//!
//! The letrec leak from ADR-021 dissolves here by construction:
//! `Addr` is `Copy`, so a closure → env → frame → addr path has no
//! Rc edge back to the value the closure was created from.
//!
//! Slots are reclaimed (ADR-033). Every `Addr` is owned by exactly one
//! `Frame`, so `Frame::drop` returns the slot to a free list and the
//! next `alloc` reuses it. For any binding that doesn't outlive its
//! frame — loop variables, parameters, `let` bindings — the high-water
//! mark is set by the deepest *live* environment rather than by total
//! allocations over the Vm's lifetime.
//!
//! **Known residual.** A closure that captures the frame owning its own
//! slot is retained:
//!
//! ```text
//! store slot ──> Val::Clo ──> captured Env ──> Rc<Frame> ──> owns slot
//! ```
//!
//! `Frame::drop` frees the slot, but the closure *in* that slot holds
//! the frame alive, so the drop never runs. This is the ADR-021 cycle
//! in a new form, and refcounting can't break it without tracing. It
//! costs one slot per recursive closure (not per iteration), it is
//! bounded, and the append-only store retained these too — but it means
//! reclamation is narrower than "slot lifetime tracks frame lifetime."
//! Pinned by `recursive_closures_retain_their_slot`; the fix is the
//! trial-deletion sweep in ADR-038.
//!
//! A persistent / HAMT-backed store (for cheap snapshots and undo) is
//! still the open follow-on in ADR-023; neither reclamation nor the
//! sweep closes that door.

use std::cell::RefCell;

use crate::val::Val;

/// Index into a `Store`. `Copy`; no Rc edge.
///
/// An `Addr` is only valid while the `Frame` that owns it is alive.
/// Slots are recycled after a frame drops, so an `Addr` outliving its
/// frame will silently read a *different* binding rather than fault.
/// Nothing in the engine holds an `Addr` without also holding the
/// `Env` that keeps its frame alive (`K::Letrec` carries both), and
/// there is no public accessor that hands one out — see ADR-033.
///
/// The `u32` is private (ADR-036) so an `Addr` can only come from
/// `Store::alloc`. That is what makes the invariant above enforceable
/// rather than merely documented: `store_weak` hands out a `Store`, but
/// without a constructible `Addr` there is nothing a holder can read or
/// write through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Addr(u32);

pub struct Store {
    cells: RefCell<Vec<Val>>,
    /// Slots whose owning frame has dropped, available for reuse.
    /// Every entry holds `Val::Nil` — `free` clears the slot so a dead
    /// binding's value isn't pinned until the slot is next allocated.
    free: RefCell<Vec<u32>>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            cells: RefCell::new(Vec::new()),
            free: RefCell::new(Vec::new()),
        }
    }

    /// Bind `v` to a slot and return its address, reusing a freed slot
    /// when one is available. Panics if the address space is exhausted
    /// (2^32 *live* slots — an environment that deep is a runaway, not
    /// a workload).
    pub(crate) fn alloc(&self, v: Val) -> Addr {
        if let Some(idx) = self.free.borrow_mut().pop() {
            // The slot holds `Val::Nil` (cleared by `free`), so the
            // implicit drop of the old value here is a no-op and can't
            // re-enter the store while `cells` is borrowed.
            self.cells.borrow_mut()[idx as usize] = v;
            return Addr(idx);
        }
        let mut cells = self.cells.borrow_mut();
        let idx = cells.len();
        assert!(
            idx < u32::MAX as usize,
            "store: address space exhausted (this Vm has 2^32 live frame slots)"
        );
        cells.push(v);
        Addr(idx as u32)
    }

    pub(crate) fn get(&self, addr: Addr) -> Val {
        self.cells.borrow()[addr.0 as usize].clone()
    }

    /// Overwrite the value at `addr` (letrec placeholder patching,
    /// `set!`). The displaced value drops *outside* the borrow for the
    /// same re-entrancy reason as `free`: it may be the last owner of a
    /// closure whose env holds frames, and those frames free slots as
    /// they die.
    pub(crate) fn set(&self, addr: Addr, v: Val) {
        let displaced = {
            let mut cells = self.cells.borrow_mut();
            std::mem::replace(&mut cells[addr.0 as usize], v)
        };
        drop(displaced);
    }

    /// Return `addr` to the free list and release the value it held.
    ///
    /// Called only from `Frame::drop`. A `Frame` owns its `Addr`
    /// uniquely — `extend_addr` is the sole constructor and every call
    /// site passes a freshly-allocated address — so double-free is
    /// structurally impossible rather than merely unlikely.
    pub(crate) fn free(&self, addr: Addr) {
        // Take the value out *under* the borrow but drop it *after*
        // releasing: dropping a `Val` can cascade into a closure → env
        // → frame chain, which re-enters `free`. Dropping in place
        // would hit a live `borrow_mut` and panic.
        let dead = {
            let mut cells = self.cells.borrow_mut();
            std::mem::replace(&mut cells[addr.0 as usize], Val::Nil)
        };
        self.free.borrow_mut().push(addr.0);
        drop(dead);
    }

    /// Number of *live* slots — allocated minus freed. This is the
    /// number that must stay bounded across repeated evaluation; see
    /// `store_reclaims_frame_slots` in `tests/eval.rs`.
    pub fn len(&self) -> usize {
        self.cells.borrow().len() - self.free.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total slots the backing vector has ever grown to (live + freed).
    /// The high-water mark, exposed for diagnostics; `len` is what
    /// callers usually want.
    pub fn slots(&self) -> usize {
        self.cells.borrow().len()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}
