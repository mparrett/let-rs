use std::rc::Rc;

use crate::error::Span;
use crate::val::Val;

pub type Sym = Rc<str>;

#[derive(Debug, Clone)]
pub enum Expr {
    Num(i64),
    Bool(bool),
    /// A variable reference, with the position it was read from.
    ///
    /// `Var` and `App` are the only variants that carry a span, because
    /// they are the only ones that can fail at run time: every runtime
    /// error the engine raises is either `unbound variable` (here) or
    /// something reached through a call — `not callable`, an arity
    /// mismatch, or a prim's own complaint. See ADR-039 on why that's
    /// the useful subset of ADR-022's deferred Phase 2 rather than a
    /// span on all nine variants.
    ///
    /// `None` for forms with no source text: macro output and
    /// host-constructed applications.
    Var(Sym, Option<Span>),
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
    /// nothing for the subexpressions themselves (ADR-035). The span
    /// covers the opening paren of the call — see `Var` above for why
    /// this variant carries one.
    App(Rc<[Rc<Expr>]>, Option<Span>),
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
    /// `(raise expr)` — evaluate `expr`, then unwind to the nearest
    /// enclosing `Guard` carrying it as the condition.
    ///
    /// `(error msg irritant …)` compiles to this too, wrapping its
    /// arguments in a `(list 'error msg irritant …)` application, so the
    /// engine has one raising form rather than two (ADR-041). The span
    /// is the raise site, used only if the condition escapes to the top
    /// and has to become a `LispErr`.
    Raise(Rc<Expr>, Option<Span>),
    /// `(guard (var handler) body)` — evaluate `body`; if it raises,
    /// bind `var` to the condition and evaluate `handler` instead.
    ///
    /// The handler runs *after* unwinding, in the guard's own
    /// environment extended with `var`. Nothing between here and the
    /// raise survives — see ADR-041 on why resumable handlers were left
    /// for later even though the reified continuation makes them
    /// unusually cheap.
    Guard {
        var: Sym,
        handler: Rc<Expr>,
        body: Rc<Expr>,
    },
}
