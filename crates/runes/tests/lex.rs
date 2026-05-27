use runes::tape_to_sexpr;

#[test]
fn empty_tape_yields_empty_list() {
    assert_eq!(tape_to_sexpr("").unwrap(), "(list )");
    assert_eq!(tape_to_sexpr("   ").unwrap(), "(list )");
}

#[test]
fn single_plain_rune() {
    assert_eq!(tape_to_sexpr("ᚠ").unwrap(), "(list fire)");
    assert_eq!(tape_to_sexpr("ᛁ").unwrap(), "(list ice)");
}

#[test]
fn canonical_example() {
    assert_eq!(
        tape_to_sexpr("ᚠ ᛊ 3 ᛁ").unwrap(),
        "(list fire (area 3) ice)"
    );
}

#[test]
fn multi_digit_numeral() {
    assert_eq!(tape_to_sexpr("ᛏ 42").unwrap(), "(list (power 42))");
}

#[test]
fn parametrized_without_number_errors() {
    let r = tape_to_sexpr("ᚠ ᛊ");
    assert!(matches!(r, Err(e) if e.contains("area") && e.contains("number")));
}

#[test]
fn unknown_rune_errors() {
    let r = tape_to_sexpr("ᚠ x");
    assert!(matches!(r, Err(e) if e.contains("unknown rune") && e.contains('x')));
}

#[test]
fn stray_number_errors() {
    let r = tape_to_sexpr("3 ᚠ");
    assert!(matches!(r, Err(e) if e.contains("stray number")));
}

#[test]
fn oversized_number_errors_not_panics() {
    // Pre-fix the unwrap() in lex panicked. A 20-digit string is past
    // i64::MAX; we want a clean Err, not a thread panic.
    let r = tape_to_sexpr("ᛊ 99999999999999999999");
    assert!(matches!(r, Err(e) if e.contains("out of i64 range")));
}

#[test]
fn whitespace_between_runes_is_optional() {
    // No whitespace between runes: each char still lexes individually.
    assert_eq!(tape_to_sexpr("ᚠᛁᚱ").unwrap(), "(list fire ice bolt)");
}
