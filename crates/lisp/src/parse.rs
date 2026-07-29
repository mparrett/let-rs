use std::iter::Peekable;
use std::rc::Rc;
use std::vec::IntoIter;

use crate::error::{LispErr, Span};
use crate::expr::{Expr, Sym};
use crate::val::Val;

/// Intermediate s-expression form, used between tokenize and compile.
/// Public so the Vm can pre-process it (macro expansion) before compile sees it.
///
/// The `kind`/`span` split (ADR-039) is what lets a compile error name a
/// line and column. `span` is `Some` for anything the reader built and
/// `None` for anything synthesized — a macro expansion, or a form a host
/// assembled programmatically. Macro output that wants positions borrows
/// the call site's span, which is why `Datum::list` and friends take one
/// explicitly rather than defaulting to `None`.
#[derive(Debug, Clone)]
pub struct Datum {
    pub kind: DatumKind,
    pub span: Option<Span>,
}

#[derive(Debug, Clone)]
pub enum DatumKind {
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

impl Datum {
    pub fn new(kind: DatumKind, span: Option<Span>) -> Datum {
        Datum { kind, span }
    }

    /// A datum with no source position — macro output, host-built forms.
    pub fn bare(kind: DatumKind) -> Datum {
        Datum { kind, span: None }
    }

    pub fn num(n: i64, span: Option<Span>) -> Datum {
        Datum::new(DatumKind::Num(n), span)
    }

    pub fn bool(b: bool, span: Option<Span>) -> Datum {
        Datum::new(DatumKind::Bool(b), span)
    }

    pub fn sym(name: impl Into<Sym>, span: Option<Span>) -> Datum {
        Datum::new(DatumKind::Sym(name.into()), span)
    }

    pub fn str(s: impl Into<Rc<str>>, span: Option<Span>) -> Datum {
        Datum::new(DatumKind::Str(s.into()), span)
    }

    pub fn list(items: Vec<Datum>, span: Option<Span>) -> Datum {
        Datum::new(DatumKind::List(items), span)
    }

    /// The items of a list datum, or `None` for an atom. Convenience for
    /// the many callers that only care about the list case.
    pub fn as_list(&self) -> Option<&[Datum]> {
        match &self.kind {
            DatumKind::List(items) => Some(items),
            _ => None,
        }
    }

