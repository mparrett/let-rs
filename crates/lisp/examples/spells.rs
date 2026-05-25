//! End-to-end spell DSL demo: rune tape → sexpr → CEK eval → final ctx.
//!
//! The rune translation lives in `crates/runes/`; this example only owns the
//! lisp prelude and the per-cast wrapper. The prelude is plain user-level
//! lisp — closures over `assoc-set` — so the engine learned nothing new for
//! this demo.

use lisp::Vm;
use runes::tape_to_sexpr;

/// The spell prelude: everything that makes runes mean things. Closes the
/// letrec bindings list but leaves letrec itself open — `cast()` appends the
/// spell body and a closing paren.
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
    let list = match tape_to_sexpr(tape) {
        Ok(s) => s,
        Err(e) => {
            println!("err:    compile: {e}\n");
            return;
        }
    };
    let body = format!("(thread (start) {list})");
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
