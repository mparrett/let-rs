use std::iter::Peekable;
use std::rc::Rc;
use std::vec::IntoIter;

use crate::expr::{Expr, Sym};
use crate::val::Val;

/// Intermediate s-expression form, used between tokenize and compile.
/// Public so the Vm can pre-process it (macro expansion) before compile sees it.
#[derive(Debug, Clone)]
pub enum Datum {
    Num(i64),
    /// Pre-normalization ratio as read from source. `compile` and
    /// `datum_to_val` route this through `Val::make_ratio` so the
    /// runtime always sees normalized form.
    Ratio(i64, u64),
    Bool(bool),
    Sym(Sym),
    Str(Rc<str>),
    List(Vec<Datum>),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LParen,
    RParen,
    Quote,
    Quasi,
    Unquote,
    UnquoteSplice,
    Num(i64),
    Ratio(i64, u64),
    Bool(bool),
    Sym(String),
    Str(String),
}

/// Read source → Datum (no compilation). Used by Vm so it can macro-expand
/// before producing an Expr. Errors if `src` contains more than one form.
pub fn read(src: &str) -> Result<Datum, String> {
    let toks = tokenize(src)?;
    let mut it = toks.into_iter().peekable();
    let d = read_datum(&mut it, 0)?;
    if it.peek().is_some() {
        return Err("extra tokens after expression".into());
    }
    Ok(d)
}

/// Read a sequence of top-level forms. Used by `Vm::eval_str` to accept
/// `(define …) (define …) (expr)` style sources without an outer `begin`
/// or `letrec` wrapper.
pub fn read_many(src: &str) -> Result<Vec<Datum>, String> {
    let toks = tokenize(src)?;
    let mut it = toks.into_iter().peekable();
    let mut out = Vec::new();
    while it.peek().is_some() {
        out.push(read_datum(&mut it, 0)?);
    }
    Ok(out)
}

/// Read + compile, no macro support. Vm::eval_str expands first and then
/// calls compile; this entry point exists for tests and contexts that don't
/// need macros.
pub fn parse(src: &str) -> Result<Expr, String> {
    compile(&read(src)?)
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            ';' => {
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '\n' {
                        break;
                    }
                }
            }
            '(' => {
                chars.next();
                toks.push(Tok::LParen);
            }
            ')' => {
                chars.next();
                toks.push(Tok::RParen);
            }
            '\'' => {
                chars.next();
                toks.push(Tok::Quote);
            }
            '`' => {
                chars.next();
                toks.push(Tok::Quasi);
            }
            ',' => {
                chars.next();
                if chars.peek() == Some(&'@') {
                    chars.next();
                    toks.push(Tok::UnquoteSplice);
                } else {
                    toks.push(Tok::Unquote);
                }
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => return Err("unclosed string literal".into()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('"') => s.push('"'),
                            Some('\\') => s.push('\\'),
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(c) => return Err(format!("unknown string escape \\{c}")),
                            None => return Err("unterminated string escape".into()),
                        },
                        Some(c) => s.push(c),
                    }
                }
                toks.push(Tok::Str(s));
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || matches!(c, '(' | ')' | ';' | '\'' | '`' | ',') {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                toks.push(classify(&s));
            }
        }
    }
    Ok(toks)
}

fn classify(s: &str) -> Tok {
    if s == "#t" || s == "true" {
        return Tok::Bool(true);
    }
    if s == "#f" || s == "false" {
        return Tok::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return Tok::Num(n);
    }
    // Rational literal: `<sign?digits>/<digits>` with non-zero den.
    // Anything else (e.g. `1/`, `/3`, `1/0`, `foo/bar`) stays a symbol.
    if let Some((num_s, den_s)) = s.split_once('/')
        && let Ok(num) = num_s.parse::<i64>()
        && let Ok(den) = den_s.parse::<u64>()
        && den > 0
    {
        return Tok::Ratio(num, den);
    }
    Tok::Sym(s.to_string())
}

/// Max structural nesting the reader will build. Deeply nested input like
/// `((((…` recurses `read_datum` once per level and overflows the native
/// (and, more tightly, the wasm) stack around a few tens of thousands deep —
/// a crash/DoS at the untrusted boundary rather than a clean error. This cap
/// aborts far below that yet sits well above any realistic hand-written or
/// generated source. Because every `Datum` originates here, bounding reader
/// depth transitively bounds `compile` / `datum_to_val` / `compile_qq` and the
/// macro expander's structural descent (a macro template can only combine
/// reader-built forms, never manufacture unbounded depth).
const MAX_DEPTH: usize = 1024;

