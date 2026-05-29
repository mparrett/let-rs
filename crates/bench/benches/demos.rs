//! DSL benchmarks for the spell + genes demos. These exercise the
//! *per-cast* cost specifically — Vm + prelude install happen once
//! per bench setup (via `iter_batched`), not per iteration. That way
//! a regression points at the cast hot path rather than installation.

use std::cell::RefCell;
use std::rc::Rc;

use codons::tape_to_sexpr as codon_tape_to_sexpr;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use lisp::Vm;
use runes::tape_to_sexpr as rune_tape_to_sexpr;
use world::World;

// ─── spells ────────────────────────────────────────────────────

fn bench_cast_spell(c: &mut Criterion) {
    // The canonical four-rune cast: fire, area-3, ice. Mirrors what
    // `examples/spells.rs` and the WASM Spell Lab run on every click.
    let list = rune_tape_to_sexpr("ᚠ ᛊ 3 ᛁ").unwrap();
    let body = format!(
        "(world-apply! (assoc-set 'tx 4 (assoc-set 'ty 2 (thread (start) {list}))))"
    );
    c.bench_function("cast_spell_canonical", |b| {
        b.iter_batched(
            || {
                let world = Rc::new(RefCell::new(World::new(8, 5).expect("8×5 fits")));
                let mut vm = Vm::new();
                spells::install_with_world(&mut vm, world);
                vm
            },
            |mut vm: Vm| black_box(vm.eval_str(black_box(&body)).unwrap()),
            BatchSize::SmallInput,
        )
    });
}

// ─── genes: express ────────────────────────────────────────────

fn bench_cast_genome_balanced(c: &mut Criterion) {
    // One allele per trait — every locus expresses solo. No mutation.
    let list = codon_tape_to_sexpr("AUG CGA GCA ACA UCA GCG AUC GAU UAA").unwrap();
    let body = format!("(express! (thread '() {list}))");
    let src = genes::seeded(0, &body);
    c.bench_function("cast_genome_balanced", |b| {
        b.iter_batched(
            || {
                let mut vm = Vm::new();
                genes::install(&mut vm);
                vm
            },
            |mut vm| black_box(vm.eval_str(black_box(&src)).unwrap()),
            BatchSize::SmallInput,
        )
    });
}

fn bench_cast_genome_with_mut(c: &mut Criterion) {
    // Same balanced strand with a MUT codon — the new rational rate
    // path through `mutate!`. Stresses xorshift + ratio comparison.
    let list = codon_tape_to_sexpr("AUG CGA GCA ACA UCA GCG AUC GAU MUT UAA").unwrap();
    let body = format!("(express! (thread '() {list}))");
    let src = genes::seeded(42, &body);
    c.bench_function("cast_genome_with_mut", |b| {
        b.iter_batched(
            || {
                let mut vm = Vm::new();
                genes::install(&mut vm);
                vm
            },
            |mut vm| black_box(vm.eval_str(black_box(&src)).unwrap()),
            BatchSize::SmallInput,
        )
    });
}

// ─── genes: breed ──────────────────────────────────────────────

fn bench_breed_diploid(c: &mut Criterion) {
    // Cross two fully-diploid parents (two alleles per locus). Forces
    // the Mendelian gamete pick on every trait.
    let mama = "AUG CGA CGU GCA GCU ACA ACU UCA UCU GCG GCC AUC AUA GAU GAC UAA";
    let papa = "AUG CGC CGG GCA GCU ACA ACU UCA UCU GCG GCC AUC AUA GAU GAC UAA";
    let la = codon_tape_to_sexpr(mama).unwrap();
    let lb = codon_tape_to_sexpr(papa).unwrap();
    let body = format!(
        "(express! (breed! seed (thread '() {la}) (thread '() {lb})))"
    );
    let src = genes::seeded(7, &body);
    c.bench_function("breed_diploid", |b| {
        b.iter_batched(
            || {
                let mut vm = Vm::new();
                genes::install(&mut vm);
                vm
            },
            |mut vm| black_box(vm.eval_str(black_box(&src)).unwrap()),
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    benches,
    bench_cast_spell,
    bench_cast_genome_balanced,
    bench_cast_genome_with_mut,
    bench_breed_diploid,
);
criterion_main!(benches);
