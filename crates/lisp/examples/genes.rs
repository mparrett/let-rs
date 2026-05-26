//! End-to-end genes DSL demo: codon tape → sexpr → CEK eval → phenotype.
//!
//! Mirrors the spells example structurally. The codon translation lives in
//! `crates/codons/`; this example only owns the lisp prelude, the
//! `express!` resolver (a pure host primitive), and the per-cast wrapper.
//!
//! Diploid by accumulation: stating two codons for the same trait stacks
//! two alleles in a per-trait list inside the genome ctx. The resolver
//! averages numerics and runs Mendelian dominance on categoricals; ties
//! are broken deterministically from a hash of the ctx so repeat runs
//! produce the same creature. See ADR-011.

use codons::tape_to_sexpr;
use lisp::Vm;
use lisp::val::{Arity, Val};

/// The genome prelude. `dom` / `rec` bind to `#t` / `#f` so the codon
/// fragments (e.g. `(size 70 dom)`) evaluate without quoting. `start` and
/// `stop` are ctx-identity anchors so the codon table can lean on the
/// biological start/stop convention without special-casing them in
/// `thread`. Each trait closure is curried: `(size 70 dom)` returns a
/// `ctx → ctx` that adds the allele.
const PRELUDE_BINDINGS: &str = r#"
(letrec ((dom        #t)
         (rec        #f)
         (assoc-set  (lambda (k v ctx) (cons (cons k v) ctx)))
         (assoc-get  (lambda (k ctx)
                       (if (null? ctx) '()
                           (if (eq? (car (car ctx)) k) (cdr (car ctx))
                               (assoc-get k (cdr ctx))))))
         (thread     (lambda (ctx fs)
                       (if (null? fs) ctx
                           (thread ((car fs) ctx) (cdr fs)))))
         (start      (lambda (ctx) ctx))
         (stop       (lambda (ctx) ctx))
         (add-allele (lambda (trait value kind ctx)
                       (assoc-set trait
                                  (cons (cons value kind)
                                        (assoc-get trait ctx))
                                  ctx)))
         (size       (lambda (n kind) (lambda (ctx) (add-allele 'size     n kind ctx))))
         (strength   (lambda (n kind) (lambda (ctx) (add-allele 'strength n kind ctx))))
         (speed      (lambda (n kind) (lambda (ctx) (add-allele 'speed    n kind ctx))))
         (armor      (lambda (n kind) (lambda (ctx) (add-allele 'armor    n kind ctx))))
         (color      (lambda (v kind) (lambda (ctx) (add-allele 'color    v kind ctx))))
         (ability    (lambda (v kind) (lambda (ctx) (add-allele 'ability  v kind ctx))))
         (biome      (lambda (v kind) (lambda (ctx) (add-allele 'biome    v kind ctx)))))
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Numeric,
    Categorical,
}

const TRAITS: &[(&str, Kind)] = &[
    ("size",     Kind::Numeric),
    ("strength", Kind::Numeric),
    ("speed",    Kind::Numeric),
    ("armor",    Kind::Numeric),
    ("color",    Kind::Categorical),
    ("ability",  Kind::Categorical),
    ("biome",    Kind::Categorical),
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
    use std::collections::HashSet;
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
            Kind::Categorical => resolve_categorical(&alleles, genome_hash),
        };
        phenotype.push((Val::Sym(key.into()), resolved));
    }
    Ok(to_pair_list(phenotype))
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
            other => return Err(format!("express!: numeric allele value must be int, got {other}")),
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
            if genome_hash & 1 == 0 { pair[0].0.clone() } else { pair[1].0.clone() }
        }
    }
}

fn to_pair_list(items: Vec<(Val, Val)>) -> Val {
    use std::rc::Rc;
    let mut acc = Val::Nil;
    for (k, v) in items.into_iter().rev() {
        let pair = Val::Cons(Rc::new(k), Rc::new(v));
        acc = Val::Cons(Rc::new(pair), Rc::new(acc));
    }
    acc
}

/// Render a phenotype alist as a small ASCII creature card. Name is a
/// 4-hex slug derived from the phenotype string so the same genome
/// always reads as the same creature.
fn render_creature(phenotype: &Val) -> String {
    let pheno_str = format!("{phenotype}");
    let name = format!("{:04x}", fnv1a(&pheno_str) & 0xffff);

    let pairs = collect_first_pairs(phenotype);
    let g = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());

    let size     = g("size").and_then(num);
    let strength = g("strength").and_then(num);
    let speed    = g("speed").and_then(num);
    let armor    = g("armor").and_then(num);
    let color    = g("color").and_then(sym).unwrap_or("—".into());
    let ability  = g("ability").and_then(sym).unwrap_or("—".into());
    let biome    = g("biome").and_then(sym).unwrap_or("—".into());

    let cell = |n: Option<i64>| n.map(|x| x.to_string()).unwrap_or("—".into());
    // portrait bucket uses size if present, falls back to mid
    let portrait_size = size.unwrap_or(50);
    let spikes = armor.map(|a| (a / 20).max(0).min(5)).unwrap_or(0) as usize;

    let portrait = portrait_for(portrait_size, &color, spikes);
    let mut out = String::new();
    out.push_str(&format!("╭─ creature #{name} ─────────────╮\n"));
    out.push_str(&format!("│ size {:>3}  strength {:>3}  speed {:>3}  armor {:>3}\n",
        cell(size), cell(strength), cell(speed), cell(armor)));
    out.push_str(&format!("│ color {color:<8}  ability {ability}\n"));
    out.push_str(&format!("│ biome {biome}\n"));
    for line in portrait {
        out.push_str(&format!("│   {line}\n"));
    }
    out.push_str("╰────────────────────────────────╯");
    out
}

fn num(v: Val) -> Option<i64> {
    if let Val::Num(n) = v { Some(n) } else { None }
}

fn sym(v: Val) -> Option<String> {
    if let Val::Sym(s) = v { Some(s.to_string()) } else { None }
}

/// A tiny portrait table: 3 lines, varying width with size (small/mid/big)
/// and crown-row with spike count. Color is a verbal cue in the card so
/// the portrait stays color-blind-safe.
fn portrait_for(size: i64, _color: &str, spikes: usize) -> [String; 3] {
    let bucket = if size < 34 { 0 } else if size < 67 { 1 } else { 2 };
    let crown = if spikes == 0 { String::from("     ") } else { "▲".repeat(spikes) };
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
    let src = format!("{PRELUDE_BINDINGS}  {body})");
    match vm.eval_str(&src) {
        Ok(phenotype) => {
            println!("{}\n", render_creature(&phenotype));
        }
        Err(e) => println!("err:    eval: {e}\n"),
    }
}

fn main() {
    let mut vm = Vm::new();
    vm.register_prim("express!", Arity::Exact(1), express_prim);

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
