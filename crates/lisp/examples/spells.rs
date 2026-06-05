//! End-to-end spell DSL demo: rune tape → sexpr → CEK eval → final ctx.
//!
//! The rune translation lives in `crates/runes/`; the spell prelude lives
//! in `crates/spells/` (shared with the WASM bridge — ADR-010, ADR-016).
//! This example only owns the per-cast wrapper.
//!
//! As of ADR-025 the spell prelude uses `defspell`/`defparam` macros, so
//! the host is a `macros::MacroVm` rather than a raw `lisp::Vm`.

use macros::MacroVm;
use runes::tape_to_sexpr;

fn cast(vm: &mut MacroVm, tape: &str) {
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
    let mut vm = MacroVm::new();
    spells::install(&mut vm);
    println!("let-rs spell demo\n================\n");

    cast(&mut vm, "ᚦ"); // just fire
    cast(&mut vm, "ᚦ ᛞ 3 ᛇ"); // the canonical example: fire, area-3, ice
    cast(&mut vm, "ᛚ ᚦ ᛟ 5"); // bolt + fire + power-5
    cast(&mut vm, "ᛗ ᛇ ᛞ 2"); // self-targeted ice area-2

    // intentional failures, to show error surfaces
    cast(&mut vm, "ᚦ ᛞ"); // ᛞ expects a number
    cast(&mut vm, "x"); // unknown rune
}
