use runes::tape_to_sexpr;

#[test]
fn empty_tape_yields_empty_list() {
    assert_eq!(tape_to_sexpr("").unwrap(), "(list )");
    assert_eq!(tape_to_sexpr("   ").unwrap(), "(list )");
}

#[test]
fn single_plain_rune() {
    assert_eq!(tape_to_sexpr("ᚦ").unwrap(), "(list fire)");
    assert_eq!(tape_to_sexpr("ᛇ").unwrap(), "(list ice)");
}

#[test]
fn canonical_example() {
    assert_eq!(
        tape_to_sexpr("ᚦ ᛞ 3 ᛇ").unwrap(),
        "(list fire (area 3) ice)"
    );
}

#[test]
fn multi_digit_numeral() {
    assert_eq!(tape_to_sexpr("ᛟ 42").unwrap(), "(list (power 42))");
}

#[test]
fn parametrized_without_number_errors() {
    let r = tape_to_sexpr("ᚦ ᛞ");
    assert!(matches!(r, Err(e) if e.contains("area") && e.contains("number")));
}

#[test]
fn unknown_rune_errors() {
    let r = tape_to_sexpr("ᚦ x");
    assert!(matches!(r, Err(e) if e.contains("unknown rune") && e.contains('x')));
}

#[test]
fn stray_number_errors() {
    let r = tape_to_sexpr("3 ᚦ");
    assert!(matches!(r, Err(e) if e.contains("stray number")));
}

#[test]
fn oversized_number_errors_not_panics() {
    // Pre-fix the unwrap() in lex panicked. A 20-digit string is past
    // i64::MAX; we want a clean Err, not a thread panic.
    let r = tape_to_sexpr("ᛞ 99999999999999999999");
    assert!(matches!(r, Err(e) if e.contains("out of i64 range")));
}

#[test]
fn whitespace_between_runes_is_optional() {
    // No whitespace between runes: each char still lexes individually.
    assert_eq!(tape_to_sexpr("ᚦᛇᛚ").unwrap(), "(list fire ice bolt)");
}