fn read_datum(it: &mut Peekable<IntoIter<Tok>>, depth: usize) -> Result<Datum, String> {
    if depth > MAX_DEPTH {
        return Err("nesting too deep".into());
    }
    match it.next().ok_or_else(|| "unexpected eof".to_string())? {
        Tok::Num(n) => Ok(Datum::Num(n)),
        Tok::Ratio(num, den) => Ok(Datum::Ratio(num, den)),
        Tok::Bool(b) => Ok(Datum::Bool(b)),
        Tok::Sym(s) => Ok(Datum::Sym(s.into())),
        Tok::Str(s) => Ok(Datum::Str(s.into())),
        Tok::Quote => prefixed("quote", read_datum(it, depth + 1)?),
        Tok::Quasi => prefixed("quasiquote", read_datum(it, depth + 1)?),
        Tok::Unquote => prefixed("unquote", read_datum(it, depth + 1)?),
        Tok::UnquoteSplice => prefixed("unquote-splicing", read_datum(it, depth + 1)?),
        Tok::RParen => Err("unexpected )".into()),
        Tok::LParen => {
            let mut items = Vec::new();
            loop {
                match it.peek() {
                    None => return Err("unclosed (".into()),
                    Some(Tok::RParen) => {
                        it.next();
                        break;
                    }
                    _ => items.push(read_datum(it, depth + 1)?),
                }
            }
            Ok(Datum::List(items))
        }
    }
}

fn prefixed(head: &str, inner: Datum) -> Result<Datum, String> {
    Ok(Datum::List(vec![Datum::Sym(head.into()), inner]))
}

pub fn compile(d: &Datum) -> Result<Expr, String> {
    match d {
        Datum::Num(n) => Ok(Expr::Num(*n)),
        // Quote-wrap a normalized ratio so eval surfaces a Val::Ratio.
        // The reader already validated den != 0, so make_ratio shouldn't
        // fail here in practice; surface any constructor error directly.
        Datum::Ratio(n, d) => {
            let v = Val::make_ratio(*n as i128, *d as i128)?;
            Ok(Expr::Quote(Rc::new(v)))
        }
        Datum::Bool(b) => Ok(Expr::Bool(*b)),
        Datum::Sym(s) => Ok(Expr::Var(s.clone())),
        // Self-evaluating; quote-wrap so eval emits the Val::Str with a
        // single Rc clone per evaluation (same shape as Ratio).
        Datum::Str(s) => Ok(Expr::Quote(Rc::new(Val::Str(s.clone())))),
        Datum::List(items) => {
            // `()` stays invalid syntax in expression position (R7RS
            // agrees), but the bare "empty list" message left two
            // callers guessing. The REPL user wants to know that `'()`
            // is the literal; a macro author hits this when a macro
            // returns `Val::Nil`, because `val_to_datum` renders it as
            // `()` and there is no context at serialization time to
            // tell an evaluated position from a `(lambda () …)` binder
            // — so the macro has to emit `'(quote ())` instead.
            if items.is_empty() {
                return Err(
                    "empty list: () is not a valid expression — write '() for the empty list"
                        .into(),
                );
            }
            if let Datum::Sym(head) = &items[0] {
                match &**head {
                    "lambda" | "λ" => return compile_lambda(&items[1..]),
                    "if" => return compile_if(&items[1..]),
                    "quote" => return compile_quote(&items[1..]),
                    "quasiquote" => return compile_quasiquote_form(&items[1..]),
                    "let" => return compile_let(&items[1..]),
                    "let*" => return compile_let_star(&items[1..]),
                    "letrec" => return compile_letrec(&items[1..]),
                    "cond" => return compile_cond(&items[1..]),
                    "set!" => return compile_set_bang(&items[1..]),
                    "unquote" | "unquote-splicing" => {
                        return Err(format!("{head} outside of quasiquote"));
                    }
                    _ => {}
                }
            }
            let compiled: Result<Vec<_>, _> =
                items.iter().map(|i| compile(i).map(Rc::new)).collect();
            Ok(Expr::App(compiled?))
        }
    }
}

