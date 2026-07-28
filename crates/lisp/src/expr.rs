use std::rc::Rc;

use crate::val::Val;

pub type Sym = Rc<str>;

#[derive(Debug, Clone)]
pub enum Expr {
    Num(i64),
    Bool(bool),
    Var(Sym),
    /// `'x`, `(quote (a b c))`. The datum is pre-converted to a Val at compile
    /// time, so eval is a single Rc clone — no per-eval allocation for literals.
    Quote(Rc<Val>),
    /// Params are `Rc<[Sym]>`, not `Vec<Sym>`, so evaluating a `lambda`
    /// into a `Val::Clo` is a refcount bump rather than a fresh
    /// allocation — and so is every later lookup of a name bound to
    /// that closure, since `Env::lookup` clones the `Val` out of the
    /// store. See ADR-035.
    Lam(Rc<[Sym]>, Rc<Expr>),
    /// The callee and its argument expressions, shared with the
    /// `K::App` that walks them so entering an application allocates
    /// nothing for the subexpressions themselves (ADR-035).
    App(Rc<[Rc<Expr>]>),
    If(Rc<Expr>, Rc<Expr>, Rc<Expr>),
    /// `(letrec ((name init) ...) body)`. Bindings see each other and themselves.
    /// Init expressions are evaluated left-to-right; each result is patched into
    /// the binding's pre-allocated cell before the next init runs.
    Letrec(Vec<(Sym, Rc<Expr>)>, Rc<Expr>),
    /// `(set! name val)`. Evaluates `val`, then mutates the slot
    /// `name` is bound to — frame slot via the store, or the globals
    /// table cell. Errors at apply time if `name` is unbound. The
    /// CESK store (ADR-023) makes this cheap: frame slots are
    /// already mutable cells, we just need a write path.
    SetBang(Sym, Rc<Expr>),
}
