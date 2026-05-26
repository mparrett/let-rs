//! Integration tests for the genes resolver — locks `express!` behavior
//! (diploid averaging, Mendelian dominance, hash-deterministic tiebreaks)
//! against known codon strands so future refactors can't silently drift.

use codons::tape_to_sexpr;
use lisp::{Vm, genes};

/// Cast a strand and return the phenotype Val as its Display string. The
/// shape is an alist `((trait . value) ...)` printed in lisp form. Tests
/// assert on substring containment so they're robust to trait ordering.
fn express(tape: &str) -> String {
    let mut vm = Vm::new();
    genes::install(&mut vm);
    let list = tape_to_sexpr(tape).expect("tape should lex");
    let body = format!("(express! (thread '() {list}))");
    let src = format!("{}  {body})", genes::PRELUDE_BINDINGS);
    format!("{}", vm.eval_str(&src).expect("express should evaluate"))
}

#[test]
fn balanced_creature_expresses_all_seven_traits() {
    // One allele per trait — every locus expresses solo.
    let p = express("AUG CGA GCA ACA UCA GCG AUC GAU UAA");
    for needle in [
        "(size . 70)",
        "(strength . 75)",
        "(speed . 80)",
        "(armor . 60)",
        "(color . green)",
        "(ability . fire-breath)",
        "(biome . volcanic)",
    ] {
        assert!(p.contains(needle), "missing {needle:?} in {p}");
    }
}

#[test]
fn diploid_numeric_averages_alleles() {
    // CGA = size 70 dom, CGU = size 30 rec → averaged to 50.
    let p = express("AUG CGA CGU UAA");
    assert!(p.contains("(size . 50)"), "expected averaged size, got {p}");
}

#[test]
fn fragmentary_genome_only_expresses_stated_traits() {
    // Only color stated; numerics + other categoricals should be absent.
    let p = express("AUG GCG UAA");
    assert!(p.contains("(color . green)"));
    for absent in ["size", "strength", "speed", "armor", "ability", "biome"] {
        assert!(!p.contains(absent), "did not expect {absent:?} in {p}");
    }
}

#[test]
fn dominant_wins_over_recessive_regardless_of_order() {
    // GCG = green dom, GCC = red rec. Either ordering → green wins.
    let dom_first = express("AUG GCG GCC UAA");
    let rec_first = express("AUG GCC GCG UAA");
    assert!(dom_first.contains("(color . green)"), "got {dom_first}");
    assert!(rec_first.contains("(color . green)"), "got {rec_first}");
}

#[test]
fn dom_dom_tiebreak_is_deterministic() {
    // Two GCG (both green dom) — same strand twice should produce the
    // same color across runs. Tests that the FNV-hash tiebreak is stable.
    let a = express("AUG GCG GCG UAA");
    let b = express("AUG GCG GCG UAA");
    assert_eq!(a, b);
    assert!(a.contains("(color . green)"), "got {a}");
}

#[test]
fn rec_rec_tiebreak_is_deterministic() {
    // Two GCC (both red rec) — single-allele set; both alleles' values
    // are red, so the picked one is red regardless of tiebreak parity.
    let a = express("AUG GCC GCC UAA");
    let b = express("AUG GCC GCC UAA");
    assert_eq!(a, b);
    assert!(a.contains("(color . red)"), "got {a}");
}

#[test]
fn render_creature_produces_stable_name_for_same_genome() {
    // Two casts of the balanced strand should render the same name slug.
    let mut vm = Vm::new();
    genes::install(&mut vm);
    let list = tape_to_sexpr("AUG CGA GCA ACA UCA GCG AUC GAU UAA").unwrap();
    let body = format!("(express! (thread '() {list}))");
    let src = format!("{}  {body})", genes::PRELUDE_BINDINGS);
    let p1 = vm.eval_str(&src).unwrap();
    let p2 = vm.eval_str(&src).unwrap();
    let card1 = genes::render_creature(&p1);
    let card2 = genes::render_creature(&p2);
    assert_eq!(card1, card2);
    // Sanity: the name should be a 4-hex slug.
    assert!(card1.contains("creature #"), "card was: {card1}");
}

#[test]
fn unknown_codon_error_threads_through_tape_lex() {
    // The error surface — codons crate rejects, never reaches express!.
    let r = tape_to_sexpr("AUG XYZ UAA");
    assert!(matches!(r, Err(e) if e.contains("unknown codon") && e.contains("XYZ")));
}
