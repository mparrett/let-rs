//! Rune-tape translation: Unicode rune characters → lisp list.
//!
//! A DSL layer on top of the [`lisp`](../lisp/index.html) crate. Each rune
//! is mapped to a primitive name; parametrized runes pair with the
//! immediately following numeral (e.g. `ᛊ 3` → `(area 3)`). The output is
//! a `(list …)` expression that callers wrap in their own spell pipeline
//! (`(thread (start) …)` for the in-Vm demo; `(world-apply! (thread …))`
//! for the resolver path used by the WASM bridge).
//!
//! The split keeps the lisp crate ignorant of runes (it shouldn't know
//! about a particular DSL surface) while letting every host of the spell
//! pipeline reuse one source of truth — see ADR-007 and ADR-010.

/// Plain runes: each maps to a unary `ctx → ctx` primitive name.
pub const PLAIN: &[(char, &str)] = &[
    ('ᚠ', "fire"), // FEHU
    ('ᛁ', "ice"),  // ISA
    ('ᚱ', "bolt"), // RAIDO
    ('ᛒ', "self"), // BERKANO — target self
];

/// Parametrized runes: each maps to `n → ctx → ctx`. Consumes the
/// immediately following numeral.
pub const PARAM: &[(char, &str)] = &[
    ('ᛊ', "area"),  // SOWILO
    ('ᛏ', "power"), // TIWAZ
];

#[derive(Debug, Clone)]
enum Tok {
    Plain(&'static str),
    Param(&'static str),
    Num(i64),
}

fn lex(tape: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = tape.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c.is_ascii_digit() {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if !c.is_ascii_digit() {
                    break;
                }
                s.push(c);
                chars.next();
            }
            let n: i64 = s
                .parse()
                .map_err(|_| format!("rune number out of i64 range: {s}"))?;
            out.push(Tok::Num(n));
            continue;
        }
        if let Some(&(_, name)) = PLAIN.iter().find(|(k, _)| *k == c) {
            out.push(Tok::Plain(name));
        } else if let Some(&(_, name)) = PARAM.iter().find(|(k, _)| *k == c) {
            out.push(Tok::Param(name));
        } else {
            return Err(format!("unknown rune: '{c}'"));
        }
        chars.next();
    }
    Ok(out)
}

fn resolve(toks: &[Tok]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Plain(name) => {
                out.push((*name).into());
                i += 1;
            }
            Tok::Param(name) => match toks.get(i + 1) {
                Some(Tok::Num(n)) => {
                    out.push(format!("({name} {n})"));
                    i += 2;
                }
                _ => return Err(format!("rune '{name}' expects a number to follow")),
            },
            Tok::Num(n) => return Err(format!("stray number with no parametrized rune: {n}")),
        }
    }
    Ok(out)
}

/// Translate a rune tape into a `(list …)` expression suitable for splicing
/// into a spell pipeline. An empty tape yields `(list)`. Returns an `Err`
/// for unknown runes, stray numbers, or parametrized runes with no number.
pub fn tape_to_sexpr(tape: &str) -> Result<String, String> {
    let toks = lex(tape)?;
    let parts = resolve(&toks)?;
    Ok(format!("(list {})", parts.join(" ")))
}