fn compile_lambda(rest: &[Datum]) -> Result<Expr, String> {
    if rest.len() != 2 {
        return Err("lambda: expected (lambda (params...) body)".into());
    }
    let params: Vec<Sym> = match &rest[0] {
        Datum::List(items) => items
            .iter()
            .map(|i| match i {
                Datum::Sym(s) => Ok(s.clone()),
                _ => Err::<Sym, String>("lambda: param must be a symbol".into()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("lambda: params must be a list".into()),
    };
    let body = compile(&rest[1])?;
    Ok(Expr::Lam(params, Rc::new(body)))
}

fn compile_if(rest: &[Datum]) -> Result<Expr, String> {
    if rest.len() != 3 {
        return Err("if: expected (if cond then else)".into());
    }
    let c = compile(&rest[0])?;
    let t = compile(&rest[1])?;
    let e = compile(&rest[2])?;
    Ok(Expr::If(Rc::new(c), Rc::new(t), Rc::new(e)))
}

fn compile_quote(rest: &[Datum]) -> Result<Expr, String> {
    if rest.len() != 1 {
        return Err("quote: expected (quote datum)".into());
    }
    Ok(Expr::Quote(Rc::new(datum_to_val(&rest[0]))))
}

pub fn datum_to_val(d: &Datum) -> Val {
    match d {
        Datum::Num(n) => Val::Num(*n),
        // A reader-validated ratio (den > 0) cannot fail to normalize —
        // unwrap is safe here. If make_ratio ever grows another failure
        // mode, this is where to revisit.
        Datum::Ratio(n, d) => Val::make_ratio(*n as i128, *d as i128)
            .expect("reader-validated ratio failed to normalize"),
        Datum::Bool(b) => Val::Bool(*b),
        Datum::Sym(s) => Val::Sym(s.clone()),
        Datum::Str(s) => Val::Str(s.clone()),
        Datum::List(items) => {
            let vals: Vec<Val> = items.iter().map(datum_to_val).collect();
            Val::list_from(&vals)
        }
    }
}

fn compile_let(rest: &[Datum]) -> Result<Expr, String> {
    let (names, inits, body) = split_bindings(rest, "let")?;
    let lam = Expr::Lam(names, Rc::new(body));
    let mut app = vec![Rc::new(lam)];
    app.extend(inits.into_iter().map(Rc::new));
    Ok(Expr::App(app))
}

fn compile_let_star(rest: &[Datum]) -> Result<Expr, String> {
    let (names, inits, body) = split_bindings(rest, "let*")?;
    let mut expr = body;
    for (name, init) in names.into_iter().zip(inits).rev() {
        let lam = Expr::Lam(vec![name], Rc::new(expr));
        expr = Expr::App(vec![Rc::new(lam), Rc::new(init)]);
    }
    Ok(expr)
}

fn compile_letrec(rest: &[Datum]) -> Result<Expr, String> {
    let (names, inits, body) = split_bindings(rest, "letrec")?;
    let bindings: Vec<(Sym, Rc<Expr>)> = names
        .into_iter()
        .zip(inits.into_iter().map(Rc::new))
        .collect();
    Ok(Expr::Letrec(bindings, Rc::new(body)))
}

fn split_bindings(rest: &[Datum], form: &str) -> Result<(Vec<Sym>, Vec<Expr>, Expr), String> {
    if rest.len() != 2 {
        return Err(format!("{form}: expected ({form} ((name init)...) body)"));
    }
    let pairs = match &rest[0] {
        Datum::List(items) => items,
        _ => return Err(format!("{form}: bindings must be a list")),
    };
    let mut names = Vec::with_capacity(pairs.len());
    let mut inits = Vec::with_capacity(pairs.len());
    for p in pairs {
        match p {
            Datum::List(pair) if pair.len() == 2 => {
                match &pair[0] {
                    Datum::Sym(s) => names.push(s.clone()),
                    _ => return Err(format!("{form}: binding name must be a symbol")),
                }
                inits.push(compile(&pair[1])?);
            }
            _ => return Err(format!("{form}: binding must be (name init)")),
        }
    }
    let body = compile(&rest[1])?;
    Ok((names, inits, body))
}

fn compile_set_bang(rest: &[Datum]) -> Result<Expr, String> {
    if rest.len() != 2 {
        return Err("set!: expected (set! name val)".into());
    }
    let name = match &rest[0] {
        Datum::Sym(s) => s.clone(),
        _ => return Err("set!: name must be a symbol".into()),
    };
    let val = compile(&rest[1])?;
    Ok(Expr::SetBang(name, Rc::new(val)))
}

fn compile_cond(rest: &[Datum]) -> Result<Expr, String> {
    // `else` is only legal as the final clause; rejecting mid-list else
    // here avoids the surprising "(cond (else 'wrong) (#t 'right))"
    // returning 'wrong.
    for (i, clause) in rest.iter().enumerate() {
        if let Datum::List(items) = clause
            && items.len() == 2
            && let Datum::Sym(s) = &items[0]
            && &**s == "else"
            && i + 1 != rest.len()
        {
            return Err("cond: else must be the final clause".into());
        }
    }

    let mut tail: Expr = Expr::Bool(false);
    for clause in rest.iter().rev() {
        let items = match clause {
            Datum::List(items) if items.len() == 2 => items,
            _ => return Err("cond: clause must be (test expr)".into()),
        };
        let expr = compile(&items[1])?;
        if let Datum::Sym(s) = &items[0]
            && &**s == "else"
        {
            tail = expr;
            continue;
        }
        let test = compile(&items[0])?;
        tail = Expr::If(Rc::new(test), Rc::new(expr), Rc::new(tail));
    }
    Ok(tail)
}

fn compile_quasiquote_form(rest: &[Datum]) -> Result<Expr, String> {
    if rest.len() != 1 {
        return Err("quasiquote: expected (quasiquote datum)".into());
    }
    compile_qq(&rest[0], 1)
}

/// Wrap an inner expression in `(list 'TAG INNER)` — builds a literal
/// `(TAG INNER)` cons at runtime. Used by `compile_qq` for nested
/// quasiquote / unquote / unquote-splicing forms where the head symbol
/// must be preserved verbatim rather than fired.
fn qq_wrap_form(tag: &'static str, inner: Expr) -> Expr {
    Expr::App(vec![
        Rc::new(Expr::Var("list".into())),
        Rc::new(Expr::Quote(Rc::new(Val::Sym(tag.into())))),
        Rc::new(inner),
    ])
}

/// Compile `(quasiquote DATUM)` to an Expr that constructs the value DATUM at
/// runtime, with `(unquote x)` becoming the eval of x and `(unquote-splicing xs)`
/// splicing xs (a list) into the surrounding list.
///
/// `depth` tracks quasiquote nesting (top-level call uses depth=1). Each
/// nested `(quasiquote …)` bumps depth; each `(unquote …)` / `(unquote-
/// splicing …)` reduces it. Escapes only fire when depth reaches 1, so
/// `` `(a `(b ,c)) `` keeps `,c` literal inside the inner quasiquote.
fn compile_qq(d: &Datum, depth: usize) -> Result<Expr, String> {
    match d {
        Datum::Num(_) | Datum::Ratio(_, _) | Datum::Bool(_) | Datum::Sym(_) | Datum::Str(_) => {
            Ok(Expr::Quote(Rc::new(datum_to_val(d))))
        }
        Datum::List(items) => {
            // Head-symbol forms (quasiquote / unquote / unquote-splicing).
            // unquote at depth==1 fires; deeper depths keep it literal at
            // depth-1. quasiquote bumps depth and stays literal.
            if items.len() == 2
                && let Datum::Sym(s) = &items[0]
            {
                match s.as_ref() {
                    "unquote" => {
                        return if depth == 1 {
                            compile(&items[1])
                        } else {
                            Ok(qq_wrap_form("unquote", compile_qq(&items[1], depth - 1)?))
                        };
                    }
                    "unquote-splicing" => {
                        return if depth == 1 {
                            Err("unquote-splicing at top of quasiquoted form".into())
                        } else {
                            Ok(qq_wrap_form(
                                "unquote-splicing",
                                compile_qq(&items[1], depth - 1)?,
                            ))
                        };
                    }
                    "quasiquote" => {
                        return Ok(qq_wrap_form(
                            "quasiquote",
                            compile_qq(&items[1], depth + 1)?,
                        ));
                    }
                    _ => {}
                }
            }

            // Splices only fire at depth 1. Deeper-nested unquote-splicing
            // is just a literal cons.
            let any_splice = depth == 1 && items.iter().any(is_splice);
            if !any_splice {
                let mut app = vec![Rc::new(Expr::Var("list".into()))];
                for item in items {
                    app.push(Rc::new(compile_qq(item, depth)?));
                }
                return Ok(Expr::App(app));
            }

            let mut parts: Vec<Rc<Expr>> = Vec::new();
            let mut bucket: Vec<Rc<Expr>> = Vec::new();
            let flush = |bucket: &mut Vec<Rc<Expr>>, parts: &mut Vec<Rc<Expr>>| {
                if !bucket.is_empty() {
                    let mut list_expr = vec![Rc::new(Expr::Var("list".into()))];
                    list_expr.append(bucket);
                    parts.push(Rc::new(Expr::App(list_expr)));
                }
            };
            for item in items {
                if let Some(inner) = splice_inner(item) {
                    flush(&mut bucket, &mut parts);
                    parts.push(Rc::new(compile(inner)?));
                } else {
                    bucket.push(Rc::new(compile_qq(item, depth)?));
                }
            }
            flush(&mut bucket, &mut parts);

            let mut app = vec![Rc::new(Expr::Var("append".into()))];
            app.extend(parts);
            Ok(Expr::App(app))
        }
    }
}

fn is_splice(d: &Datum) -> bool {
    splice_inner(d).is_some()
}

fn splice_inner(d: &Datum) -> Option<&Datum> {
    if let Datum::List(items) = d
        && items.len() == 2
        && let Datum::Sym(s) = &items[0]
        && &**s == "unquote-splicing"
    {
        return Some(&items[1]);
    }
    None
}
