use std::fmt;
use std::rc::Rc;

use crate::env::Env;
use crate::expr::{Expr, Sym};
use crate::world::World;

#[derive(Clone)]
pub enum Val {
    Num(i64),
    Bool(bool),
    Sym(Sym),
    Nil,
    Cons(Rc<Val>, Rc<Val>),
    Clo {
        params: Vec<Sym>,
        body: Rc<Expr>,
        env: Env,
    },
    Prim {
        name: &'static str,
        arity: Arity,
        f: fn(&[Val]) -> Result<Val, String>,
    },
    /// Like `Prim`, but its `f` receives mutable access to the host world.
    /// Used for primitives that read or modify game state.
    WorldPrim {
        name: &'static str,
        arity: Arity,
        f: fn(&[Val], &mut World) -> Result<Val, String>,
    },
}

#[derive(Clone, Copy)]
pub enum Arity {
    Exact(usize),
    AtLeast(usize),
}

impl Arity {
    pub fn accepts(self, n: usize) -> bool {
        match self {
            Arity::Exact(k) => n == k,
            Arity::AtLeast(k) => n >= k,
        }
    }

    pub fn describe(self) -> String {
        match self {
            Arity::Exact(k) => k.to_string(),
            Arity::AtLeast(k) => format!("at least {k}"),
        }
    }
}

impl Val {
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Val::Bool(false))
    }

    /// Build a proper list `(a b c)` from a slice, terminated by `Nil`.
    pub fn list_from(items: &[Val]) -> Val {
        let mut acc = Val::Nil;
        for v in items.iter().rev() {
            acc = Val::Cons(Rc::new(v.clone()), Rc::new(acc));
        }
        acc
    }

    /// Pointer-style equality for atoms; `#f` for compound values (Scheme `eq?`
    /// behavior on conses/closures without interning).
    pub fn eq_shallow(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Num(a), Val::Num(b)) => a == b,
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Sym(a), Val::Sym(b)) => &**a == &**b,
            (Val::Nil, Val::Nil) => true,
            _ => false,
        }
    }
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Num(n) => write!(f, "{n}"),
            Val::Bool(true) => write!(f, "#t"),
            Val::Bool(false) => write!(f, "#f"),
            Val::Sym(s) => write!(f, "{s}"),
            Val::Nil => write!(f, "()"),
            Val::Cons(h, t) => {
                write!(f, "(")?;
                write_pair(h, t, f)?;
                write!(f, ")")
            }
            Val::Clo { params, .. } => write!(f, "#<closure/{}>", params.len()),
            Val::Prim { name, arity, .. } => write!(f, "#<prim {name}/{}>", arity.describe()),
            Val::WorldPrim { name, arity, .. } => {
                write!(f, "#<world-prim {name}/{}>", arity.describe())
            }
        }
    }
}

fn write_pair(head: &Val, tail: &Val, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{head}")?;
    match tail {
        Val::Nil => Ok(()),
        Val::Cons(h, t) => {
            write!(f, " ")?;
            write_pair(h, t, f)
        }
        other => write!(f, " . {other}"),
    }
}

impl fmt::Debug for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
