use std::cell::RefCell;
use std::rc::Rc;

use curves::{Turtle, install, render};
use lisp::Vm;
use strokes::tape_to_sexpr;

fn fresh_vm() -> (Vm, Rc<RefCell<Turtle>>) {
    let turtle = Rc::new(RefCell::new(Turtle::new()));
    let mut vm = Vm::new();
    install(&mut vm, turtle.clone());
    (vm, turtle)
}

/// Convenience: parse a tape, eval `(draw! …)`, return the resulting
/// canvas string from the host-side renderer (bypasses the symbol-wrap
/// in `render!`).
fn cast_axiom(tape: &str) -> String {
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr(tape).unwrap();
    vm.eval_str(&format!("(draw! {list})")).unwrap();
    render(&turtle.borrow()).unwrap()
}

#[test]
fn forward_stamps_one_cell() {
    let canvas = cast_axiom("F");
    // One cell stamped means the bbox is a single character — the
    // heading-glyph for East (`─`).
    assert_eq!(canvas, "─");
}

#[test]
fn turn_left_then_forward_draws_diagonal_glyph() {
    // F+F: forward east, turn left to NE, forward NE. Two cells,
    // diagonal trail.
    let canvas = cast_axiom("F+F");
    let lines: Vec<&str> = canvas.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|l| l.contains('╱')));
    assert!(lines.iter().any(|l| l.contains('─')));
}

#[test]
fn closed_octagon_returns_to_start() {
    // 8 forward strokes with a +45° turn between each → closes back
    // to origin. The trail is 8 cells but the bbox is a 4×4 square
    // (corners trimmed because the path doesn't go through them).
    let canvas = cast_axiom("F+F+F+F+F+F+F+F");
    let lines: Vec<&str> = canvas.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "octagon should occupy 4 rows, got:\n{canvas}"
    );
    // every row should be exactly the bbox width (4 cells)
    for line in &lines {
        assert_eq!(line.chars().count(), 4, "row width mismatch in:\n{canvas}");
    }
}

#[test]
fn push_pop_restores_position() {
    // F[+F]F: forward, save, turn-forward, restore, forward. The
    // second F (after pop) goes east from the saved position, so we
    // expect cells at (1,0), (2,-1) from the branch, and (2,0) from
    // the post-pop forward. Three cells total.
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr("F[+F]F").unwrap();
    vm.eval_str(&format!("(draw! {list})")).unwrap();
    assert_eq!(turtle.borrow().cell_count(), 3);
}

#[test]
fn nonterminals_pass_silently() {
    // X and Y are L-system non-terminals — `draw!` should treat them
    // as identity (no turtle action) rather than erroring. Lets users
    // run Sierpiński / Hilbert / dragon L-systems whose rules carry
    // auxiliary symbols through to the final tape.
    let (mut vm, turtle) = fresh_vm();
    vm.eval_str("(draw! '(X F Y F X))").unwrap();
    // 2 F's stamped, X/Y silently skipped.
    assert_eq!(turtle.borrow().cell_count(), 2);
}

#[test]
fn truly_unknown_symbol_still_errors() {
    // Multi-char and punctuation symbols still surface as errors —
    // the nonterminal pass-through is scoped to single ASCII letters.
    let (mut vm, _t) = fresh_vm();
    let r = vm.eval_str("(draw! '(F foo F))");
    assert!(
        matches!(&r, Err(e) if e.contains("unknown stroke")),
        "got {r:?}"
    );
}

#[test]
fn unmatched_pop_errors() {
    let (mut vm, _t) = fresh_vm();
    let list = tape_to_sexpr("F]").unwrap();
    let r = vm.eval_str(&format!("(draw! {list})"));
    assert!(
        matches!(&r, Err(e) if e.contains("pop on empty")),
        "got {r:?}"
    );
}

