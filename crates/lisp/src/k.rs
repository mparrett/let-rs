use std::cell::RefCell;
use std::rc::Rc;

use crate::env::Env;
use crate::expr::Expr;
use crate::val::Val;

/// A continuation — the "rest of the computation", reified as data.
/// Each frame says "when the current expression produces a value, do this next".
#[derive(Clone)]
pub enum K {
    /// Top of the chain: we're done.
    Halt,

    /// In the middle of `(f a b c)`. `evaled` holds the function + already-evaluated
    /// args; `remaining` is what's left. Each value we produce gets pushed onto
    /// `evaled`, then we either advance to the next `remaining` or apply.
    App {
        evaled: Vec<Val>,
        remaining: Vec<Rc<Expr>>,
        env: Env,
        k: Rc<K>,
    },

    /// We evaluated the `cond` of an `if`; the value picks the branch.
    If {
        then_branch: Rc<Expr>,
        else_branch: Rc<Expr>,
        env: Env,
        k: Rc<K>,
    },

    /// Letrec init evaluation in progress. The just-evaluated value gets written
    /// to `cells[next]`; if `remaining` is empty we eval `body`, else the next
    /// init. `env` already contains all the placeholder cells, so any closure
    /// produced by an init captures the recursive environment.
    Letrec {
        cells: Vec<Rc<RefCell<Val>>>,
        next: usize,
        remaining: Vec<Rc<Expr>>,
        body: Rc<Expr>,
        env: Env,
        k: Rc<K>,
    },
}
