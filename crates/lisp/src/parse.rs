use std::iter::Peekable;
use std::rc::Rc;
use std::vec::IntoIter;

use crate::expr::{Expr, Sym};
use crate::val::Val;

#[derive(Debug, Clone)]
enum Datum {
    Num(i64),
    Bool(bool),
    Sym(Sym),
    List(Vec<Datum>),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LParen,
    RParen,
    Quote,
    Num(i64),
    Bool(bool),
    Sym(String),
}

pub fn parse(src: &str) -> Result<Expr, String> {
    let toks = tokenize(src)?;
    let mut it = toks.into_iter().peekable();
    let d = read_datum(&mut it)?;
    if it.peek().is_some() {
        return Err("extra tokens after expression".into());
    }
    compile(&d)
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
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || matches!(c, '(' | ')' | ';' | '\'') {
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
    Tok::Sym(s.to_string())
}

fn read_datum(it: &mut Peekable<IntoIter<Tok>>) -> Result<Datum, String> {
    match it.next().ok_or_else(|| "unexpected eof".to_string())? {
        Tok::Num(n) => Ok(Datum::Num(n)),
        Tok::Bool(b) => Ok(Datum::Bool(b)),
        Tok::Sym(s) => Ok(Datum::Sym(s.into())),
        Tok::Quote => {
            // 'x  →  (quote x)
            let inner = read_datum(it)?;
            Ok(Datum::List(vec![Datum::Sym("quote".into()), inner]))
        }
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
                    _ => items.push(read_datum(it)?),
                }
            }
            Ok(Datum::List(items))
        }
    }
}

fn compile(d: &Datum) -> Result<Expr, String> {
    match d {
        Datum::Num(n) => Ok(Expr::Num(*n)),
        Datum::Bool(b) => Ok(Expr::Bool(*b)),
        Datum::Sym(s) => Ok(Expr::Var(s.clone())),
        Datum::List(items) => {
            if items.is_empty() {
                return Err("empty list".into());
            }
            if let Datum::Sym(head) = &items[0] {
                match &**head {
                    "lambda" | "λ" => return compile_lambda(&items[1..]),
                    "if" => return compile_if(&items[1..]),
                    "quote" => return compile_quote(&items[1..]),
                    "let" => return compile_let(&items[1..]),
                    "let*" => return compile_let_star(&items[1..]),
                    "letrec" => return compile_letrec(&items[1..]),
                    "cond" => return compile_cond(&items[1..]),
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

fn datum_to_val(d: &Datum) -> Val {
    match d {
        Datum::Num(n) => Val::Num(*n),
        Datum::Bool(b) => Val::Bool(*b),
        Datum::Sym(s) => Val::Sym(s.clone()),
        Datum::List(items) => {
            let vals: Vec<Val> = items.iter().map(datum_to_val).collect();
            Val::list_from(&vals)
        }
    }
}

/// `(let ((x e1) (y e2)) body)`  →  `((λ (x y) body) e1 e2)`
fn compile_let(rest: &[Datum]) -> Result<Expr, String> {
    let (names, inits, body) = split_bindings(rest, "let")?;
    let lam = Expr::Lam(names, Rc::new(body));
    let mut app = vec![Rc::new(lam)];
    app.extend(inits.into_iter().map(Rc::new));
    Ok(Expr::App(app))
}

/// `(let* ((x e1) (y e2)) body)`  →  `(let ((x e1)) (let ((y e2)) body))`
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
    let bindings: Vec<(Sym, Rc<Expr>)> =
        names.into_iter().zip(inits.into_iter().map(Rc::new)).collect();
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

/// `(cond (c1 e1) (c2 e2) (else en))`  →  `(if c1 e1 (if c2 e2 en))`
fn compile_cond(rest: &[Datum]) -> Result<Expr, String> {
    // Build from the back so the innermost else gets nested deepest.
    let mut tail: Expr = Expr::Bool(false); // unreachable fallthrough
    let mut saw_else = false;
    for clause in rest.iter().rev() {
        let items = match clause {
            Datum::List(items) if items.len() == 2 => items,
            _ => return Err("cond: clause must be (test expr)".into()),
        };
        let expr = compile(&items[1])?;
        if let Datum::Sym(s) = &items[0] {
            if &**s == "else" {
                if saw_else {
                    return Err("cond: multiple else clauses".into());
                }
                saw_else = true;
                tail = expr;
                continue;
            }
        }
        let test = compile(&items[0])?;
        tail = Expr::If(Rc::new(test), Rc::new(expr), Rc::new(tail));
    }
    Ok(tail)
}
