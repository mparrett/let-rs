//! End-to-end genes DSL demo: codon tape → sexpr → CEK eval → phenotype.
//!
//! The vocabulary, the resolver, and the renderer all live in
//! `lisp::genes` so the WASM bridge can share them (ADR-011). This
//! example just drives a handful of sequences against a fresh VM.

use codons::tape_to_sexpr;
use lisp::{Vm, genes};

fn sequence(vm: &mut Vm, label: &str, seed: i64, tape: &str) {
    println!("── {label}  (seed={seed}) ──");
    println!("tape:   {tape}");
    let list = match tape_to_sexpr(tape) {
        Ok(s) => s,
        Err(e) => {
            println!("err:    compile: {e}\n");
            return;
        }
    };
    // Wrap the prelude in (let ((seed N)) …) so the MUT codon's
    // `(mutate ctx)` closure can read the caller's seed via lexical
    // scope. See ADR-012.
    let body = format!("(express! (thread '() {list}))");
    let src = format!(
        "(let ((seed {seed})) {}  {body}))",
        genes::PRELUDE_BINDINGS,
    );
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
    sequence(&mut vm, "balanced", 0,
        "AUG CGA GCA ACA UCA GCG AUC GAU UAA");

    // two size alleles (70 dom + 30 rec) — phenotype averages to 50
    sequence(&mut vm, "size-averaged", 0,
        "AUG CGA CGU GCA UAA");

    // partial genome — only color stated, the rest sit out
    sequence(&mut vm, "fragmentary", 0,
        "AUG GCG UAA");

    // both alleles dominant for color — hash tiebreak chooses one,
    // deterministically
    sequence(&mut vm, "color-conflict-dom", 0,
        "AUG GCG GCG UAA");

    // both recessive — same tiebreak path
    sequence(&mut vm, "color-conflict-rec", 0,
        "AUG GCC GCC UAA");

    // mutation: same parent genome, two different seeds → two different
    // offspring. Same seed twice → identical offspring.
    sequence(&mut vm, "balanced + MUT", 42,
        "AUG CGA GCA ACA UCA GCG AUC GAU MUT UAA");
    sequence(&mut vm, "balanced + MUT", 99,
        "AUG CGA GCA ACA UCA GCG AUC GAU MUT UAA");

    // error surface — unknown codon
    sequence(&mut vm, "bad-codon", 0, "AUG XYZ UAA");
}
