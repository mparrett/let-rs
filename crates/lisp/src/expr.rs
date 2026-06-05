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
    Lam(Vec<Sym>, Rc<Expr>),
    App(Vec<Rc<Expr>>),
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
