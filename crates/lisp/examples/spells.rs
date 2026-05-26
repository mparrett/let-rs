//! End-to-end spell DSL demo: rune tape → sexpr → CEK eval → final ctx.
//!
//! The rune translation lives in `crates/runes/`; the spell prelude lives
//! in `lisp::spells` (shared with the WASM bridge — ADR-010). This
//! example only owns the per-cast wrapper.

use lisp::{Vm, spells};
use runes::tape_to_sexpr;

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
    match vm.eval_str(&body) {
        Ok(v) => println!("ctx:    {v}\n"),
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn main() {
    let mut vm = Vm::new();
    spells::install(&mut vm);
    println!("letrs spell demo\n================\n");

    cast(&mut vm, "ᚠ");              // just fire
    cast(&mut vm, "ᚠ ᛊ 3 ᛁ");        // the canonical example: fire, area-3, ice
    cast(&mut vm, "ᚱ ᚠ ᛏ 5");        // bolt + fire + power-5
    cast(&mut vm, "ᛒ ᛁ ᛊ 2");        // self-targeted ice area-2

    // intentional failures, to show error surfaces
    cast(&mut vm, "ᚠ ᛊ");            // ᛊ expects a number
    cast(&mut vm, "x");              // unknown rune
}
