//! End-to-end curves DSL demo: stroke tape → sexpr → CEK eval → ASCII canvas.
//!
//! The glyph translation lives in `crates/strokes/`; the turtle, prims,
//! prelude, and renderer all live in `crates/curves/` (ADR-019). This
//! example just drives a handful of sequences against a fresh VM.
//!
//! Note on `+` / `-`: the engine binds these to arithmetic prims, so the
//! stroke tape goes through `strokes` which emits them as *quoted*
//! symbols (`'+`, `'-`) — the `draw!` prim then matches symbol names,
//! not function values.

use std::cell::RefCell;
use std::rc::Rc;

use curves::Turtle;
use lisp::Vm;
use strokes::tape_to_sexpr;

fn cast(vm: &mut Vm, label: &str, axiom: &str, rules: &str, iters: i64) {
    println!("── {label}  (iterations={iters}) ──");
    println!("axiom:  {axiom}");
    if !rules.is_empty() {
        println!("rules:  {rules}");
    }
    let axiom_list = match tape_to_sexpr(axiom) {
        Ok(s) => s,
        Err(e) => {
            println!("err:    compile axiom: {e}\n");
            return;
        }
    };
    // Wrap the axiom in `(grow … rules iters)`. With `iters=0` `grow`
    // returns the axiom unchanged, so this branch handles both
    // axiom-only casts and iterated rewrites without a special case.
    let body = format!(
        "(let ((_ (reset!))) \
           (let ((_ (draw! (grow {axiom_list} '({rules}) {iters})))) \
             (render!)))"
    );
    match vm.eval_str(&body) {
        Ok(v) => println!("{v}\n"),
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn main() {
    let turtle = Rc::new(RefCell::new(Turtle::new()));
    let mut vm = Vm::new();
    curves::install(&mut vm, turtle);

    println!("let-rs curves demo\n=================\n");

    // octagon: 8 unit edges, 45° turns. No recursion — just the axiom.
    cast(&mut vm, "octagon", "F+F+F+F+F+F+F+F", "", 0);

    // Lévy C curve at 45°: F → +F--F+ produces the canonical curlicue.
    // 3 iterations is enough to recognize the curve in ASCII.
    cast(&mut vm, "lévy C curve", "F", "(F + F - - F +)", 3);

    // Fractal-plant shape: F → F[+F]F[-F]F. Branches splay at 45°;
    // 2 iterations gives a recognizable Y-tree, 3 starts to fill in.
    cast(
        &mut vm,
        "fractal plant",
        "F",
        "(F F [ + F ] F [ - F ] F)",
        2,
    );

    // Error surfaces: unknown glyph in tape, unmatched ']'
    cast(&mut vm, "bad-glyph", "Fx", "", 0);
    cast(&mut vm, "unmatched-pop", "F]F", "", 0);
}
