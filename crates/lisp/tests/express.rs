//! Integration tests for the genes resolver — locks `express!` behavior
//! (diploid averaging, Mendelian dominance, hash-deterministic tiebreaks)
//! against known codon strands so future refactors can't silently drift.

use codons::tape_to_sexpr;
use lisp::Vm;

/// Cast a strand and return the phenotype Val as its Display string. The
/// shape is an alist `((trait . value) ...)` printed in lisp form. Tests
/// assert on substring containment so they're robust to trait ordering.
fn express(tape: &str) -> String {
    express_seeded(tape, 0)
}

/// Same as [`express`] but with an explicit seed in scope so `MUT`
/// codons see deterministic randomness.
fn express_seeded(tape: &str, seed: i64) -> String {
    let mut vm = Vm::new();
    genes::install(&mut vm);
    let list = tape_to_sexpr(tape).expect("tape should lex");
    let body = format!("(express! (thread '() {list}))");
    let src = genes::seeded(seed, &body);
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
    let src = format!("(express! (thread '() {list}))");
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

// ─── mutation (MUT codon + mutate! prim) ────────────────────────

const BALANCED_MUT: &str = "AUG CGA GCA ACA UCA GCG AUC GAU MUT UAA";

#[test]
fn mutate_prim_rejects_rates_outside_zero_to_one() {
    // The numeric tower switched mutate! from integer percent (0..100)
    // to rational probability (0..1). Old-style integer percents like
    // 25 now error rather than silently mutate at the wrong rate.
    let mut vm = Vm::new();
    genes::install(&mut vm);
    for bad in ["25", "100", "-1", "2/1", "-1/4"] {
        let r = vm.eval_str(&format!("(mutate! {bad} 1 '())"));
        assert!(
            r.is_err(),
            "{bad} should be rejected as a probability: {r:?}"
        );
    }
    // 0 and 1 (the integer-shaped endpoints) and any ratio in [0,1]
    // are accepted.
    for ok in ["0", "1", "1/4", "1/100"] {
        let r = vm.eval_str(&format!("(mutate! {ok} 1 '())"));
        assert!(r.is_ok(), "{ok} should be accepted: {r:?}");
    }
}

#[test]
fn mutation_is_deterministic_for_same_seed() {
    // Same input → same output. Bedrock of ADR-012's "seeded, pure" choice.
    let a = express_seeded(BALANCED_MUT, 42);
    let b = express_seeded(BALANCED_MUT, 42);
    assert_eq!(a, b);
}

#[test]
fn mutation_differs_across_seeds() {
    // Different seeds → different mutations (with high probability).
    // We check several pairs to make the test robust against happening
    // to land on the same drift pattern by chance.
    let baseline = express_seeded(BALANCED_MUT, 42);
    let mut seen_diff = false;
    for seed in [43, 44, 45, 100, 999] {
        if express_seeded(BALANCED_MUT, seed) != baseline {
            seen_diff = true;
            break;
        }
    }
    assert!(
        seen_diff,
        "no seed in 43..=999 produced a different phenotype than seed 42"
    );
}

#[test]
fn numeric_mutation_stays_in_bounds() {
    // At 25% per-allele rate, ±10 drift, clamped to [0, 100]. Even after
    // many seeds, no numeric trait should ever leave the bounds.
    for seed in 0..20 {
        let p = express_seeded(BALANCED_MUT, seed);
        // Walk the phenotype string and extract each `(trait . N)` value.
        for trait_name in ["size", "strength", "speed", "armor"] {
            let needle = format!("({trait_name} . ");
            if let Some(pos) = p.find(&needle) {
                let tail = &p[pos + needle.len()..];
                let end = tail.find(')').unwrap();
                let val: i64 = tail[..end].trim().parse().unwrap();
                assert!(
                    (0..=100).contains(&val),
                    "seed={seed}: {trait_name} = {val} is out of [0,100]; phenotype = {p}",
                );
            }
        }
    }
}

#[test]
fn categorical_mutation_lands_in_pool() {
    // Categorical mutations must pick a value from the trait's option
    // pool — never anything else.
    let pools: &[(&str, &[&str])] = &[
        ("color", &["green", "red"]),
        ("ability", &["fire-breath", "sonic-roar"]),
        ("biome", &["volcanic", "ocean"]),
    ];
    for seed in 0..10 {
        let p = express_seeded(BALANCED_MUT, seed);
        for (trait_name, pool) in pools {
            let needle = format!("({trait_name} . ");
            if let Some(pos) = p.find(&needle) {
                let tail = &p[pos + needle.len()..];
                let end = tail.find(')').unwrap();
                let val = tail[..end].trim();
                assert!(
                    pool.contains(&val),
                    "seed={seed}: {trait_name} = {val:?} not in pool {pool:?}",
                );
            }
        }
    }
}

#[test]
fn mutation_without_mut_codon_is_a_no_op() {
    // No MUT in the tape → seed doesn't matter; same phenotype regardless.
    let baseline = "AUG CGA GCA ACA UCA GCG AUC GAU UAA";
    let s42 = express_seeded(baseline, 42);
    let s99 = express_seeded(baseline, 99);
    assert_eq!(s42, s99);
}

// ─── breeding (breed! prim) ─────────────────────────────────────

/// Cross two parent strands at a given seed and return the child's
/// phenotype as a Display string. Mirrors the example's `breeding`
/// helper but for tests — it asserts on phenotype substrings.
fn breed(tape_a: &str, tape_b: &str, seed: i64) -> String {
    let mut vm = Vm::new();
    genes::install(&mut vm);
    let la = tape_to_sexpr(tape_a).expect("parent A should lex");
    let lb = tape_to_sexpr(tape_b).expect("parent B should lex");
    let body = format!("(express! (breed! seed (thread '() {la}) (thread '() {lb})))");
    let src = genes::seeded(seed, &body);
    format!("{}", vm.eval_str(&src).expect("breed should evaluate"))
}

const DIPLOID_A: &str = "AUG CGA CGU GCA GCU ACA ACU UCA UCU GCG GCC AUC AUA GAU GAC UAA";
const DIPLOID_B: &str = "AUG CGC CGG GCA GCU ACA ACU UCA UCU GCG GCC AUC AUA GAU GAC UAA";

#[test]
fn breeding_is_deterministic_for_same_seed() {
    let a = breed(DIPLOID_A, DIPLOID_B, 7);
    let b = breed(DIPLOID_A, DIPLOID_B, 7);
    assert_eq!(a, b);
}

#[test]
fn breeding_differs_across_seeds() {
    // Diploid parents have a real choice per locus, so different seeds
    // produce different children with high probability. Probe several
    // pairs to stay robust against accidental matches.
    let baseline = breed(DIPLOID_A, DIPLOID_B, 7);
    let mut seen_diff = false;
    for seed in [8, 9, 10, 100, 999] {
        if breed(DIPLOID_A, DIPLOID_B, seed) != baseline {
            seen_diff = true;
            break;
        }
    }
    assert!(seen_diff);
}

#[test]
fn child_inherits_traits_from_either_parent() {
    // Parent A has size + speed; parent B has color + biome. Child
    // should have ALL four (trait union, not intersection).
    let a = "AUG CGA ACA UAA"; // size + speed only
    let b = "AUG GCG GAU UAA"; // color + biome only
    let child = breed(a, b, 1);
    for needle in ["size", "speed", "color", "biome"] {
        assert!(child.contains(needle), "missing {needle:?} in {child}");
    }
}

#[test]
fn trait_missing_from_both_parents_is_missing_from_child() {
    // Neither parent has strength/armor/ability. Neither should appear
    // in the child.
    let a = "AUG CGA ACA UAA"; // size + speed
    let b = "AUG GCG GAU UAA"; // color + biome
    let child = breed(a, b, 1);
    for absent in ["strength", "armor", "ability"] {
        assert!(
            !child.contains(absent),
            "did not expect {absent:?} in {child}"
        );
    }
}

#[test]
fn child_phenotype_can_differ_from_both_parents() {
    // Parent A only has size 70 dom; parent B only has size 30 rec.
    // The child has one allele from each: (70 dom, 30 rec) → avg 50.
    // Neither parent expresses size 50 in isolation.
    let a = "AUG CGA UAA"; // size 70 dom
    let b = "AUG CGU UAA"; // size 30 rec
    let child = breed(a, b, 1);
    assert!(
        child.contains("(size . 50)"),
        "expected averaged size, got {child}"
    );
}
