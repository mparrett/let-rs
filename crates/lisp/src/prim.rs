use crate::env::Env;
use crate::val::{Arity, Val};

type R = Result<Val, String>;

// ---- arithmetic (variadic) ----

fn nums(args: &[Val], name: &str) -> Result<Vec<i64>, String> {
    args.iter()
        .map(|v| match v {
            Val::Num(n) => Ok(*n),
            other => Err(format!("{name}: expected number, got {other}")),
        })
        .collect()
}

fn add(args: &[Val]) -> R {
    Ok(Val::Num(nums(args, "+")?.iter().sum()))
}

fn sub(args: &[Val]) -> R {
    let ns = nums(args, "-")?;
    match ns.as_slice() {
        [] => Err("-: needs at least one argument".into()),
        [x] => Ok(Val::Num(-x)),
        [x, rest @ ..] => Ok(Val::Num(rest.iter().fold(*x, |a, b| a - b))),
    }
}

fn mul(args: &[Val]) -> R {
    Ok(Val::Num(nums(args, "*")?.iter().product()))
}

fn div(args: &[Val]) -> R {
    let ns = nums(args, "/")?;
    match ns.as_slice() {
        [] | [_] => Err("/: needs at least two arguments".into()),
        [x, rest @ ..] => {
            let mut acc = *x;
            for d in rest {
                if *d == 0 {
                    return Err("/: division by zero".into());
                }
                acc /= d;
            }
            Ok(Val::Num(acc))
        }
    }
}

fn modulo(args: &[Val]) -> R {
    match args {
        [Val::Num(a), Val::Num(b)] => {
            if *b == 0 {
                return Err("mod: division by zero".into());
            }
            Ok(Val::Num(a.rem_euclid(*b)))
        }
        _ => Err("mod: expected two numbers".into()),
    }
}

// ---- comparison ----

fn cmp_chain(args: &[Val], name: &str, ok: fn(i64, i64) -> bool) -> R {
    let ns = nums(args, name)?;
    for w in ns.windows(2) {
        if !ok(w[0], w[1]) {
            return Ok(Val::Bool(false));
        }
    }
    Ok(Val::Bool(true))
}

fn eq_num(args: &[Val]) -> R { cmp_chain(args, "=", |a, b| a == b) }
fn lt(args: &[Val]) -> R     { cmp_chain(args, "<", |a, b| a < b) }
fn gt(args: &[Val]) -> R     { cmp_chain(args, ">", |a, b| a > b) }
fn le(args: &[Val]) -> R     { cmp_chain(args, "<=", |a, b| a <= b) }
fn ge(args: &[Val]) -> R     { cmp_chain(args, ">=", |a, b| a >= b) }

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

fn number_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Num(_))))
}

fn symbol_q(args: &[Val]) -> R {
    Ok(Val::Bool(matches!(args[0], Val::Sym(_))))
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

const BUILTINS: &[(&str, Arity, fn(&[Val]) -> R)] = &[
    ("+",       Arity::AtLeast(0), add),
    ("-",       Arity::AtLeast(1), sub),
    ("*",       Arity::AtLeast(0), mul),
    ("/",       Arity::AtLeast(2), div),
    ("mod",     Arity::Exact(2),   modulo),
    ("=",       Arity::AtLeast(2), eq_num),
    ("<",       Arity::AtLeast(2), lt),
    (">",       Arity::AtLeast(2), gt),
    ("<=",      Arity::AtLeast(2), le),
    (">=",      Arity::AtLeast(2), ge),
    ("not",     Arity::Exact(1),   not),
    ("eq?",     Arity::Exact(2),   eq_q),
    ("cons",    Arity::Exact(2),   cons),
    ("car",     Arity::Exact(1),   car),
    ("cdr",     Arity::Exact(1),   cdr),
    ("list",    Arity::AtLeast(0), list),
    ("append",  Arity::AtLeast(0), append),
    ("null?",   Arity::Exact(1),   null_q),
    ("pair?",   Arity::Exact(1),   pair_q),
    ("number?", Arity::Exact(1),   number_q),
    ("symbol?", Arity::Exact(1),   symbol_q),
    ("assoc-get", Arity::Exact(2), assoc_get),
];

pub fn initial_env() -> Env {
    BUILTINS.iter().fold(Env::empty(), |env, &(name, arity, f)| {
        env.extend(name.into(), Val::Prim { name, arity, f })
    })
}