    /// The name of a symbol datum, or `None` for anything else.
    pub fn as_sym(&self) -> Option<&Sym> {
        match &self.kind {
            DatumKind::Sym(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum TokKind {
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

#[derive(Debug, Clone, PartialEq)]
struct Tok {
    kind: TokKind,
    span: Span,
}

/// Read source → Datum (no compilation). Used by Vm so it can macro-expand
/// before producing an Expr. Errors if `src` contains more than one form.
pub fn read(src: &str) -> Result<Datum, LispErr> {
    let (toks, eof) = tokenize(src)?;
    let mut it = toks.into_iter().peekable();
    let d = read_datum(&mut it, eof)?;
    if let Some(extra) = it.peek() {
        return Err(LispErr::at("extra tokens after expression", extra.span));
    }
    Ok(d)
}

/// Read a sequence of top-level forms. Used by `Vm::eval_str` to accept
/// `(define …) (define …) (expr)` style sources without an outer `begin`
/// or `letrec` wrapper.
pub fn read_many(src: &str) -> Result<Vec<Datum>, LispErr> {
    let (toks, eof) = tokenize(src)?;
    let mut it = toks.into_iter().peekable();
    let mut out = Vec::new();
    while it.peek().is_some() {
        out.push(read_datum(&mut it, eof)?);
    }
    Ok(out)
}

/// Read + compile, no macro support. Vm::eval_str expands first and then
/// calls compile; this entry point exists for tests and contexts that don't
/// need macros.
pub fn parse(src: &str) -> Result<Expr, LispErr> {
    compile(&read(src)?)
}

/// Tracks line/column while scanning so every token can carry its
/// position. Columns count characters rather than bytes, so a caret
/// rendered under a trigram or rune literal lands in the right place.
struct Cursor<'a> {
    chars: Peekable<std::str::Chars<'a>>,
    line: u32,
    col: u32,
}

impl<'a> Cursor<'a> {
    fn new(src: &'a str) -> Cursor<'a> {
        Cursor {
            chars: src.chars().peekable(),
            line: 1,
            col: 1,
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// The position the cursor is *about* to read. Zero-length spans are
    /// widened to 1 by the renderer, so an end-of-input span still draws
    /// a caret.
    fn here(&self) -> Span {
        Span::new(self.line, self.col, 1)
    }
}

/// Tokenize `src`, returning the tokens plus a span pointing just past
/// the end of input. The latter is what an `unexpected eof` error uses:
/// "the form that starts here never closed" wants the opening paren, but
/// "input stopped early" wants the end.
fn tokenize(src: &str) -> Result<(Vec<Tok>, Span), LispErr> {
    let mut toks = Vec::new();
    let mut cur = Cursor::new(src);
    while let Some(c) = cur.peek() {
        let start = cur.here();
        match c {
            c if c.is_whitespace() => {
                cur.next();
            }
            ';' => {
                while let Some(c) = cur.next() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '(' => {
                cur.next();
                toks.push(Tok {
                    kind: TokKind::LParen,
                    span: start,
                });
            }
            ')' => {
                cur.next();
                toks.push(Tok {
                    kind: TokKind::RParen,
                    span: start,
                });
            }
            '\'' => {
                cur.next();
                toks.push(Tok {
                    kind: TokKind::Quote,
                    span: start,
                });
            }
            '`' => {
                cur.next();
                toks.push(Tok {
                    kind: TokKind::Quasi,
                    span: start,
                });
            }
            ',' => {
                cur.next();
                if cur.peek() == Some('@') {
                    cur.next();
                    toks.push(Tok {
                        kind: TokKind::UnquoteSplice,
                        span: Span::new(start.line, start.col, 2),
                    });
                } else {
                    toks.push(Tok {
                        kind: TokKind::Unquote,
                        span: start,
                    });
                }
            }
            '"' => {
                cur.next();
                let mut s = String::new();
                loop {
                    let esc_at = cur.here();
                    match cur.next() {
                        // The span points at the opening quote, not the
                        // end of input: the actionable position is where
                        // the literal started.
                        None => {
                            return Err(LispErr::at("unclosed string literal", start));
                        }
                        Some('"') => break,
                        Some('\\') => match cur.next() {
                            Some('"') => s.push('"'),
                            Some('\\') => s.push('\\'),
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(c) => {
                                return Err(LispErr::at(
                                    format!("unknown string escape \\{c}"),
                                    Span::new(esc_at.line, esc_at.col, 2),
                                ));
                            }
                            None => {
                                return Err(LispErr::at("unterminated string escape", esc_at));
                            }
                        },
                        Some(c) => s.push(c),
                    }
                }
                toks.push(Tok {
                    kind: TokKind::Str(s),
                    span: span_to(start, &cur),
                });
            }
            _ => {
                let mut s = String::new();
                while let Some(c) = cur.peek() {
                    if c.is_whitespace() || matches!(c, '(' | ')' | ';' | '\'' | '`' | ',') {
                        break;
                    }
                    s.push(c);
                    cur.next();
                }
                toks.push(Tok {
                    kind: classify(&s),
                    span: Span::new(start.line, start.col, s.chars().count() as u32),
                });
            }
        }
    }
    Ok((toks, cur.here()))
}

/// A span from `start` up to wherever the cursor now sits. Only used for
/// string literals, which are the one token whose text isn't retained
/// verbatim (escapes are decoded), so its length can't be recomputed.
/// A literal containing a newline reports its length as the distance on
/// the closing line, which is the best a line:col span can do.
fn span_to(start: Span, cur: &Cursor<'_>) -> Span {
    if cur.line == start.line {
        Span::new(start.line, start.col, cur.col.saturating_sub(start.col))
    } else {
        start
    }
}

fn classify(s: &str) -> TokKind {
    if s == "#t" || s == "true" {
        return TokKind::Bool(true);
    }
    if s == "#f" || s == "false" {
        return TokKind::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return TokKind::Num(n);
    }
    // Rational literal: `<sign?digits>/<digits>` with non-zero den.
    // Anything else (e.g. `1/`, `/3`, `1/0`, `foo/bar`) stays a symbol.
    if let Some((num_s, den_s)) = s.split_once('/')
        && let Ok(num) = num_s.parse::<i64>()
        && let Ok(den) = den_s.parse::<u64>()
        && den > 0
    {
        return TokKind::Ratio(num, den);
    }
    TokKind::Sym(s.to_string())
}

/// Max structural nesting the reader will accept. Deeply nested input like
/// `((((…` is a crash/DoS vector at the untrusted boundary, so it has to
/// abort cleanly rather than exhaust a stack. Because every `Datum`
/// originates here, bounding reader depth transitively bounds `compile` /
/// `datum_to_val` / `compile_qq` and the macro expander's structural
/// descent (a macro template can only combine reader-built forms, never
/// manufacture unbounded depth).
///
/// This cap used to also *be* the stack guard, because `read_datum`
/// recursed once per level. That coupled the amount of source the reader
/// accepts to the size of one debug-build stack frame: adding spans grew
/// the frame by roughly half and 1024 levels stopped fitting in the 2 MiB
/// a Rust test thread gets, which showed up as
/// `deeply_nested_input_errors_instead_of_overflowing` aborting the test
/// binary. `read_datum` is now iterative, so this bounds a heap `Vec`
/// instead and the two concerns are separate.
const MAX_DEPTH: usize = 1024;

/// An open construct the reader is still filling.
enum Open {
    /// An unclosed `(`, with the datums read so far.
    List { items: Vec<Datum>, span: Span },
    /// A reader prefix (`'`, `` ` ``, `,`, `,@`) waiting for the datum it
    /// applies to.
    Prefix { head: &'static str, span: Span },
}

/// Read one datum, iteratively.
///
/// The explicit `open` stack replaces recursion — both for lists and for
/// the reader prefixes, which nest just as freely (`''''x`). Its depth
/// *is* the nesting depth, so it doubles as the [`MAX_DEPTH`] counter that
/// used to be a parameter.
fn read_datum(it: &mut Peekable<IntoIter<Tok>>, eof: Span) -> Result<Datum, LispErr> {
    let mut open: Vec<Open> = Vec::new();

    loop {
        if open.len() > MAX_DEPTH {
            let at = it.peek().map(|t| t.span).unwrap_or(eof);
            return Err(LispErr::at("nesting too deep", at));
        }

        let tok = match it.next() {
            Some(t) => t,
            // Out of input with something still open. The innermost
            // construct is the one to name — which is what the recursive
            // version reported too, since its innermost frame errored
            // first. An unclosed paren is worth pointing at; a dangling
            // prefix has nothing better than the end of input.
            None => {
                return Err(match open.last() {
                    // ADR-022's motivating example: a multi-line form with
                    // a missing close paren used to say `unclosed (` and
                    // nothing else.
                    Some(Open::List { span, .. }) => LispErr::at("unclosed (", *span),
                    Some(Open::Prefix { .. }) | None => LispErr::at("unexpected eof", eof),
                });
            }
        };

        let at = Some(tok.span);
        // Either we produce a complete datum, or we push an open
        // construct and go read what goes inside it.
        let mut value = match tok.kind {
            TokKind::Num(n) => Datum::num(n, at),
            TokKind::Ratio(num, den) => Datum::new(DatumKind::Ratio(num, den), at),
            TokKind::Bool(b) => Datum::bool(b, at),
            TokKind::Sym(s) => Datum::sym(s, at),
            TokKind::Str(s) => Datum::str(s, at),
            TokKind::LParen => {
                open.push(Open::List {
                    items: Vec::new(),
                    span: tok.span,
                });
                continue;
            }
            TokKind::Quote => {
                open.push(prefix_frame("quote", tok.span));
                continue;
            }
            TokKind::Quasi => {
                open.push(prefix_frame("quasiquote", tok.span));
                continue;
            }
            TokKind::Unquote => {
                open.push(prefix_frame("unquote", tok.span));
                continue;
            }
            TokKind::UnquoteSplice => {
                open.push(prefix_frame("unquote-splicing", tok.span));
                continue;
            }
            TokKind::RParen => match open.pop() {
                Some(Open::List { items, span }) => Datum::list(items, Some(span)),
                // `(')` — a prefix with no datum after it. The recursive
                // reader reported this the same way, from the frame that
                // was waiting on the quoted form.
                Some(Open::Prefix { .. }) | None => {
                    return Err(LispErr::at("unexpected )", tok.span));
                }
            },
        };

        // Hand the finished datum to whatever was waiting for it,
        // completing any chain of pending prefixes on the way.
        loop {
            match open.last_mut() {
                Some(Open::List { items, .. }) => {
                    items.push(value);
                    break;
                }
                Some(Open::Prefix { .. }) => {
                    let Some(Open::Prefix { head, span }) = open.pop() else {
                        unreachable!("just matched a Prefix");
                    };
                    value = prefixed(head, value, span);
                }
                // Nothing is open: this datum is the whole form.
                None => return Ok(value),
            }
        }
    }
}

fn prefix_frame(head: &'static str, span: Span) -> Open {
    Open::Prefix { head, span }
}

/// Desugar `'x` / `` `x `` / `,x` / `,@x` into the two-element list form.
/// The synthesized list takes the *prefix token's* span, so an error in
/// `'(1 . 2)` points at the quote rather than at nothing.
fn prefixed(head: &str, inner: Datum, span: Span) -> Datum {
    Datum::list(vec![Datum::sym(head, Some(span)), inner], Some(span))
}

pub fn compile(d: &Datum) -> Result<Expr, LispErr> {
    // One annotation point for the whole compiler. `with_span` only fills
    // an empty span, so an error raised while compiling a subform keeps
    // that subform's position as it passes through each enclosing form on
    // the way out — and every helper below gets positions for free
    // without threading spans through its own error sites.
    compile_inner(d).map_err(|e| e.with_span(d.span))
}

fn compile_inner(d: &Datum) -> Result<Expr, LispErr> {
    match &d.kind {
        DatumKind::Num(n) => Ok(Expr::Num(*n)),
        // Quote-wrap a normalized ratio so eval surfaces a Val::Ratio.
        // The reader already validated den != 0, so make_ratio shouldn't
        // fail here in practice; surface any constructor error directly.
        DatumKind::Ratio(n, den) => {
            let v = Val::make_ratio(*n as i128, *den as i128)?;
            Ok(Expr::Quote(Rc::new(v)))
        }
        DatumKind::Bool(b) => Ok(Expr::Bool(*b)),
        DatumKind::Sym(s) => Ok(Expr::Var(s.clone(), d.span)),
        // Self-evaluating; quote-wrap so eval emits the Val::Str with a
        // single Rc clone per evaluation (same shape as Ratio).
        DatumKind::Str(s) => Ok(Expr::Quote(Rc::new(Val::Str(s.clone())))),
        DatumKind::List(items) => {
            // `()` stays invalid syntax in expression position (R7RS
            // agrees), but the bare "empty list" message left two
            // callers guessing. The REPL user wants to know that `'()`
            // is the literal; a macro author hits this when a macro
            // returns `Val::Nil`, because a `Val::Nil` expansion becomes
            // an empty list datum and there is no context at conversion
            // time to tell an evaluated position from a `(lambda () …)`
            // binder — so the macro has to emit `'(quote ())` instead.
            if items.is_empty() {
                return Err(LispErr::new(
                    "empty list: () is not a valid expression — write '() for the empty list",
                ));
            }
            if let DatumKind::Sym(head) = &items[0].kind {
                match &**head {
                    "lambda" | "λ" => return compile_lambda(&items[1..]),
                    "if" => return compile_if(&items[1..]),
                    "quote" => return compile_quote(&items[1..]),
                    "quasiquote" => return compile_quasiquote_form(&items[1..]),
                    "let" => return compile_let(&items[1..], d.span),
                    "let*" => return compile_let_star(&items[1..], d.span),
                    "letrec" => return compile_letrec(&items[1..]),
                    "cond" => return compile_cond(&items[1..]),
                    "set!" => return compile_set_bang(&items[1..]),
                    "unquote" | "unquote-splicing" => {
                        return Err(LispErr::new(format!("{head} outside of quasiquote")));
                    }
                    _ => {}
                }
            }
            let compiled: Result<Vec<_>, _> =
                items.iter().map(|i| compile(i).map(Rc::new)).collect();
            Ok(Expr::App(compiled?.into(), d.span))
        }
    }
}

fn compile_lambda(rest: &[Datum]) -> Result<Expr, LispErr> {
    if rest.len() != 2 {
        return Err(LispErr::new("lambda: expected (lambda (params...) body)"));
    }
    let params: Vec<Sym> = match &rest[0].kind {
        DatumKind::List(items) => items
            .iter()
            .map(|i| {
                i.as_sym()
                    .cloned()
                    .ok_or_else(|| LispErr::maybe_at("lambda: param must be a symbol", i.span))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(LispErr::maybe_at(
                "lambda: params must be a list",
                rest[0].span,
            ));
        }
    };
    let body = compile(&rest[1])?;
    Ok(Expr::Lam(params.into(), Rc::new(body)))
}

fn compile_if(rest: &[Datum]) -> Result<Expr, LispErr> {
    if rest.len() != 3 {
        return Err(LispErr::new("if: expected (if cond then else)"));
    }
    let c = compile(&rest[0])?;
    let t = compile(&rest[1])?;
    let e = compile(&rest[2])?;
    Ok(Expr::If(Rc::new(c), Rc::new(t), Rc::new(e)))
}

fn compile_quote(rest: &[Datum]) -> Result<Expr, LispErr> {
    if rest.len() != 1 {
        return Err(LispErr::new("quote: expected (quote datum)"));
    }
    Ok(Expr::Quote(Rc::new(datum_to_val(&rest[0]))))
}

pub fn datum_to_val(d: &Datum) -> Val {
    match &d.kind {
        DatumKind::Num(n) => Val::Num(*n),
        // A reader-validated ratio (den > 0) cannot fail to normalize —
        // unwrap is safe here. If make_ratio ever grows another failure
        // mode, this is where to revisit.
        DatumKind::Ratio(n, den) => Val::make_ratio(*n as i128, *den as i128)
            .expect("reader-validated ratio failed to normalize"),
        DatumKind::Bool(b) => Val::Bool(*b),
        DatumKind::Sym(s) => Val::Sym(s.clone()),
        DatumKind::Str(s) => Val::Str(s.clone()),
        DatumKind::List(items) => {
            let vals: Vec<Val> = items.iter().map(datum_to_val).collect();
            Val::list_from(&vals)
        }
    }
}

fn compile_let(rest: &[Datum], span: Option<Span>) -> Result<Expr, LispErr> {
    let (names, inits, body) = split_bindings(rest, "let")?;
    let lam = Expr::Lam(names.into(), Rc::new(body));
    let mut app = vec![Rc::new(lam)];
    app.extend(inits.into_iter().map(Rc::new));
    Ok(Expr::App(app.into(), span))
}

fn compile_let_star(rest: &[Datum], span: Option<Span>) -> Result<Expr, LispErr> {
    let (names, inits, body) = split_bindings(rest, "let*")?;
    let mut expr = body;
    for (name, init) in names.into_iter().zip(inits).rev() {
        let lam = Expr::Lam([name].into(), Rc::new(expr));
        expr = Expr::App([Rc::new(lam), Rc::new(init)].into(), span);
    }
    Ok(expr)
}

fn compile_letrec(rest: &[Datum]) -> Result<Expr, LispErr> {
    let (names, inits, body) = split_bindings(rest, "letrec")?;
    let bindings: Vec<(Sym, Rc<Expr>)> = names
        .into_iter()
        .zip(inits.into_iter().map(Rc::new))
        .collect();
    Ok(Expr::Letrec(bindings, Rc::new(body)))
}

fn split_bindings(rest: &[Datum], form: &str) -> Result<(Vec<Sym>, Vec<Expr>, Expr), LispErr> {
    if rest.len() != 2 {
        return Err(LispErr::new(format!(
            "{form}: expected ({form} ((name init)...) body)"
        )));
    }
    let pairs = rest[0].as_list().ok_or_else(|| {
        LispErr::maybe_at(format!("{form}: bindings must be a list"), rest[0].span)
    })?;
    let mut names = Vec::with_capacity(pairs.len());
    let mut inits = Vec::with_capacity(pairs.len());
    for p in pairs {
        match p.as_list() {
            Some(pair) if pair.len() == 2 => {
                names.push(pair[0].as_sym().cloned().ok_or_else(|| {
                    LispErr::maybe_at(
                        format!("{form}: binding name must be a symbol"),
                        pair[0].span,
                    )
                })?);
                inits.push(compile(&pair[1])?);
            }
            _ => {
                return Err(LispErr::maybe_at(
                    format!("{form}: binding must be (name init)"),
                    p.span,
                ));
            }
        }
    }
    let body = compile(&rest[1])?;
    Ok((names, inits, body))
}

fn compile_set_bang(rest: &[Datum]) -> Result<Expr, LispErr> {
    if rest.len() != 2 {
        return Err(LispErr::new("set!: expected (set! name val)"));
    }
    let name = rest[0]
        .as_sym()
        .cloned()
        .ok_or_else(|| LispErr::maybe_at("set!: name must be a symbol", rest[0].span))?;
    let val = compile(&rest[1])?;
    Ok(Expr::SetBang(name, Rc::new(val)))
}

fn compile_cond(rest: &[Datum]) -> Result<Expr, LispErr> {
    // `else` is only legal as the final clause; rejecting mid-list else
    // here avoids the surprising "(cond (else 'wrong) (#t 'right))"
    // returning 'wrong.
    for (i, clause) in rest.iter().enumerate() {
        if let Some(items) = clause.as_list()
            && items.len() == 2
            && let Some(s) = items[0].as_sym()
            && &**s == "else"
            && i + 1 != rest.len()
        {
            return Err(LispErr::maybe_at(
                "cond: else must be the final clause",
                clause.span,
            ));
        }
    }

    let mut tail: Expr = Expr::Bool(false);
    for clause in rest.iter().rev() {
        let items = match clause.as_list() {
            Some(items) if items.len() == 2 => items,
            _ => {
                return Err(LispErr::maybe_at(
                    "cond: clause must be (test expr)",
                    clause.span,
                ));
            }
        };
        let expr = compile(&items[1])?;
        if let Some(s) = items[0].as_sym()
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

fn compile_quasiquote_form(rest: &[Datum]) -> Result<Expr, LispErr> {
    if rest.len() != 1 {
        return Err(LispErr::new("quasiquote: expected (quasiquote datum)"));
    }
    compile_qq(&rest[0], 1)
}

/// Wrap an inner expression in `(list 'TAG INNER)` — builds a literal
/// `(TAG INNER)` cons at runtime. Used by `compile_qq` for nested
/// quasiquote / unquote / unquote-splicing forms where the head symbol
/// must be preserved verbatim rather than fired.
fn qq_wrap_form(tag: &'static str, inner: Expr, span: Option<Span>) -> Expr {
    Expr::App(
        [
            Rc::new(Expr::Var("list".into(), span)),
            Rc::new(Expr::Quote(Rc::new(Val::Sym(tag.into())))),
            Rc::new(inner),
        ]
        .into(),
        span,
    )
}

/// Compile `(quasiquote DATUM)` to an Expr that constructs the value DATUM at
/// runtime, with `(unquote x)` becoming the eval of x and `(unquote-splicing xs)`
/// splicing xs (a list) into the surrounding list.
///
/// `depth` tracks quasiquote nesting (top-level call uses depth=1). Each
/// nested `(quasiquote …)` bumps depth; each `(unquote …)` / `(unquote-
/// splicing …)` reduces it. Escapes only fire when depth reaches 1, so
/// `` `(a `(b ,c)) `` keeps `,c` literal inside the inner quasiquote.
fn compile_qq(d: &Datum, depth: usize) -> Result<Expr, LispErr> {
    let span = d.span;
    match &d.kind {
        DatumKind::Num(_)
        | DatumKind::Ratio(_, _)
        | DatumKind::Bool(_)
        | DatumKind::Sym(_)
        | DatumKind::Str(_) => Ok(Expr::Quote(Rc::new(datum_to_val(d)))),
        DatumKind::List(items) => {
            // Head-symbol forms (quasiquote / unquote / unquote-splicing).
            // unquote at depth==1 fires; deeper depths keep it literal at
            // depth-1. quasiquote bumps depth and stays literal.
            if items.len() == 2
                && let Some(s) = items[0].as_sym()
            {
                match s.as_ref() {
                    "unquote" => {
                        return if depth == 1 {
                            compile(&items[1])
                        } else {
                            Ok(qq_wrap_form(
                                "unquote",
                                compile_qq(&items[1], depth - 1)?,
                                span,
                            ))
                        };
                    }
                    "unquote-splicing" => {
                        return if depth == 1 {
                            Err(LispErr::maybe_at(
                                "unquote-splicing at top of quasiquoted form",
                                span,
                            ))
                        } else {
                            Ok(qq_wrap_form(
                                "unquote-splicing",
                                compile_qq(&items[1], depth - 1)?,
                                span,
                            ))
                        };
                    }
                    "quasiquote" => {
                        return Ok(qq_wrap_form(
                            "quasiquote",
                            compile_qq(&items[1], depth + 1)?,
                            span,
                        ));
                    }
                    _ => {}
                }
            }

            // Splices only fire at depth 1. Deeper-nested unquote-splicing
            // is just a literal cons.
            let any_splice = depth == 1 && items.iter().any(is_splice);
            if !any_splice {
                let mut app = vec![Rc::new(Expr::Var("list".into(), span))];
                for item in items {
                    app.push(Rc::new(compile_qq(item, depth)?));
                }
                return Ok(Expr::App(app.into(), span));
            }

            let mut parts: Vec<Rc<Expr>> = Vec::new();
            let mut bucket: Vec<Rc<Expr>> = Vec::new();
            let flush = |bucket: &mut Vec<Rc<Expr>>, parts: &mut Vec<Rc<Expr>>| {
                if !bucket.is_empty() {
                    let mut list_expr = vec![Rc::new(Expr::Var("list".into(), span))];
                    list_expr.append(bucket);
                    parts.push(Rc::new(Expr::App(list_expr.into(), span)));
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

            let mut app = vec![Rc::new(Expr::Var("append".into(), span))];
            app.extend(parts);
            Ok(Expr::App(app.into(), span))
        }
    }
}

fn is_splice(d: &Datum) -> bool {
    splice_inner(d).is_some()
}

fn splice_inner(d: &Datum) -> Option<&Datum> {
    if let Some(items) = d.as_list()
        && items.len() == 2
        && let Some(s) = items[0].as_sym()
        && &**s == "unquote-splicing"
    {
        return Some(&items[1]);
    }
    None
}
