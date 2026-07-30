use std::cell::RefCell;
use std::rc::Rc;

use crate::val::{Arity, Val, gcd_i128};

type R = Result<Val, String>;
/// Internal fn-ptr type for the `BUILTINS` table. Each entry is wrapped
/// once in an `Rc<dyn Fn>` at `Vm::new` time (~40 allocations per Vm),
/// so the per-call lookup cost is a refcount bump rather than a fn-ptr
/// copy. See ADR-017 (closure-capable prims) and ADR-020 (prims live
/// alongside user defines in the Vm-owned globals table).
type BuiltinFn = fn(&[Val]) -> R;

// ---- numeric tower helpers ----
//
// Internal representation during arithmetic is `(i128, i128)` for
// (numerator, denominator). Integers promote to `(n, 1)`. Each binary
// op pre-reduces by gcd and uses checked arithmetic so overflow on
// representable inputs surfaces as a clean Err instead of an i128 wrap.
// `Val::make_ratio` still gates the final i64/u64 narrowing.

type Ratio = (i128, i128);

fn as_ratio(v: &Val, name: &str) -> Result<Ratio, String> {
    match v {
        Val::Num(n) => Ok((*n as i128, 1)),
        Val::Ratio(n, d) => Ok((*n as i128, *d as i128)),
        other => Err(format!("{name}: expected number, got {other}")),
    }
}

fn ratio_args(args: &[Val], name: &str) -> Result<Vec<Ratio>, String> {
    args.iter().map(|v| as_ratio(v, name)).collect()
}

fn gcd(a: i128, b: i128) -> i128 {
    gcd_i128(a.unsigned_abs(), b.unsigned_abs()) as i128
}

/// Reduce `(n, d)` to lowest terms with `d > 0`. Identity on (0, 0);
/// callers that can produce zero denominators must check first.
fn reduce((n, d): Ratio) -> Ratio {
    if d == 0 {
        return (n, d);
    }
    let g = gcd(n, d);
    if d < 0 {
        (-n / g, -d / g)
    } else {
        (n / g, d / g)
    }
}

fn overflow(op: &str) -> String {
    format!("{op}: numeric overflow")
}

/// `a/x + b/y` with denominator pre-gcd reduction. Cross-multiplying
/// unreduced (the previous approach) overflowed on representable inputs
/// like `(+ 1/N 1/N)` where `N` is near `i64::MAX` — `N*N` exceeds i128
/// range. Reducing first keeps the magnitudes bounded.
fn ratio_add(a: Ratio, b: Ratio, op: &str) -> Result<Ratio, String> {
    let (an, ad) = a;
    let (bn, bd) = b;
    let g = gcd(ad, bd);
    let ad_g = ad / g;
    let bd_g = bd / g;
    let term1 = an.checked_mul(bd_g).ok_or_else(|| overflow(op))?;
    let term2 = bn.checked_mul(ad_g).ok_or_else(|| overflow(op))?;
    let n = term1.checked_add(term2).ok_or_else(|| overflow(op))?;
    let d = ad.checked_mul(bd_g).ok_or_else(|| overflow(op))?;
    Ok(reduce((n, d)))
}

fn ratio_sub(a: Ratio, b: Ratio, op: &str) -> Result<Ratio, String> {
    let (an, ad) = a;
    let (bn, bd) = b;
    let g = gcd(ad, bd);
    let ad_g = ad / g;
    let bd_g = bd / g;
    let term1 = an.checked_mul(bd_g).ok_or_else(|| overflow(op))?;
    let term2 = bn.checked_mul(ad_g).ok_or_else(|| overflow(op))?;
    let n = term1.checked_sub(term2).ok_or_else(|| overflow(op))?;
    let d = ad.checked_mul(bd_g).ok_or_else(|| overflow(op))?;
    Ok(reduce((n, d)))
}

/// `a/x * b/y` with cross-pair gcd reduction (cancel a-vs-y and b-vs-x
/// before multiplying so the products stay bounded).
fn ratio_mul(a: Ratio, b: Ratio, op: &str) -> Result<Ratio, String> {
    let (an, ad) = a;
    let (bn, bd) = b;
    let g1 = gcd(an, bd);
    let g2 = gcd(bn, ad);
    let n = (an / g1.max(1))
        .checked_mul(bn / g2.max(1))
        .ok_or_else(|| overflow(op))?;
    let d = (ad / g2.max(1))
        .checked_mul(bd / g1.max(1))
        .ok_or_else(|| overflow(op))?;
    Ok(reduce((n, d)))
}

