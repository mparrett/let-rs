use codons::tape_to_sexpr;

#[test]
fn empty_tape_yields_empty_list() {
    assert_eq!(tape_to_sexpr("").unwrap(), "(list )");
    assert_eq!(tape_to_sexpr("   ").unwrap(), "(list )");
}

#[test]
fn single_control_codon() {
    // AUG/UAA/UGA emit namespaced anchors (`genome-start`/`genome-stop`)
    // so that hosts installing the spell pack alongside genes don't
    // collide on the bare `start` name (zero-arg in spells, one-arg
    // ctx-identity here).
    assert_eq!(tape_to_sexpr("AUG").unwrap(), "(list genome-start)");
    assert_eq!(tape_to_sexpr("UAA").unwrap(), "(list genome-stop)");
}

#[test]
fn canonical_strand() {
    assert_eq!(
        tape_to_sexpr("AUG CGA CGU UAA").unwrap(),
        "(list genome-start (size 70 dom) (size 30 rec) genome-stop)"
    );
}

#[test]
fn whitespace_between_codons_is_required() {
    // "AUGCGA" is one six-char token, not two triplets — codons need
    // whitespace separation (the visual genetic-strand convention).
    let r = tape_to_sexpr("AUGCGA");
    assert!(matches!(r, Err(e) if e.contains("3 characters") && e.contains('6')));
}

#[test]
fn unknown_codon_errors() {
    let r = tape_to_sexpr("AUG XYZ UAA");
    assert!(matches!(r, Err(e) if e.contains("unknown codon") && e.contains("XYZ")));
}

#[test]
fn full_genome_balanced_creature() {
    // Mirrors the "balanced" sequence in examples/genes.rs.
    let s = tape_to_sexpr("AUG CGA GCA ACA UCA GCG AUC GAU UAA").unwrap();
    assert!(s.starts_with("(list genome-start "));
    assert!(s.contains("(size 70 dom)"));
    assert!(s.contains("(strength 75 dom)"));
    assert!(s.contains("(biome 'volcanic dom)"));
    assert!(s.ends_with(" genome-stop)"));
}
