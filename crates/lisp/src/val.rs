use std::fmt;
use std::rc::Rc;

use crate::env::Env;
use crate::expr::{Expr, Sym};

/// Boxed function pointer for host primitives. Closures can capture
/// host state (an `Rc<RefCell<World>>`, an `Rc<RefCell<i64>>` counter,
/// whatever) at registration time; the engine has no awareness of what
/// they capture. Replaces ADR-005's two-variant `Prim` / `WorldPrim`
/// split (ADR-017).
pub type PrimFn = Rc<dyn Fn(&[Val]) -> Result<Val, String>>;

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
    /// Immutable UTF-8 string. `Rc<str>` so cloning is a refcount bump
    /// and contents are shared. Self-evaluating; `eq?` compares contents
    /// (matches `Sym` precedent — neither is interned, both compare by value).
    Str(Rc<str>),
    Nil,
    Cons(Rc<Val>, Rc<Val>),
    /// A closure. `params` is `Rc<[Sym]>` rather than `Vec<Sym>`
    /// because `Val` is `Clone` and `Env::lookup` clones out of the
    /// store — with a `Vec` every reference to a function name
    /// allocated a fresh params vector (ADR-035).
    Clo {
        params: Rc<[Sym]>,
        body: Rc<Expr>,
        env: Env,
    },
    /// Host primitive: a closure of `&[Val] -> Result<Val, String>` plus
    /// a name and arity. Closures may capture host state (see `PrimFn`).
    Prim {
        name: &'static str,
        arity: Arity,
        f: PrimFn,
    },
}

impl Drop for Val {
    fn drop(&mut self) {
        // Dropping a long list recurses once per cell as each `Rc<Val>` tail
        // reaches refcount zero — a deep enough list overflows the stack the
        // same way the printer used to. Dismantle the spine iteratively:
        // sever each link and descend only into tails we uniquely own (a
        // shared tail is left intact for its other owners). Cons heads still
        // drop normally, bounded by their own nesting depth.
        if let Val::Cons(_, tail) = self {
            let mut next = std::mem::replace(tail, Rc::new(Val::Nil));
            while let Ok(mut cell) = Rc::try_unwrap(next) {
                match &mut cell {
                    Val::Cons(_, t) => next = std::mem::replace(t, Rc::new(Val::Nil)),
                    _ => break,
                }
                // `cell` drops here with its tail already severed to Nil.
            }
        }
    }
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
            (Val::Str(a), Val::Str(b)) => **a == **b,
            (Val::Nil, Val::Nil) => true,
            _ => false,
        }
    }
}

pub(crate) fn gcd_i128(mut a: u128, mut b: u128) -> u128 {
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
            Val::Str(s) => write_string_literal(s, f),
            Val::Nil => write!(f, "()"),
            Val::Cons(h, t) => {
                write!(f, "(")?;
                write_pair(h, t, f)?;
                write!(f, ")")
            }
            Val::Clo { params, .. } => write!(f, "#<closure/{}>", params.len()),
            Val::Prim { name, arity, .. } => write!(f, "#<prim {name}/{}>", arity.describe()),
        }
    }
}

/// `write`-style string output: surrounding `"` plus the four escapes
/// the tokenizer understands (`\"`, `\\`, `\n`, `\t`). Round-trips through
/// `read` for any string this implementation can parse.
fn write_string_literal(s: &str, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "\"")?;
    for c in s.chars() {
        match c {
            '"' => write!(f, "\\\"")?,
            '\\' => write!(f, "\\\\")?,
            '\n' => write!(f, "\\n")?,
            '\t' => write!(f, "\\t")?,
            _ => write!(f, "{c}")?,
        }
    }
    write!(f, "\"")
}

fn write_pair(head: &Val, tail: &Val, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    // Walk the cons spine iteratively so a long proper list prints in O(n)
    // stack space instead of recursing once per element (which overflowed
    // the stack around ~30k elements). Only each head recurses, via its own
    // Display, bounded by that element's nesting depth.
    write!(f, "{head}")?;
    let mut tail = tail;
    loop {
        match tail {
            Val::Nil => return Ok(()),
            Val::Cons(h, t) => {
                write!(f, " {h}")?;
                tail = t;
            }
            other => return write!(f, " . {other}"),
        }
    }
}

impl fmt::Debug for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