/// `a/x / b/y = a/x * y/b`. Errors on zero `b`.
fn ratio_div(a: Ratio, b: Ratio, op: &str) -> Result<Ratio, String> {
    let (bn, bd) = b;
    if bn == 0 {
        return Err(format!("{op}: division by zero"));
    }
    ratio_mul(a, (bd, bn), op)
}

// ---- arithmetic (variadic) ----

fn add(args: &[Val]) -> R {
    let xs = ratio_args(args, "+")?;
    let mut acc: Ratio = (0, 1);
    for r in xs {
        acc = ratio_add(acc, r, "+")?;
    }
    Val::make_ratio(acc.0, acc.1)
}

fn sub(args: &[Val]) -> R {
    let xs = ratio_args(args, "-")?;
    match xs.as_slice() {
        [] => Err("-: needs at least one argument".into()),
        [(n, d)] => Val::make_ratio(-n, *d),
        [first, rest @ ..] => {
            let mut acc = *first;
            for r in rest {
                acc = ratio_sub(acc, *r, "-")?;
            }
            Val::make_ratio(acc.0, acc.1)
        }
    }
}

fn mul(args: &[Val]) -> R {
    let xs = ratio_args(args, "*")?;
    let mut acc: Ratio = (1, 1);
    for r in xs {
        acc = ratio_mul(acc, r, "*")?;
    }
    Val::make_ratio(acc.0, acc.1)
}

fn div(args: &[Val]) -> R {
    let xs = ratio_args(args, "/")?;
    match xs.as_slice() {
        [] | [_] => Err("/: needs at least two arguments".into()),
        [first, rest @ ..] => {
            let mut acc = *first;
            for r in rest {
                acc = ratio_div(acc, *r, "/")?;
            }
            Val::make_ratio(acc.0, acc.1)
        }
    }
}

fn modulo(args: &[Val]) -> R {
    match args {
        [Val::Num(a), Val::Num(b)] => {
            if *b == 0 {
                return Err("mod: division by zero".into());
            }
            // rem_euclid also panics on division overflow (i64::MIN % -1);
            // checked_rem_euclid returns None there instead of aborting.
            a.checked_rem_euclid(*b)
                .map(Val::Num)
                .ok_or_else(|| "mod: overflow".into())
        }
        _ => Err("mod: expected two integers".into()),
    }
}

// ---- comparison ----

fn cmp_chain(args: &[Val], name: &str, ok: fn(i128, i128) -> bool) -> R {
    let xs = ratio_args(args, name)?;
    // Compare a < b by cross-multiplying: a.n * b.d  ?  b.n * a.d.
    // Both denominators are positive (Val::make_ratio invariant), so
    // the cross-multiplied compare preserves direction.
    //
    // Unchecked multiplication is safe *given the current Val::Ratio
    // widths*, and only just: `Val::make_ratio` narrows to (i64, u64),
    // so |n| ≤ 2^63 and d ≤ 2^64 - 1, bounding each product at
    // 2^127 - 2^63 — inside i128 by one part in 2^64. Widen either
    // field and this overflows; switch to checked_mul if that day
    // comes. Every other arithmetic path here already is checked.
    for w in xs.windows(2) {
        let (an, ad) = w[0];
        let (bn, bd) = w[1];
        if !ok(an * bd, bn * ad) {
            return Ok(Val::Bool(false));
        }
    }
    Ok(Val::Bool(true))
}

fn eq_num(args: &[Val]) -> R {
    cmp_chain(args, "=", |a, b| a == b)
}
fn lt(args: &[Val]) -> R {
    cmp_chain(args, "<", |a, b| a < b)
}
fn gt(args: &[Val]) -> R {
    cmp_chain(args, ">", |a, b| a > b)
}
fn le(args: &[Val]) -> R {
    cmp_chain(args, "<=", |a, b| a <= b)
}
fn ge(args: &[Val]) -> R {
    cmp_chain(args, ">=", |a, b| a >= b)
}

fn not(args: &[Val]) -> R {
    Ok(Val::Bool(!args[0].is_truthy()))
}

fn eq_q(args: &[Val]) -> R {
    Ok(Val::Bool(args[0].eq_shallow(&args[1])))
}

// ---- list ops ----

fn cons(args: &[Val]) -> R {
    Ok(Val::cons(args[0].clone(), args[1].clone()))
}

fn car(args: &[Val]) -> R {
    match &args[0] {
        Val::Cons(h, _) => Ok((**h).clone()),
        other => Err(format!("car: expected pair, got {other}")),
    }
}

fn cdr(args: &[Val]) -> R {
    match &args[0] {
        Val::Cons(_, t) => Ok((**t).clone()),
        other => Err(format!("cdr: expected pair, got {other}")),
    }
}

fn list(args: &[Val]) -> R {
    Ok(Val::list_from(args))
}

