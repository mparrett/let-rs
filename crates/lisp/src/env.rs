use std::rc::{Rc, Weak};

use crate::expr::Sym;
use crate::ns::Namespace;
use crate::store::{Addr, Store};
use crate::val::Val;

/// The top-level bindings an `Env` resolves against: a [`Namespace`],
/// and through it the chain out to the root (ADR-042). Closures see it
/// via `Env::globals` as a `Weak` back-edge so the cycle that would
/// otherwise form — globals slot → closure → captured env → globals —
/// stays open. See ADR-015 / issue_2.
///
/// Note: top-level bindings stayed as their own region across the
/// ADR-023 CESK migration. Frame slots moved to the `Store`; globals
/// kept their `Rc<RefCell<Val>>` cells so the ADR-015 Weak back-edge
/// pattern still holds end-to-end without re-derivation.
pub type Globals = Rc<Namespace>;

/// Immutable, structurally-shared linked frames for lexical bindings
/// (`let`, `letrec`, closure params). Each slot is an `Addr` into the
/// Vm's `Store` (ADR-023). The store holds the value; frames just hold
/// the index. `Addr` is `Copy`, so a closure that captures an env can
/// no longer Rc-reach back to its own letrec cell — the cycle from
/// ADR-021 dissolves by construction.
///
/// A frame owns its slot: when the last `Env` naming a frame drops,
/// `Frame::drop` returns the slot to the store's free list (ADR-033).
/// So a loop that allocates a binding per iteration reuses one slot
/// rather than growing the arena without bound — *unless* the slot's
/// value captures this very frame, which keeps it alive and defeats
/// the drop. See the known-residual note in `store.rs` and ADR-038.
///
/// `globals` is a `Weak` reference to the Vm's top-level table; any
/// name not found in the frame chain falls back to it. `store` is the
/// matching `Weak` for frame-slot resolution. Both `Weak` keeps any
/// closure from rooting its own Vm; dropping the Vm releases the
/// globals table and the store, which together release every closure.
#[derive(Clone)]
pub struct Env {
    frame: Option<Rc<Frame>>,
    globals: Weak<Namespace>,
    store: Weak<Store>,
}

struct Frame {
    name: Sym,
    addr: Addr,
    /// Back-edge to the arena this frame's slot lives in, so the slot
    /// can be reclaimed when the frame dies (ADR-033). `Weak` for the
    /// same reason `Env::store` is: a frame must not root its own Vm.
    store: Weak<Store>,
    parent: Option<Rc<Frame>>,
}

impl Drop for Frame {
    /// A frame owns its store slot. When the last `Env` referencing
    /// this frame goes away the binding is unreachable, so the slot
    /// returns to the free list (ADR-033).
    ///
    /// This does not fire when the slot's own value reaches back here —
    /// a recursive closure holds its frame alive, so the frame never
    /// drops and the slot is never freed. Pinned by
    /// `recursive_closures_retain_their_slot`; see ADR-038.
    ///
    /// An upgrade failure means the Vm dropped first and took the
    /// whole arena with it; there is nothing to reclaim into.
    fn drop(&mut self) {
        if let Some(store) = self.store.upgrade() {
            store.free(self.addr);
        }
    }
}

impl Env {
    /// Env with no frames but a live back-edge to a globals table and
    /// a store. The `Weak`s are upgraded on lookup; while the Vm holds
    /// the strong refs they always succeed.
    ///
    /// Crate-private: it takes the two `Rc`s the `Vm` is meant to be sole
    /// strong owner of, so a public constructor is a public request for
    /// exactly what must not escape. Only `Vm::new` calls it.
    pub(crate) fn with_globals(globals: &Globals, store: &Rc<Store>) -> Self {
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

    /// Sole constructor of `Frame`. Every caller passes a freshly
    /// allocated `addr`, which is what makes the frame the unique owner
    /// of its slot — the invariant `Store::free` relies on.
    fn extend_addr(&self, name: Sym, addr: Addr) -> Env {
        Env {
            frame: Some(Rc::new(Frame {
                name,
                addr,
                store: self.store.clone(),
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
        // Frame miss — fall through to this env's namespace and, on a
        // miss there, outward along its chain to the root. The Weak
        // upgrade only fails after the owning Vm has been dropped, in
        // which case any surviving closure was definitionally orphaned.
        //
        // *Which* namespace comes from the closure's captured env, not
        // from anything ambient, which is what makes a pack's internals
        // resolve to its own definitions no matter who calls them
        // (ADR-042).
        self.globals.upgrade()?.get(name)
    }

    // `namespace` (upgrade the globals back-edge to `Rc<Namespace>`) was
    // removed in ADR-043. It had no caller inside the engine and was a
    // public ownership escape: `Env` is public and holds only `Weak`s, so
    // handing one out roots nothing — but *upgrading* let a caller keep a
    // pack table, its cells and its closures alive after the `Vm` was
    // gone. That is the sole-strong-owner invariant ADR-036 made
    // `Vm::globals` private to protect and ADR-042 restored via the
    // opaque `NsHandle`, reopened from the other side. `Vm::env_in`
    // widened it from the root to any pack, which is how it was found.
    // If something ever genuinely needs the namespace behind an `Env`,
    // give it a `pub(crate)` accessor — not a public one.

    /// Same frames and store, resolving top-level names against `ns`
    /// instead. How a host evaluates source *inside* a pack.
    ///
    /// Crate-private for the same reason as [`Env::namespace`], plus one
    /// of its own: it takes an `Rc<Namespace>`, so a public version would
    /// need callers to hold the very thing that must not escape.
    pub(crate) fn with_namespace(&self, ns: &Rc<Namespace>) -> Env {
        Env {
            frame: self.frame.clone(),
            globals: Rc::downgrade(ns),
            store: self.store.clone(),
        }
    }

    /// Return the store handle this env is anchored to, if the owning
    /// Vm is still alive. Used by `K::Letrec`'s patch step to write
    /// the just-evaluated init into the placeholder slot.
    ///
    /// Crate-private, and the same escape as [`Env::namespace`] one
    /// register over: this upgrades to a strong `Rc<Store>`, so a public
    /// version lets a caller keep the whole arena — every frame slot in
    /// it — alive past its `Vm`. `Vm::store_weak` is the read-only
    /// diagnostic handle ADR-036 intends for outside use. Found while
    /// fixing the namespace case; it was public from the ADR-023 CESK
    /// migration onward and no test had ever tried it.
    pub(crate) fn store_handle(&self) -> Option<Rc<Store>> {
        self.store.upgrade()
    }

    // `lookup_addr` (a frames-only `Addr` accessor, added for diagnostic
    // tests and never used by one) was removed in ADR-033. Slots are
    // recycled now, so an `Addr` that outlives its frame reads a
    // different binding instead of faulting — and that accessor was the
    // only way for one to escape the engine's "holds the Addr ⇒ holds
    // the Env" invariant.

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
        self.globals
            .upgrade()
            .ok_or_else(|| "set!: globals dropped before assignment".to_string())?
            .set(name, val)
    }
}
