//! Genetics-demo host support: the genome prelude (lisp source), the
//! `express!` resolver primitive, and the ASCII creature-card renderer.
//!
//! Pulled out of `examples/genes.rs` once the WASM bridge became a second
//! consumer (ADR-011's "promote when a second consumer appears" clause).
//! Subsequently extracted from the `lisp` crate into its own sibling
//! crate alongside `spells` (ADR-016) — the engine carries no
//! demo-specific vocabulary.
//!
//! Depends only on `lisp` (`Vm`, `Val`, `Arity`). The CEK engine itself
//! is untouched.

use std::collections::HashSet;

use lisp::Vm;
use lisp::val::{Arity, Val};

/// Seed-independent half of the genome prelude — installed once via
/// `install(vm)`. `dom` / `rec` bind to `#t` / `#f` so the codon
/// fragments (e.g. `(size 70 dom)`) evaluate without quoting.
/// `genome-start` and `genome-stop` are ctx-identity anchors so the
/// codon table can lean on the biological start/stop convention
/// without special-casing them in `thread` — they're namespaced
/// (rather than bare `start`/`stop`) so that a host installing the
/// spells pack alongside genes can keep spells' zero-arg `start` for
/// its pipeline seed without collision. Each trait closure is
/// curried: `(size 70 dom)` returns a `ctx → ctx` that adds the
/// allele.
pub const PRELUDE_DEFINES: &str = r#"
(define dom          #t)
(define rec          #f)
(define assoc-set    (lambda (k v ctx) (cons (cons k v) ctx)))
(define thread       (lambda (ctx fs)
                       (if (null? fs) ctx
                           (thread ((car fs) ctx) (cdr fs)))))
(define genome-start (lambda (ctx) ctx))
(define genome-stop  (lambda (ctx) ctx))
(define add-allele (lambda (trait value kind ctx)
                     (assoc-set trait
                                (cons (cons value kind)
                                      (assoc-get trait ctx))
                                ctx)))
(define size       (lambda (n kind) (lambda (ctx) (add-allele 'size     n kind ctx))))
(define strength   (lambda (n kind) (lambda (ctx) (add-allele 'strength n kind ctx))))
(define speed      (lambda (n kind) (lambda (ctx) (add-allele 'speed    n kind ctx))))
(define armor      (lambda (n kind) (lambda (ctx) (add-allele 'armor    n kind ctx))))
(define color      (lambda (v kind) (lambda (ctx) (add-allele 'color    v kind ctx))))
(define ability    (lambda (v kind) (lambda (ctx) (add-allele 'ability  v kind ctx))))
(define biome      (lambda (v kind) (lambda (ctx) (add-allele 'biome    v kind ctx))))
"#;

/// Register the genes prims AND install the seed-independent prelude
/// defines. Idempotent: re-installing shadows prior bindings.
///
/// Seed-dependent mutate variants are *not* installed here — they're
/// re-created per cast inside `seeded`'s let chain so the closure can
/// capture the caller's seed via lexical scope (ADR-012).
pub fn install(vm: &mut Vm) {
    vm.register_prim("express!", Arity::Exact(1), express_prim);
    vm.register_prim("mutate!", Arity::Exact(3), mutate_prim);
    vm.register_prim("breed!", Arity::Exact(3), breed_prim);
    vm.eval_str(PRELUDE_DEFINES)
        .expect("genes prelude failed to install");
}

/// Wrap `body` in the per-cast let chain that exposes `seed` and the
/// four mutate variants (`mutate`, `mut01`, `mut10`, `mut50`) to the
/// body. Each variant is a `ctx → ctx` closure that captures `seed`
/// lexically and threads it into `mutate!`.
///
/// This split — install the bulk of the prelude once, re-bind the
/// seed-dependent closures per call — preserves ADR-012's lexical
/// seed pattern while still avoiding the per-call cost of parsing the
/// rest of the prelude.
///
/// Mutation rates expressed as exact rationals: `1/4` for `mutate`
/// (chosen for demo visibility — at our 7-trait scale ~25% gives a
/// ~84% chance of at least one visible drift per cast); `1/100`,
/// `1/10`, `1/2` for the explicit variants.
pub fn seeded(seed: i64, body: &str) -> String {
    format!(
        "(let ((seed {seed}))\n  \
           (let ((mutate (lambda (ctx) (mutate! 1/4   seed ctx)))\n        \
                 (mut01  (lambda (ctx) (mutate! 1/100 seed ctx)))\n        \
                 (mut10  (lambda (ctx) (mutate! 1/10  seed ctx)))\n        \
                 (mut50  (lambda (ctx) (mutate! 1/2   seed ctx))))\n    \
             {body}))"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Numeric,
    /// Categorical traits carry a static option pool — the values
    /// `mutate!` can pick from when a categorical allele is mutated.
    /// Today every pool has two values (one dom, one rec in the codon
    /// table); the resolver itself doesn't care about pool size.
    Categorical(&'static [&'static str]),
}

const TRAITS: &[(&str, Kind)] = &[
    ("size", Kind::Numeric),
    ("strength", Kind::Numeric),
    ("speed", Kind::Numeric),
    ("armor", Kind::Numeric),
    ("color", Kind::Categorical(&["green", "red"])),
    ("ability", Kind::Categorical(&["fire-breath", "sonic-roar"])),
    ("biome", Kind::Categorical(&["volcanic", "ocean"])),
];

fn trait_kind(name: &str) -> Option<Kind> {
    TRAITS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

/// FNV-1a 32-bit hash. Used for deterministic tiebreaks and creature
/// names. Tiny and dep-free.
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// Walk an alist `((k1 . v1) (k2 . v2) …)` and collect first-occurrence
/// `(key, value)` pairs. Later duplicates are shadowed (matches the lisp
/// `assoc-get` behavior in the prelude).
fn collect_first_pairs(alist: &Val) -> Vec<(String, Val)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    let mut cur = alist;
    while let Val::Cons(head, tail) = cur {
        if let Val::Cons(k, v) = head.as_ref()
            && let Val::Sym(s) = k.as_ref()
        {
            let key = s.to_string();
            if seen.insert(key.clone()) {
                out.push((key, (**v).clone()));
            }
        }
        cur = tail;
    }
    out
}

/// Walk a list of cons-pairs `((a1 . b1) (a2 . b2) …)` and return them.
/// Stops at the first non-cons cell.
fn unpack_pairs(list: &Val) -> Vec<(Val, Val)> {
    let mut out = Vec::new();
    let mut cur = list;
    while let Val::Cons(head, tail) = cur {
        if let Val::Cons(a, b) = head.as_ref() {
            out.push(((**a).clone(), (**b).clone()));
        }
        cur = tail;
    }
    out
}

/// `(express! genome)` — read the genome ctx, resolve each trait to a
/// single phenotype value, return an alist of `(trait . value)` pairs.
///
/// Numeric traits average all alleles present (single allele expresses
/// alone). Categorical traits follow Mendelian dominance: dominant wins
/// over recessive; in dom/dom or rec/rec ties the genome hash picks a
/// deterministic winner.
fn express_prim(args: &[Val]) -> Result<Val, String> {
    let genome = &args[0];
    let genome_hash = fnv1a(&format!("{genome}"));

    let mut phenotype: Vec<(Val, Val)> = Vec::new();
    for (key, value) in collect_first_pairs(genome) {
        let kind = match trait_kind(&key) {
            Some(k) => k,
            None => continue, // unknown traits silently passed over
        };
        let alleles = unpack_pairs(&value);
        if alleles.is_empty() {
            continue;
        }
        let resolved = match kind {
            Kind::Numeric => resolve_numeric(&alleles)?,
            Kind::Categorical(_) => resolve_categorical(&alleles, genome_hash),
        };
        phenotype.push((Val::Sym(key.into()), resolved));
    }
    Ok(Val::alist_from(&phenotype))
}

/// `(mutate! rate seed ctx)` — walk the genome and roll a per-allele
/// coin at probability `rate` to decide which alleles drift. `rate`
/// is a number in `[0, 1]` — typically a rational like `1/4` (25%).
/// Same `(rate, seed, ctx)` → same output. dom/rec is preserved
/// across mutation; only the value changes.
///
/// - Numeric alleles drift by ±10 clamped to [0, 100].
/// - Categorical alleles swap to a *different* value from the trait's
///   option pool (so a mutation event is always visible — never a
///   no-op pick of the same allele).
///
/// Implementation uses xorshift32 seeded from `seed`. The coin uses
/// the full 32-bit RNG output against the rate's denominator (widened
/// to u128 to avoid overflow). Iteration order is the cons-list
/// order, so it's deterministic across runs.
fn mutate_prim(args: &[Val]) -> Result<Val, String> {
    let (rnum, rden) = rate_as_probability(&args[0])?;
    let seed = match &args[1] {
        Val::Num(n) => *n as u32,
        other => return Err(format!("mutate!: seed must be an int, got {other}")),
    };
    // A 0 seed traps xorshift32 in the all-zero state. Bump it once so the
    // caller can still pass 0 as a "default" without losing all randomness.
    let mut rng = if seed == 0 { 0x9E37_79B9 } else { seed };

    let mut out_traits: Vec<(String, Vec<(Val, Val)>)> = Vec::new();
    for (key, value) in collect_first_pairs(&args[2]) {
        let kind = match trait_kind(&key) {
            Some(k) => k,
            None => continue,
        };
        let mut alleles = unpack_pairs(&value);
        for (val_slot, _dom) in alleles.iter_mut() {
            // Mutate iff (r / 2^32) < (rnum / rden), i.e. r*rden < rnum*2^32.
            let r = xorshift32(&mut rng) as u128;
            if r * rden >= rnum * (1u128 << 32) {
                continue;
            }
            *val_slot = match (kind, &*val_slot) {
                (Kind::Numeric, Val::Num(n)) => {
                    let delta = if xorshift32(&mut rng) & 1 == 0 {
                        10
                    } else {
                        -10
                    };
                    Val::Num((*n + delta).clamp(0, 100))
                }
                (Kind::Categorical(pool), Val::Sym(s)) => {
                    let cur = s.to_string();
                    // pick a *different* pool value; for 2-option pools
                    // this is the other one, for larger pools it's a
                    // random pick excluding the current value.
                    let others: Vec<&&str> = pool.iter().filter(|opt| **opt != cur).collect();
                    if others.is_empty() {
                        continue; // pool only contains current value; no-op
                    }
                    let pick = others[(xorshift32(&mut rng) as usize) % others.len()];
                    Val::Sym((*pick).into())
                }
                _ => continue, // allele shape doesn't match its kind; skip
            };
        }
        out_traits.push((key, alleles));
    }
    Ok(traits_to_genome_ctx(out_traits))
}

/// `(breed! seed parent-A parent-B)` — Mendelian segregation across
/// two genomes. For each trait present in either parent, the child
/// receives one random allele from each parent that has the trait.
/// A parent with zero alleles for a trait contributes nothing (so the
/// child is haploid for that trait); a trait missing from both is
/// missing from the child. Pure: same `(seed, A, B)` → same child.
///
/// This is the simplest meiosis model that matches kaiju's "child
/// inherits one allele per locus from each parent, randomly chosen
/// when the parent is diploid." Mutation is intentionally NOT
/// applied here — caller chains `(mutate (breed! …))` if they want
/// drift on top.
fn breed_prim(args: &[Val]) -> Result<Val, String> {
    let seed = match &args[0] {
        Val::Num(n) => *n as u32,
        other => return Err(format!("breed!: seed must be an int, got {other}")),
    };
    let mut rng = if seed == 0 { 0x9E37_79B9 } else { seed };

    let pa = collect_first_pairs(&args[1]);
    let pb = collect_first_pairs(&args[2]);

    // Trait union — every trait in either parent. Preserve parent-A's
    // ordering for stability, then append parent-B's traits that A
    // didn't have.
    let mut keys: Vec<String> = pa.iter().map(|(k, _)| k.clone()).collect();
    for (k, _) in &pb {
        if !keys.iter().any(|x| x == k) {
            keys.push(k.clone());
        }
    }

    let lookup = |list: &[(String, Val)], key: &str| -> Option<Val> {
        list.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };

    let mut out_traits: Vec<(String, Vec<(Val, Val)>)> = Vec::new();
    for key in keys {
        let from_a = lookup(&pa, &key)
            .map(|v| unpack_pairs(&v))
            .unwrap_or_default();
        let from_b = lookup(&pb, &key)
            .map(|v| unpack_pairs(&v))
            .unwrap_or_default();
        let mut child_alleles: Vec<(Val, Val)> = Vec::new();
        if !from_a.is_empty() {
            let pick = (xorshift32(&mut rng) as usize) % from_a.len();
            child_alleles.push(from_a[pick].clone());
        }
        if !from_b.is_empty() {
            let pick = (xorshift32(&mut rng) as usize) % from_b.len();
            child_alleles.push(from_b[pick].clone());
        }
        if !child_alleles.is_empty() {
            out_traits.push((key, child_alleles));
        }
    }
    Ok(traits_to_genome_ctx(out_traits))
}

/// Extract a probability rate as `(numerator, denominator)` u128s.
/// Accepts `Val::Num` (only 0 or 1 — anything else is out of [0,1])
/// or `Val::Ratio` (any value in [0,1]). Validates `0 ≤ num/den ≤ 1`.
fn rate_as_probability(v: &Val) -> Result<(u128, u128), String> {
    let (n, d): (i128, i128) = match v {
        Val::Num(n) => (*n as i128, 1),
        Val::Ratio(n, d) => (*n as i128, *d as i128),
        other => return Err(format!("mutate!: rate must be a number, got {other}")),
    };
    if n < 0 || n > d {
        return Err(format!(
            "mutate!: rate must be in [0, 1], got {}",
            display_ratio(n, d)
        ));
    }
    Ok((n as u128, d as u128))
}

fn display_ratio(n: i128, d: i128) -> String {
    if d == 1 {
        n.to_string()
    } else {
        format!("{n}/{d}")
    }
}

/// xorshift32 PRNG. Tiny, dep-free, deterministic — perfect for a demo.
/// Bad for cryptography. We're not doing cryptography.
fn xorshift32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// Build a genome-shaped ctx `((trait . ((v1 . d1) (v2 . d2))) …)`
/// from a Vec of (trait, alleles). The inverse of
/// `collect_first_pairs` + `unpack_pairs`.
fn traits_to_genome_ctx(traits: Vec<(String, Vec<(Val, Val)>)>) -> Val {
    let pairs: Vec<(Val, Val)> = traits
        .into_iter()
        .map(|(key, alleles)| (Val::Sym(key.into()), Val::alist_from(&alleles)))
        .collect();
    Val::alist_from(&pairs)
}

fn resolve_numeric(alleles: &[(Val, Val)]) -> Result<Val, String> {
    let mut sum: i64 = 0;
    let mut count: i64 = 0;
    for (v, _kind) in alleles.iter().take(2) {
        match v {
            Val::Num(n) => {
                sum += n;
                count += 1;
            }
            other => {
                return Err(format!(
                    "express!: numeric allele value must be int, got {other}"
                ));
            }
        }
    }
    Ok(Val::Num(sum / count.max(1)))
}

fn resolve_categorical(alleles: &[(Val, Val)], genome_hash: u32) -> Val {
    let pair = &alleles[..alleles.len().min(2)];
    if pair.len() == 1 {
        return pair[0].0.clone();
    }
    let a_dom = matches!(pair[0].1, Val::Bool(true));
    let b_dom = matches!(pair[1].1, Val::Bool(true));
    match (a_dom, b_dom) {
        (true, false) => pair[0].0.clone(),
        (false, true) => pair[1].0.clone(),
        _ => {
            // tie: deterministic by genome hash parity
            if genome_hash & 1 == 0 {
                pair[0].0.clone()
            } else {
                pair[1].0.clone()
            }
        }
    }
}

/// Render a phenotype alist as a small ASCII creature card. Name is a
/// 4-hex slug derived from the phenotype string so the same genome
/// always reads as the same creature.
pub fn render_creature(phenotype: &Val) -> String {
    let pheno_str = format!("{phenotype}");
    let name = format!("{:04x}", fnv1a(&pheno_str) & 0xffff);

    let pairs = collect_first_pairs(phenotype);
    let g = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());

    let size = g("size").and_then(num);
    let strength = g("strength").and_then(num);
    let speed = g("speed").and_then(num);
    let armor = g("armor").and_then(num);
    let color = g("color").and_then(sym).unwrap_or("—".into());
    let ability = g("ability").and_then(sym).unwrap_or("—".into());
    let biome = g("biome").and_then(sym).unwrap_or("—".into());

    let cell = |n: Option<i64>| n.map(|x| x.to_string()).unwrap_or("—".into());
    // portrait bucket uses size if present, falls back to mid
    let portrait_size = size.unwrap_or(50);
    let spikes = armor.map(|a| (a / 20).clamp(0, 5)).unwrap_or(0) as usize;

    let portrait = portrait_for(portrait_size, &color, spikes);
    // Card width: 48 cells, sized to fit the widest interior line
    // (the stats row) plus a left+right `│` border. Every interior
    // line is padded to the same width so the right border lines up
    // and the top/bottom rules match.
    //
    // Caveat: padding uses Rust's char-count formatting (`{:<N}`),
    // which assumes one unicode codepoint = one display cell. That
    // holds for the ASCII glyphs and box-drawing chars we use, and
    // for the portrait's CJK-ish unicode (`◉`, `▽`, etc.) in every
    // mono font we render against today (terminals + the web demo).
    // A double-width font would push the right border on portrait
    // rows over by a cell — we'd need a wcwidth-aware padding helper.
    let mut out = String::new();
    out.push_str(&format!(
        "╭─ creature #{name} ─────────────────────────────╮\n"
    ));
    out.push_str(&format!(
        "│ size {:>3}  strength {:>3}  speed {:>3}  armor {:>3} │\n",
        cell(size),
        cell(strength),
        cell(speed),
        cell(armor)
    ));
    out.push_str(&format!("│ color {color:<8}  ability {ability:<20} │\n"));
    out.push_str(&format!("│ biome {biome:<38} │\n"));
    for line in portrait {
        out.push_str(&format!("│   {line:<42} │\n"));
    }
    out.push_str("╰──────────────────────────────────────────────╯");
    out
}

fn num(v: Val) -> Option<i64> {
    if let Val::Num(n) = v { Some(n) } else { None }
}

fn sym(v: Val) -> Option<String> {
    if let Val::Sym(s) = v {
        Some(s.to_string())
    } else {
        None
    }
}

/// A tiny portrait table: 3 lines, varying width with size (small/mid/big)
/// and crown-row with spike count. Color is a verbal cue in the card so
/// the portrait stays color-blind-safe.
fn portrait_for(size: i64, _color: &str, spikes: usize) -> [String; 3] {
    let bucket = if size < 34 {
        0
    } else if size < 67 {
        1
    } else {
        2
    };
    let crown = if spikes == 0 {
        String::from("     ")
    } else {
        "▲".repeat(spikes)
    };
    let body = match bucket {
        0 => "(o o)",
        1 => "(◉ ◉)",
        _ => "(◎ ◎)",
    };
    let base = match bucket {
        0 => "  ▽▽ ",
        1 => " ▼▽▼ ",
        _ => "▼▽▽▽▼",
    };
    [crown, body.into(), base.into()]
}
