use std::fmt;
use std::rc::Rc;

use crate::env::Env;
use crate::expr::{Expr, Sym};
use crate::world::World;

#[derive(Clone)]
pub enum Val {
    Num(i64),
    /// Exact rational. Always stored in lowest terms with `den > 0`;
    /// construct via `Val::make_ratio` to enforce both invariants.
    /// A ratio that simplifies to an integer is returned as `Val::Num`
    /// by the constructor, so a `Val::Ratio` always has `den >= 2`.
    Ratio(i64, u64),
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

    /// Sugar over `Val::Cons(Rc::new(h), Rc::new(t))`. Same shape Scheme
    /// uses to build both proper lists and dotted pairs.
    pub fn cons(head: Val, tail: Val) -> Val {
        Val::Cons(Rc::new(head), Rc::new(tail))
    }

    /// Build a proper list `(a b c)` from a slice, terminated by `Nil`.
    pub fn list_from(items: &[Val]) -> Val {
        let mut acc = Val::Nil;
        for v in items.iter().rev() {
            acc = Val::cons(v.clone(), acc);
        }
        acc
    }

    /// Build an association list `((k1 . v1) (k2 . v2) …)` from a slice
    /// of key/value pairs. Order preserved.
    pub fn alist_from(pairs: &[(Val, Val)]) -> Val {
        let mut acc = Val::Nil;
        for (k, v) in pairs.iter().rev() {
            acc = Val::cons(Val::cons(k.clone(), v.clone()), acc);
        }
        acc
    }

    /// Construct an exact rational. Normalizes:
    /// - errors on zero denominator
    /// - moves sign onto the numerator (denominator always positive)
    /// - reduces by gcd
    /// - collapses to `Val::Num` when the reduced denominator is 1
    ///
    /// Intermediates use `i128` to survive the kind of denominator
    /// growth common in unreduced arithmetic; the final values must
    /// fit in `(i64, u64)` or the call errors.
    pub fn make_ratio(num: i128, den: i128) -> Result<Val, String> {
        if den == 0 {
            return Err("ratio: zero denominator".into());
        }
        let (mut n, mut d) = if den < 0 { (-num, -den) } else { (num, den) };
        let g = gcd_i128(n.unsigned_abs(), d as u128) as i128;
        n /= g;
        d /= g;
        if d == 1 {
            let n64: i64 = n
                .try_into()
                .map_err(|_| format!("ratio: numerator {n} out of i64 range"))?;
            return Ok(Val::Num(n64));
        }
        let n64: i64 = n
            .try_into()
            .map_err(|_| format!("ratio: numerator {n} out of i64 range"))?;
        let d64: u64 = (d as u128)
            .try_into()
            .map_err(|_| format!("ratio: denominator {d} out of u64 range"))?;
        Ok(Val::Ratio(n64, d64))
    }

    /// Pointer-style equality for atoms; `#f` for compound values (Scheme `eq?`
    /// behavior on conses/closures without interning). Ratios are stored in
    /// lowest terms so structural eq matches numerical eq for them. A `Num`
    /// and a `Ratio` are never `eq?` because `make_ratio` collapses any
    /// integer-valued ratio to `Num` at construction.
    pub fn eq_shallow(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Num(a), Val::Num(b)) => a == b,
            (Val::Ratio(an, ad), Val::Ratio(bn, bd)) => an == bn && ad == bd,
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Sym(a), Val::Sym(b)) => **a == **b,
            (Val::Nil, Val::Nil) => true,
            _ => false,
        }
    }
}

fn gcd_i128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.max(1)
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Num(n) => write!(f, "{n}"),
            Val::Ratio(n, d) => write!(f, "{n}/{d}"),
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
