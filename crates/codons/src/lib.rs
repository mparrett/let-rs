//! Codon-tape translation: RNA-style ASCII triplets → lisp list.
//!
//! A DSL layer on top of the [`lisp`](../lisp/index.html) crate, parallel
//! to [`runes`](../runes/index.html) but with genetics vocabulary. Each
//! codon is a three-character triplet (drawn from A/U/C/G) that maps to a
//! complete sexpr fragment — usually an allele declaration like
//! `(size 70 dom)`. The output is a `(list …)` expression that callers
//! splice into a genome pipeline, e.g.
//! `(express! (thread (start '()) <list>))`.
//!
//! Unlike runes, codons don't take a following numeral — the allele value
//! is baked into the table entry. The lexer is correspondingly simpler:
//! split on whitespace, validate each token is a known triplet.
//!
//! See ADR-011 for why this lives in its own crate rather than in
//! `runes/` (sibling DSLs each get their own table, per ADR-010).

/// Codon table: each entry pairs a 3-character RNA triplet with a complete
/// sexpr fragment. Control codons (`AUG`, `UAA`, `UGA`) emit no-op anchors;
/// allele codons emit `(trait value dom|rec)` triples.
pub const CODONS: &[(&str, &str)] = &[
    // control
    ("AUG", "start"),
    ("UAA", "stop"),
    ("UGA", "stop"),
    // size
    ("CGA", "(size 70 dom)"),
    ("CGU", "(size 30 rec)"),
    ("CGC", "(size 90 dom)"),
    ("CGG", "(size 10 rec)"),
    // strength
    ("GCA", "(strength 75 dom)"),
    ("GCU", "(strength 25 rec)"),
    // speed
    ("ACA", "(speed 80 dom)"),
    ("ACU", "(speed 20 rec)"),
    // armor
    ("UCA", "(armor 60 dom)"),
    ("UCU", "(armor 15 rec)"),
    // color (symbol payloads are quoted so they don't resolve as vars)
    ("GCG", "(color 'green dom)"),
    ("GCC", "(color 'red rec)"),
    // ability
    ("AUC", "(ability 'fire-breath dom)"),
    ("AUA", "(ability 'sonic-roar rec)"),
    // biome
    ("GAU", "(biome 'volcanic dom)"),
    ("GAC", "(biome 'ocean rec)"),
    // mutation: 5% per-allele drift using the lexically-scoped `seed`
    // the driver wraps around the prelude. See ADR-012.
    ("MUT", "mutate"),
];

fn lookup(token: &str) -> Option<&'static str> {
    CODONS.iter().find(|(k, _)| *k == token).map(|(_, v)| *v)
}

/// Translate a codon tape into a `(list …)` expression suitable for
/// splicing into a genome pipeline. Whitespace separates codons; any
/// 3-character ASCII triplet matching the [`CODONS`] table is accepted.
/// Returns `Err` for unknown codons or tokens that aren't exactly three
/// characters long. An empty (or whitespace-only) tape yields `(list )`.
pub fn tape_to_sexpr(tape: &str) -> Result<String, String> {
    let mut parts: Vec<&'static str> = Vec::new();
    for token in tape.split_whitespace() {
        if token.chars().count() != 3 {
            return Err(format!(
                "codons must be 3 characters: got '{token}' ({} chars)",
                token.chars().count()
            ));
        }
        match lookup(token) {
            Some(frag) => parts.push(frag),
            None => return Err(format!("unknown codon: '{token}'")),
        }
    }
    Ok(format!("(list {})", parts.join(" ")))
}
