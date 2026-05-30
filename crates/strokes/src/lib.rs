//! Stroke-tape translation: turtle glyphs → lisp list of quoted symbols.
//!
//! A DSL layer on top of the [`lisp`](../lisp/index.html) crate, parallel
//! to [`runes`](../runes/index.html) and [`codons`](../codons/index.html)
//! but for L-system / turtle-graphics work. Each glyph maps to a quoted
//! symbol (e.g. `F` → `'F`, `+` → `'+`) so the resulting list can be
//! rewritten in pure lisp before being dispatched through a side-effecting
//! `draw!` prim. See ADR-019.
//!
//! The six glyphs are the classic 2D turtle alphabet — forward (with/
//! without drawing), turn left/right by one tick, and push/pop turtle
//! state for branching. Angle-per-tick is the consumer's choice (the
//! `curves` pack uses 45°); the alphabet itself is angle-agnostic.

/// Stroke table: each glyph maps to a quoted-symbol fragment for splicing
/// into a `(list …)`. Quoting matters because two of the glyph names
/// (`+`, `-`) shadow arithmetic prims if left bare — the consumer wants
/// the *symbol*, not the function value.
pub const STROKES: &[(char, &str)] = &[
    ('F', "'F"),
    ('G', "'G"),
    ('+', "'+"),
    ('-', "'-"),
    ('[', "'["),
    (']', "']"),
];

fn lookup(c: char) -> Option<&'static str> {
    STROKES.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

/// Translate a stroke tape into a `(list …)` expression suitable for
/// splicing into a curves pipeline. Whitespace between glyphs is
/// optional and ignored. An empty (or whitespace-only) tape yields
/// `(list )`.
///
/// Any ASCII letter not in [`STROKES`] is passed through as a quoted
/// symbol (e.g. `X` → `'X`) — this is the L-system "non-terminal"
/// convention, where auxiliary symbols drive the rewrite rules but
/// don't correspond to a turtle action. The matching skip-on-draw
/// logic lives in `curves::draw_prim`. Non-letter unknowns (digits,
/// punctuation, non-ASCII) still error so typos surface.
pub fn tape_to_sexpr(tape: &str) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for c in tape.chars() {
        if c.is_whitespace() {
            continue;
        }
        if let Some(frag) = lookup(c) {
            parts.push(frag.into());
        } else if c.is_ascii_alphabetic() {
            parts.push(format!("'{c}"));
        } else {
            return Err(format!("unknown stroke: '{c}'"));
        }
    }
    Ok(format!("(list {})", parts.join(" ")))
}
