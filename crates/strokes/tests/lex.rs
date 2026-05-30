use strokes::tape_to_sexpr;

#[test]
fn empty_tape_yields_empty_list() {
    assert_eq!(tape_to_sexpr("").unwrap(), "(list )");
    assert_eq!(tape_to_sexpr("   ").unwrap(), "(list )");
}

#[test]
fn single_glyph() {
    assert_eq!(tape_to_sexpr("F").unwrap(), "(list 'F)");
    assert_eq!(tape_to_sexpr("[").unwrap(), "(list '[)");
}

#[test]
fn canonical_example() {
    assert_eq!(tape_to_sexpr("F+F-F").unwrap(), "(list 'F '+ 'F '- 'F)");
}

#[test]
fn whitespace_is_optional() {
    assert_eq!(
        tape_to_sexpr("F + F - F").unwrap(),
        tape_to_sexpr("F+F-F").unwrap()
    );
}

#[test]
fn branching_glyphs() {
    assert_eq!(
        tape_to_sexpr("F[+F]F[-F]F").unwrap(),
        "(list 'F '[ '+ 'F '] 'F '[ '- 'F '] 'F)"
    );
}

#[test]
fn nodraw_glyph() {
    assert_eq!(tape_to_sexpr("FGF").unwrap(), "(list 'F 'G 'F)");
}

#[test]
fn ascii_letters_pass_through_as_quoted_symbols() {
    // L-system non-terminals (X, Y, A, B, …) emit as quoted symbols so
    // the rewrite rules can match them and the `draw!` skip-list can
    // ignore them at render time. Lowercase letters are accepted too
    // for L-systems that use them as a separate non-terminal alphabet.
    assert_eq!(tape_to_sexpr("FXF").unwrap(), "(list 'F 'X 'F)");
    assert_eq!(tape_to_sexpr("XY").unwrap(), "(list 'X 'Y)");
    assert_eq!(tape_to_sexpr("Fa").unwrap(), "(list 'F 'a)");
}

#[test]
fn unknown_non_letter_glyph_errors() {
    // Digits, punctuation, non-ASCII: still surface as a typo error.
    let r = tape_to_sexpr("F$");
    assert!(matches!(r, Err(e) if e.contains("unknown stroke") && e.contains('$')));
    let r = tape_to_sexpr("F!");
    assert!(
        matches!(&r, Err(e) if e.contains("unknown stroke") && e.contains('!')),
        "got {r:?}"
    );
}
