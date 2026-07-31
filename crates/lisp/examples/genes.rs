//! End-to-end genes DSL demo: codon tape → sexpr → CEK eval → phenotype.
//!
//! The vocabulary, the resolver, and the renderer all live in
//! `crates/genes/` so the WASM bridge can share them (ADR-011,
//! ADR-016). This example just drives a handful of sequences against a
//! fresh VM.

use std::rc::Rc;

use codons::tape_to_sexpr;
use lisp::Namespace;
use lisp::Vm;

fn sequence(vm: &mut Vm, ns: &Rc<Namespace>, label: &str, seed: i64, tape: &str) {
    println!("── {label}  (seed={seed}) ──");
    println!("tape:   {tape}");
    let list = match tape_to_sexpr(tape) {
        Ok(s) => s,
        Err(e) => {
            println!("err:    compile: {e}\n");
            return;
        }
    };
    // `genes::seeded` wraps the body in a let chain so the MUT codon's
    // `(mutate ctx)` closure captures `seed` via lexical scope. See
    // ADR-012.
    let body = format!("(express! (thread '() {list}))");
    let src = genes::seeded(seed, &body);
    match vm.eval_str_in(ns, &src) {
        Ok(phenotype) => println!("{}\n", genes::render_creature(&phenotype)),
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn breeding(vm: &mut Vm, ns: &Rc<Namespace>, label: &str, seed: i64, tape_a: &str, tape_b: &str) {
    println!("── {label}  (seed={seed}) ──");
    println!("mama:   {tape_a}");
    println!("papa:   {tape_b}");
    let (la, lb) = match (tape_to_sexpr(tape_a), tape_to_sexpr(tape_b)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            println!("err:    compile: {e}\n");
            return;
        }
    };
    let body = format!("(express! (breed! seed (thread '() {la}) (thread '() {lb})))");
    let src = genes::seeded(seed, &body);
    match vm.eval_str_in(ns, &src) {
        Ok(phenotype) => println!("{}\n", genes::render_creature(&phenotype)),
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn main() {
    let mut vm = Vm::new();
    // Genome source uses `thread`, private to the pack (ADR-042).
    let ns = genes::install(&mut vm);

    println!("let-rs genes demo\n================\n");

    // one allele per trait — every locus expresses solo
    sequence(
        &mut vm,
        &ns,
        "balanced",
        0,
        "AUG CGA GCA ACA UCA GCG AUC GAU UAA",
    );

    // two size alleles (70 dom + 30 rec) — phenotype averages to 50
    sequence(&mut vm, &ns, "size-averaged", 0, "AUG CGA CGU GCA UAA");

    // partial genome — only color stated, the rest sit out
    sequence(&mut vm, &ns, "fragmentary", 0, "AUG GCG UAA");

    // both alleles dominant for color — hash tiebreak chooses one,
    // deterministically
    sequence(&mut vm, &ns, "color-conflict-dom", 0, "AUG GCG GCG UAA");

    // both recessive — same tiebreak path
    sequence(&mut vm, &ns, "color-conflict-rec", 0, "AUG GCC GCC UAA");

    // mutation: same parent genome, two different seeds → two different
    // offspring. Same seed twice → identical offspring.
    sequence(
        &mut vm,
        &ns,
        "balanced + MUT",
        42,
        "AUG CGA GCA ACA UCA GCG AUC GAU MUT UAA",
    );
    sequence(
        &mut vm,
        &ns,
        "balanced + MUT",
        99,
        "AUG CGA GCA ACA UCA GCG AUC GAU MUT UAA",
    );

    // breeding: two diploid parents. Each parent has TWO alleles per
    // trait, so the seed actually controls which allele is inherited
    // by the child (gamete model). With haploid parents (one allele
    // per trait), every seed produces the same child — the seed only
    // matters when there's a choice to make.
    let mama = "AUG CGA CGU GCA GCU ACA ACU UCA UCU GCG GCC AUC AUA GAU GAC UAA";
    let papa = "AUG CGC CGG GCA GCU ACA ACU UCA UCU GCG GCC AUC AUA GAU GAC UAA";
    breeding(&mut vm, &ns, "mama × papa", 7, mama, papa);
    breeding(&mut vm, &ns, "mama × papa", 9, mama, papa);

    // error surface — unknown codon
    sequence(&mut vm, &ns, "bad-codon", 0, "AUG XYZ UAA");
}