fn append(args: &[Val]) -> R {
    // Concatenate proper lists left-to-right.
    let mut acc = Val::Nil;
    for arg in args.iter().rev() {
        let mut items: Vec<Val> = Vec::new();
        let mut cur = arg;
        loop {
            match cur {
                Val::Cons(h, t) => {
                    items.push((**h).clone());
                    cur = t;
                }
                Val::Nil => break,
                other => return Err(format!("append: expected list, got {other}")),
            }
        }
        for item in items.into_iter().rev() {
            acc = Val::cons(item, acc);
        }
    }
    Ok(acc)
}

fn null_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Nil)))
}

fn pair_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Cons(_, _))))
}

/// The `(error msg irritant …)` parts of a condition, or `None` if `v`
/// isn't that shape. Conditions are ordinary lists (ADR-041), so this is
/// a structural check and nothing stops a user from `raise`-ing a list
/// that looks like one — which is the usual Lisp bargain, and why these
/// three accessors are the supported way to read a condition rather than
/// the only way.
fn as_condition(v: &Val) -> Option<(&Val, &Val)> {
    let Val::Cons(head, tail) = v else {
        return None;
    };
    if !matches!(&**head, Val::Sym(s) if &**s == "error") {
        return None;
    }
    let Val::Cons(msg, irritants) = &**tail else {
        return None;
    };
    Some((msg, irritants))
}

fn error_q(args: &[Val]) -> R {
    Ok(Val::Bool(as_condition(&args[0]).is_some()))
}

fn error_message(args: &[Val]) -> R {
    match as_condition(&args[0]) {
        Some((msg, _)) => Ok(msg.clone()),
        None => Err(format!("error-message: not a condition: {}", args[0])),
    }
}

fn error_irritants(args: &[Val]) -> R {
    match as_condition(&args[0]) {
        Some((_, rest)) => Ok(rest.clone()),
        None => Err(format!("error-irritants: not a condition: {}", args[0])),
    }
}

fn number_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Num(_) | Val::Ratio(_, _))))
}

fn symbol_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Sym(_))))
}

// ---- strings ----

fn string_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Str(_))))
}

fn string_length(args: &[Val]) -> R {
    match &args[0] {
        Val::Str(s) => Ok(Val::Num(s.chars().count() as i64)),
        other => Err(format!("string-length: expected string, got {other}")),
    }
}

fn string_append(args: &[Val]) -> R {
    let mut out = String::new();
    for arg in args {
        match arg {
            Val::Str(s) => out.push_str(s),
            other => return Err(format!("string-append: expected string, got {other}")),
        }
    }
    Ok(Val::Str(out.into()))
}

fn string_to_symbol(args: &[Val]) -> R {
    match &args[0] {
        Val::Str(s) => Ok(Val::Sym(s.clone())),
        other => Err(format!("string->symbol: expected string, got {other}")),
    }
}

fn symbol_to_string(args: &[Val]) -> R {
    match &args[0] {
        Val::Sym(s) => Ok(Val::Str(s.clone())),
        other => Err(format!("symbol->string: expected symbol, got {other}")),
    }
}

fn number_to_string(args: &[Val]) -> R {
    match &args[0] {
        Val::Num(n) => Ok(Val::Str(n.to_string().into())),
        Val::Ratio(n, d) => Ok(Val::Str(format!("{n}/{d}").into())),
        other => Err(format!("number->string: expected number, got {other}")),
    }
}

// ---- rational accessors ----

fn numerator(args: &[Val]) -> R {
    match &args[0] {
        Val::Num(n) => Ok(Val::Num(*n)),
        Val::Ratio(n, _) => Ok(Val::Num(*n)),
        other => Err(format!("numerator: expected number, got {other}")),
    }
}

fn denominator(args: &[Val]) -> R {
    match &args[0] {
        Val::Num(_) => Ok(Val::Num(1)),
        Val::Ratio(_, d) => i64::try_from(*d)
            .map(Val::Num)
            .map_err(|_| format!("denominator: {d} doesn't fit in i64")),
        other => Err(format!("denominator: expected number, got {other}")),
    }
}

fn floor(args: &[Val]) -> R {
    floor_or_ceiling(&args[0], "floor", false)
}

fn ceiling(args: &[Val]) -> R {
    floor_or_ceiling(&args[0], "ceiling", true)
}

