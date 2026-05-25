//! End-to-end spell DSL demo: rune tape → sexpr → CEK eval → final ctx.
//!
//! The rune tape is the player-facing surface. A tiny Rust translator turns
//! each Unicode rune into a primitive name (and pairs parametrized runes with
//! the following number), wraps the result in `(thread (start) (list …))`,
//! and feeds it to the lisp Vm with a spell prelude already in scope.
//!
//! Nothing about this demo touches the engine — the prelude is plain user-
//! level lisp, the primitives are closures over `assoc-set`, and the rune
//! table is two `&[(char, &str)]` slices.

use lisp::Vm;

/// Plain runes: each is a unary `ctx → ctx` primitive.
const PLAIN: &[(char, &str)] = &[
    ('ᚠ', "fire"),  // FEHU
    ('ᛁ', "ice"),   // ISA
    ('ᚱ', "bolt"),  // RAIDO
    ('ᛒ', "self"),  // BERKANO — target self
];

/// Parametrized runes: each is `n → ctx → ctx`. Consumes the following number.
const PARAM: &[(char, &str)] = &[
    ('ᛊ', "area"),  // SOWILO
    ('ᛏ', "power"), // TIWAZ
];

#[derive(Debug)]
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
            out.push(Tok::Num(s.parse().unwrap()));
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

/// Pair each parametrized token with the following number, leaving plain
/// tokens as-is. Returns a list of fragments ready to splice into a sexpr:
/// each is either `"fire"` or `"(area 3)"`.
fn resolve(toks: Vec<Tok>) -> Result<Vec<String>, String> {
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

fn tape_to_sexpr(tape: &str) -> Result<String, String> {
    let toks = lex(tape)?;
    let parts = resolve(toks)?;
    Ok(format!("(thread (start) (list {}))", parts.join(" ")))
}

/// The spell prelude: everything that makes runes mean things.
/// Defines `thread`, `start`, the per-rune primitives, and the `assoc-set`
/// helper. Closes the letrec-bindings list but leaves letrec itself open —
/// `cast()` appends the spell body and a closing paren.
const PRELUDE_BINDINGS: &str = r#"
(letrec ((assoc-set (lambda (k v ctx) (cons (cons k v) ctx)))
         (thread    (lambda (ctx fs)
                      (if (null? fs) ctx
                          (thread ((car fs) ctx) (cdr fs)))))
         (start     (lambda () '()))
         (fire      (lambda (ctx) (assoc-set 'element 'fire ctx)))
         (ice       (lambda (ctx) (assoc-set 'element 'ice ctx)))
         (bolt      (lambda (ctx) (assoc-set 'shape   'bolt ctx)))
         (self      (lambda (ctx) (assoc-set 'target  'self ctx)))
         (area      (lambda (n)   (lambda (ctx) (assoc-set 'area  n ctx))))
         (power     (lambda (n)   (lambda (ctx) (assoc-set 'power n ctx)))))
"#;

fn cast(vm: &mut Vm, tape: &str) {
    println!("tape:   {tape}");
    let body = match tape_to_sexpr(tape) {
        Ok(s) => s,
        Err(e) => {
            println!("err:    compile: {e}\n");
            return;
        }
    };
    println!("sexpr:  {body}");
    let src = format!("{PRELUDE_BINDINGS}  {body})");
    match vm.eval_str(&src) {
        Ok(v) => println!("ctx:    {v}\n"),
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn main() {
    let mut vm = Vm::new();
    println!("letrs spell demo\n================\n");

    cast(&mut vm, "ᚠ");              // just fire
    cast(&mut vm, "ᚠ ᛊ 3 ᛁ");        // the canonical example: fire, area-3, ice
    cast(&mut vm, "ᚱ ᚠ ᛏ 5");        // bolt + fire + power-5
    cast(&mut vm, "ᛒ ᛁ ᛊ 2");        // self-targeted ice area-2

    // intentional failures, to show error surfaces
    cast(&mut vm, "ᚠ ᛊ");            // ᛊ expects a number
    cast(&mut vm, "x");              // unknown rune
}