#[test]
fn grow_zero_returns_axiom() {
    // grow with n=0 is the identity. The whole pipeline (axiom-only
    // cast through `grow`) should produce the same canvas as draw!
    // on the axiom.
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr("F+F").unwrap();
    vm.eval_str(&format!("(draw! (grow {list} '() 0))"))
        .unwrap();
    let via_grow = render(&turtle.borrow()).unwrap();

    let direct = cast_axiom("F+F");
    assert_eq!(via_grow, direct);
}

#[test]
fn grow_one_iteration_replaces_matching_symbols() {
    // F → F+F at 1 iteration. Resulting tape is F+F (3 symbols
    // from 1). Cell count = 2 forwards = 2.
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr("F").unwrap();
    vm.eval_str(&format!("(draw! (grow {list} '((F F + F)) 1))"))
        .unwrap();
    assert_eq!(turtle.borrow().cell_count(), 2);
}

#[test]
fn grow_skips_non_matching_symbols() {
    // Rule only mentions F; +/- pass through unchanged. So after one
    // grow on F+F with rule F→FF we get FF+FF (4 forwards), confirming
    // the identity-rewrite fallback in `expand-one`.
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr("F+F").unwrap();
    vm.eval_str(&format!("(draw! (grow {list} '((F F F)) 1))"))
        .unwrap();
    assert_eq!(turtle.borrow().cell_count(), 4);
}

#[test]
fn reset_clears_state() {
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr("F+F+F").unwrap();
    vm.eval_str(&format!("(draw! {list})")).unwrap();
    assert!(turtle.borrow().cell_count() > 0);
    vm.eval_str("(reset!)").unwrap();
    assert_eq!(turtle.borrow().cell_count(), 0);
}

#[test]
fn render_prim_returns_canvas_string() {
    // The lisp-side `render!` should produce a value whose Display is
    // the same as the Rust-side `render()`.
    let (mut vm, turtle) = fresh_vm();
    let list = tape_to_sexpr("F+F").unwrap();
    vm.eval_str(&format!("(draw! {list})")).unwrap();
    let host_render = render(&turtle.borrow()).unwrap();
    let lisp_render = vm.eval_str("(render!)").unwrap();
    assert_eq!(format!("{lisp_render}"), host_render);
}

/// Build a turtle that has walked `n` cells diagonally (heading NE),
/// which stamps `n` sparse cells spanning an `n`×`n` bounding box.
/// Driving the turtle directly rather than through `(grow …)` keeps
/// these bound checks fast — the lisp rewrite is what's slow, and it
/// isn't what's under test here.
fn diagonal_turtle(n: usize) -> Turtle {
    let mut t = Turtle::new();
    t.turn(1); // heading NE
    for _ in 0..n {
        t.forward(true);
    }
    t
}

#[test]
fn oversized_bbox_errors_instead_of_allocating() {
    // A diagonal walk spans an N×N bbox from N sparse cells, so the
    // dense render grid grows quadratically while the turtle's own
    // memory grows linearly. Tapes well inside the wasm host's 10M step
    // budget can reach bboxes measured in gigabytes, so the renderer
    // must refuse rather than attempt the allocation.
    let t = diagonal_turtle(1500);
    assert_eq!(t.cell_count(), 1500);
    let err = render(&t).expect_err("1500² is over the cap");
    assert!(err.contains("exceeds"), "unexpected message: {err}");
}

#[test]
fn large_but_representable_canvas_still_renders() {
    // Just under the cap — the bound must not clip legitimate curves.
    let t = diagonal_turtle(1400);
    let canvas = render(&t).expect("1400² is under the cap");
    assert_eq!(canvas.lines().count(), 1400);
}

#[test]
fn render_prim_surfaces_the_cap_as_an_eval_error() {
    // The lisp-side prim must return an Err (not panic, not truncate)
    // so a wasm host sees a catchable exception.
    let turtle = Rc::new(RefCell::new(diagonal_turtle(1500)));
    let mut vm = Vm::new();
    install(&mut vm, turtle);
    let err = vm.eval_str("(render!)").expect_err("render! should error");
    assert!(err.contains("exceeds"), "unexpected message: {err}");
}
