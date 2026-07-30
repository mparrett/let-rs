use std::rc::Rc;

use crate::env::Env;
use crate::error::Span;
use crate::expr::{Expr, Sym};
use crate::store::Addr;
use crate::val::Val;

/// A continuation — the "rest of the computation", reified as data.
/// Each frame says "when the current expression produces a value, do this next".
#[derive(Clone)]
pub enum K {
    /// Top of the chain: we're done.
    Halt,

    /// In the middle of `(f a b c)`. `args` is the whole application,
    /// shared with the `Expr::App` it came from. `evaled` holds the
    /// function plus the arguments evaluated so far — and because they
    /// are filled strictly left to right, `evaled.len()` is also the
    /// index into `args` of the subexpression currently in flight, so
    /// no separate cursor can drift out of sync with it.
    ///
    /// Pre-ADR-035 this was `evaled` plus a `remaining: Vec<Rc<Expr>>`,
    /// both cloned on every argument — O(n²) `Val` clones and 2n vector
    /// allocations per application.
    ///
    /// `span` is the call site's position, carried from the `Expr::App`.
    /// It's what `apply` attaches to arity mismatches and to prim errors,
    /// which have no source position of their own (ADR-039). It also
    /// makes the continuation chain a backtrace: walking `k` from a
    /// paused or failed state yields the enclosing call sites in order.
    App {
        evaled: Vec<Val>,
        args: Rc<[Rc<Expr>]>,
        env: Env,
        span: Option<Span>,
        k: Rc<K>,
    },

    /// We evaluated the `cond` of an `if`; the value picks the branch.
    If {
        then_branch: Rc<Expr>,
        else_branch: Rc<Expr>,
        env: Env,
        k: Rc<K>,
    },

    /// Letrec init evaluation in progress. The just-evaluated value gets
    /// written to `addrs[next]` in the store; when `next` reaches the end
    /// of `inits` we eval `body`, else the next init. `env` already
    /// contains all the placeholder bindings, so any closure produced by
    /// an init captures the recursive environment.
    ///
    /// Post-ADR-023: `addrs` are `Copy` indices into the store, not Rc
    /// cells. Post-ADR-035: `addrs` and `inits` are shared slices walked
    /// by `next`, rather than vectors re-cloned at every binding.
    Letrec {
        addrs: Rc<[Addr]>,
        inits: Rc<[Rc<Expr>]>,
        next: usize,
        body: Rc<Expr>,
        env: Env,
        k: Rc<K>,
    },

    /// Set! mutation: evaluating the val expression of `(set! name
    /// val)`. The just-evaluated value is written into whatever slot
    /// `name` resolves to — frame addr in the store, or globals cell.
    /// `env` is the env at the set! site; we look the name up there
    /// rather than capturing the slot up front so the form behaves
    /// like a `Var` reference — the same forward-reference rules
    /// apply.
    SetBang { name: Sym, env: Env, k: Rc<K> },

    /// The condition expression of a `(raise e)` is in flight. When it
    /// produces a value, that value becomes the condition. `span` is the
    /// raise site, carried so an escaping condition can report where it
    /// came from.
    Raise { span: Option<Span>, k: Rc<K> },

    /// A `guard` whose body is in flight. Inert on the way *up* — a
    /// value passing through pops it unchanged — and load-bearing only
    /// when a raise unwinds into it, at which point `var` is bound to
    /// the condition and `handler` runs in `env`.
    ///
    /// This is the only frame `Mode::Raise` stops at; every other
    /// variant is discarded as the raise walks outward, which is what
    /// makes unwinding free the store slots those frames' environments
    /// were holding (ADR-041).
    Guard {
        var: Sym,
        handler: Rc<Expr>,
        env: Env,
        k: Rc<K>,
    },
}
