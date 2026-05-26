//! End-to-end genes DSL demo: codon tape → sexpr → CEK eval → phenotype.
//!
//! The vocabulary, the resolver, and the renderer all live in
//! `lisp::genes` so the WASM bridge can share them (ADR-011). This
//! example just drives a handful of sequences against a fresh VM.

use codons::tape_to_sexpr;
use lisp::{Vm, genes};

fn sequence(vm: &mut Vm, label: &str, tape: &str) {
    println!("── {label} ──");
    println!("tape:   {tape}");
    let list = match tape_to_sexpr(tape) {
        Ok(s) => s,
        Err(e) => {
            println!("err:    compile: {e}\n");
            return;
        }
    };
    let body = format!("(express! (thread '() {list}))");
    let src = format!("{}  {body})", genes::PRELUDE_BINDINGS);
    match vm.eval_str(&src) {
        Ok(phenotype) => println!("{}\n", genes::render_creature(&phenotype)),
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn main() {
    let mut vm = Vm::new();
    genes::install(&mut vm);

    println!("letrs genes demo\n================\n");

    // one allele per trait — every locus expresses solo
    sequence(&mut vm, "balanced",
        "AUG CGA GCA ACA UCA GCG AUC GAU UAA");

    // two size alleles (70 dom + 30 rec) — phenotype averages to 50
    sequence(&mut vm, "size-dominant",
        "AUG CGA CGU GCA UAA");

    // partial genome — only color stated, the rest sit out
    sequence(&mut vm, "fragmentary",
        "AUG GCG UAA");

    // both alleles dominant for color — hash tiebreak chooses one,
    // deterministically
    sequence(&mut vm, "color-conflict-dom",
        "AUG GCG GCG UAA");

    // both recessive — same tiebreak path
    sequence(&mut vm, "color-conflict-rec",
        "AUG GCC GCC UAA");

    // error surface — unknown codon
    sequence(&mut vm, "bad-codon", "AUG XYZ UAA");
}