fn floor_or_ceiling(v: &Val, name: &str, ceil: bool) -> R {
    match v {
        Val::Num(n) => Ok(Val::Num(*n)),
        Val::Ratio(n, d) => {
            // Work in i128 so the u64 denominator fits without
            // sign-flip surprises. div_euclid rounds toward
            // negative infinity (Rust's "Euclidean division").
            let n = *n as i128;
            let d = *d as i128;
            let q = if ceil {
                -((-n).div_euclid(d))
            } else {
                n.div_euclid(d)
            };
            q.try_into()
                .map(Val::Num)
                .map_err(|_| format!("{name}: result {q} doesn't fit in i64"))
        }
        other => Err(format!("{name}: expected number, got {other}")),
    }
}

/// `(assoc-get key alist)` — walk `((k1 . v1) (k2 . v2) …)` and return
/// the value paired with the first key `eq?`-matching `key`, or `'()`
/// if not found. Matches the prelude closure both demos used to
/// hand-roll.
fn assoc_get(args: &[Val]) -> R {
    let key = &args[0];
    let mut cur = &args[1];
    while let Val::Cons(head, tail) = cur {
        if let Val::Cons(k, v) = head.as_ref()
            && k.eq_shallow(key)
        {
            return Ok((**v).clone());
        }
        cur = tail;
    }
    Ok(Val::Nil)
}

const BUILTINS: &[(&str, Arity, BuiltinFn)] = &[
    ("+", Arity::AtLeast(0), add),
    ("-", Arity::AtLeast(1), sub),
    ("*", Arity::AtLeast(0), mul),
    ("/", Arity::AtLeast(2), div),
    ("mod", Arity::Exact(2), modulo),
    ("=", Arity::AtLeast(2), eq_num),
    ("<", Arity::AtLeast(2), lt),
    (">", Arity::AtLeast(2), gt),
    ("<=", Arity::AtLeast(2), le),
    (">=", Arity::AtLeast(2), ge),
    ("not", Arity::Exact(1), not),
    ("eq?", Arity::Exact(2), eq_q),
    ("cons", Arity::Exact(2), cons),
    ("car", Arity::Exact(1), car),
    ("cdr", Arity::Exact(1), cdr),
    ("list", Arity::AtLeast(0), list),
    ("append", Arity::AtLeast(0), append),
    ("null?", Arity::Exact(1), null_q),
    ("pair?", Arity::Exact(1), pair_q),
    ("number?", Arity::Exact(1), number_q),
    ("symbol?", Arity::Exact(1), symbol_q),
    ("string?", Arity::Exact(1), string_q),
    ("string-length", Arity::Exact(1), string_length),
    ("string-append", Arity::AtLeast(0), string_append),
    ("string->symbol", Arity::Exact(1), string_to_symbol),
    ("symbol->string", Arity::Exact(1), symbol_to_string),
    ("number->string", Arity::Exact(1), number_to_string),
    ("assoc-get", Arity::Exact(2), assoc_get),
    ("numerator", Arity::Exact(1), numerator),
    ("denominator", Arity::Exact(1), denominator),
    ("floor", Arity::Exact(1), floor),
    ("ceiling", Arity::Exact(1), ceiling),
    ("error?", Arity::Exact(1), error_q),
    ("error-message", Arity::Exact(1), error_message),
    ("error-irritants", Arity::Exact(1), error_irritants),
];

/// A built-in as a *value*, for forms the compiler generates itself.
///
/// Compiler-generated code must not reach its operators through
/// `Expr::Var`: that is an ordinary lookup, so a user binding named
/// `list` silently changes what the generated form means. `(error …)`
/// built its condition that way, and `(let ((list …)) (error "boom"))`
/// returned the user's `list` result instead of raising; quasiquote had
/// the same bug against `list` and `append` since it was written.
/// Quoting the prim value makes those forms unshadowable — and costs
/// less at run time than the lookup did, since `Expr::Quote` is one `Rc`
/// clone.
///
/// Panics on an unknown name: callers pass literals, so a miss is a
/// compiler bug rather than anything a program can cause.
pub(crate) fn builtin(name: &str) -> Val {
    BUILTINS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|&(name, arity, f)| Val::Prim {
            name,
            arity,
            f: Rc::new(f),
        })
        .unwrap_or_else(|| panic!("compiler asked for a builtin that isn't registered: {name}"))
}

/// Seed the Vm's globals table with the built-in prims. Called once
/// at `Vm::new` time. Each prim lives in the same table as user-level
/// `(define …)` bindings, so a `(define + 5)` overwrites the slot and
/// the next `(+ 1 2)` errors with "not callable: 5" — see ADR-020.
pub fn install_builtins(globals: &crate::env::Globals) {
    let mut g = globals.borrow_mut();
    for &(name, arity, f) in BUILTINS {
        let val = Val::Prim {
            name,
            arity,
            f: Rc::new(f),
        };
        g.insert(name.into(), Rc::new(RefCell::new(val)));
    }
}
