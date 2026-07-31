//! In-language error handling (ADR-041): `raise`, `error`, `guard`.

use lisp::step::Machine;
use lisp::{Progress, Vm, parse};

fn eval(src: &str) -> String {
    let mut vm = Vm::new();
    format!("{}", vm.eval_str(src).expect("eval"))
}

#[test]
fn a_guard_catches_and_returns_the_handler_value() {
    assert_eq!(eval("(guard (e 'caught) (car '()))"), "caught");
    // A body that doesn't raise ignores the handler entirely.
    assert_eq!(eval("(guard (e 'caught) (+ 1 2))"), "3");
}

#[test]
fn prim_failures_are_catchable() {
    // The point of routing prim errors through the raise mode: a prim
    // reports failure as a `String` and knows nothing about conditions,
    // yet its complaint is catchable.
    assert_eq!(
        eval("(guard (e (error-message e)) (car '()))"),
        "\"car: expected pair, got ()\""
    );
    assert_eq!(eval("(guard (e 'div) (/ 1 0))"), "div");
    assert_eq!(
        eval("(guard (e 'overflow) (* 9223372036854775807 9223372036854775807))"),
        "overflow"
    );
}

#[test]
fn machine_failures_are_catchable_too() {
    // Unbound names, arity, and non-callable heads all enter the same
    // mode, so there's one answer to "what can a guard catch" rather
    // than a list of exceptions.
    assert_eq!(eval("(guard (e 'unbound) (no-such-name))"), "unbound");
    assert_eq!(eval("(guard (e 'arity) ((lambda (a b) a) 1))"), "arity");
    assert_eq!(eval("(guard (e 'not-callable) (5 1))"), "not-callable");
    assert_eq!(eval("(guard (e 'set) (set! never-bound 1))"), "set");
}

#[test]
fn error_builds_the_conventional_condition_shape() {
    assert_eq!(
        eval("(guard (e e) (error \"no mana\" 5 10))"),
        "(error \"no mana\" 5 10)"
    );
    assert_eq!(eval("(guard (e (error? e)) (error \"x\"))"), "#t");
    assert_eq!(
        eval("(guard (e (error-message e)) (error \"no mana\" 5))"),
        "\"no mana\""
    );
    assert_eq!(
        eval("(guard (e (error-irritants e)) (error \"no mana\" 5 10))"),
        "(5 10)"
    );
    // Irritants keep their identity as values rather than being
    // flattened into the message — which is why `error` is a special
    // form and not a prim (a prim reports failure as a String).
    assert_eq!(
        eval("(guard (e (car (error-irritants e))) (error \"m\" (+ 2 3)))"),
        "5"
    );
}

#[test]
fn raise_carries_any_value_verbatim() {
    assert_eq!(eval("(guard (e e) (raise 'plain))"), "plain");
    assert_eq!(eval("(guard (e e) (raise 42))"), "42");
    assert_eq!(eval("(guard (e e) (raise '(1 2 3)))"), "(1 2 3)");
    // A raised non-condition is not an `error?`.
    assert_eq!(eval("(guard (e (error? e)) (raise 42))"), "#f");
}

#[test]
fn guards_nest_and_re_raising_reaches_the_outer_one() {
    assert_eq!(
        eval("(guard (o (list 'outer o)) (guard (i (raise 'passed-along)) (car '())))"),
        "(outer passed-along)"
    );
    // An inner guard that handles it stops the search.
    assert_eq!(
        eval("(guard (o 'outer) (guard (i 'inner) (car '())))"),
        "inner"
    );
    // A raise from inside a *handler* escapes to the next guard out.
    assert_eq!(
        eval("(guard (o (list 'outer o)) (guard (i (raise 'from-handler)) (car '())))"),
        "(outer from-handler)"
    );
}

#[test]
fn a_guard_only_covers_its_own_body() {
    // The handler runs after unwinding, in the guard's environment — so
    // a failure in the handler is not caught by the same guard, which
    // would otherwise loop.
    let mut vm = Vm::new();
    let err = vm
        .eval_str("(guard (e (car '())) (car '()))")
        .expect_err("handler failure escapes");
    assert!(err.msg.contains("car: expected pair"), "{err}");
}

#[test]
fn an_uncaught_condition_reports_as_it_always_did() {
    // `(error "msg")` reaching the top reads as just `msg`, so an
    // uncaught prim failure is worded exactly as it was before
    // conditions existed, with ADR-039's position intact.
    let mut vm = Vm::new();
    let err = vm.eval_str("(+ 1\n   (car '()))").expect_err("uncaught");
    assert_eq!(err.msg, "car: expected pair, got ()");
    assert_eq!(err.span.map(|s| (s.line, s.col)), Some((2, 4)));

    // Irritants come along.
    let err = vm
        .eval_str("(error \"no mana\" 5 10)")
        .expect_err("uncaught");
    assert_eq!(err.msg, "no mana (5 10)");

    // A raised non-condition says so rather than pretending to be one.
    let err = vm.eval_str("(raise 'boom)").expect_err("uncaught");
    assert_eq!(err.msg, "raised: boom");
}

#[test]
fn unwinding_crosses_deep_recursion() {
    // 500 frames of pending work discarded, one per step. The `(+ 1 …)`
    // is load-bearing: without it the call is in tail position, the
    // machine keeps the chain flat (ADR-040's depth test), and there is
    // nothing to unwind.
    let src = "(define f (lambda (n) (if (= n 0) (error \"bottom\") (+ 1 (f (- n 1))))))
               (guard (e (error-message e)) (f 500))";
    assert_eq!(eval(src), "\"bottom\"");
}

#[test]
fn unwinding_reclaims_the_frames_it_discards() {
    // Each discarded continuation drops the `Env` it held, so the store
    // slots those bindings owned come back through `Frame::drop`
    // (ADR-033) with no special handling in the unwinder.
    let mut vm = Vm::new();
    let store = vm.store_probe();
    let live = || store.len().expect("store outlives this");

    // Non-tail recursion on purpose: each pending `(+ 1 …)` holds a
    // frame whose env owns a store slot, which is what this measures.
    vm.eval_str("(define deep (lambda (n) (if (= n 0) (error \"x\") (+ 1 (deep (- n 1))))))")
        .unwrap();
    let before = live();
    for _ in 0..5 {
        assert_eq!(
            format!("{}", vm.eval_str("(guard (e 'caught) (deep 200))").unwrap()),
            "caught"
        );
    }
    assert_eq!(
        live(),
        before,
        "repeated catches should not grow the store's live slots"
    );
}

#[test]
fn the_step_budget_is_not_catchable() {
    // The one deliberate hole in "a guard catches everything", and the
    // reason it's deliberate: the budget is ADR-040's safety net against
    // code that never terminates. If a guard could swallow it, wrapping
    // a runaway loop in one would make it unkillable.
    let mut vm = Vm::new();
    vm.set_step_budget(2_000);
    let err = vm
        .eval_str("(guard (e 'caught) (letrec ((spin (lambda () (spin)))) (spin)))")
        .expect_err("the budget must escape the guard");
    assert_eq!(err.msg, "execution exceeded step budget");
}

#[test]
fn unwinding_is_interruptible() {
    // Unwinding advances one frame per step rather than looping to the
    // handler, so a host slicing evaluation keeps control during a deep
    // unwind instead of stalling for the length of the chain.
    let mut vm = Vm::new();
    vm.eval_str("(define deep (lambda (n) (if (= n 0) (error \"x\") (+ 1 (deep (- n 1))))))")
        .unwrap();
    let expr = parse::parse("(guard (e 'caught) (deep 300))").unwrap();
    let mut m = Machine::new(expr, vm.env().clone());

    let mut pauses = 0;
    let mut max_depth = 0;
    let v = loop {
        max_depth = max_depth.max(m.depth());
        match m.run(1).expect("eval") {
            Progress::Done(v) => break format!("{v}"),
            Progress::Paused => pauses += 1,
        }
    };
    assert_eq!(v, "caught");
    assert!(max_depth > 300, "the chain should get deep: {max_depth}");
    assert!(
        pauses > max_depth,
        "unwinding should be steppable: {pauses}"
    );
}

#[test]
fn a_guard_frame_costs_nothing_on_the_way_out() {
    // The guard is inert for values: a body that returns normally pops
    // the frame and keeps going, so wrapping hot code in a guard doesn't
    // add per-step work beyond the one frame.
    let vm = Vm::new();
    let plain = parse::parse("(+ 1 2)").unwrap();
    let guarded = parse::parse("(guard (e 'unused) (+ 1 2))").unwrap();

    let steps = |e| {
        let mut m = Machine::new(e, vm.env().clone());
        while let Progress::Paused = m.run(u64::MAX).unwrap() {}
        m.steps()
    };
    // Two transitions: one to push the frame, one for the finished value
    // to pop it on the way out. Constant, not per-step — the guard costs
    // nothing while the body runs.
    assert_eq!(steps(guarded), steps(plain) + 2);
}

#[test]
fn conditions_are_ordinary_values() {
    // No new `Val` variant (ADR-041), so a condition is a list and every
    // list operation works on it. The flip side, documented rather than
    // prevented: a hand-built list is indistinguishable from a raised
    // condition.
    assert_eq!(eval("(error? '(error \"hand-built\"))"), "#t");
    assert_eq!(eval("(guard (e (cdr e)) (error \"x\" 1))"), "(\"x\" 1)");
    assert_eq!(eval("(guard (e (car e)) (error \"x\"))"), "error");
    assert_eq!(eval("(guard (e (pair? e)) (error \"x\"))"), "#t");
}

#[test]
fn generated_forms_do_not_go_through_shadowable_bindings() {
    // `error` builds its condition with `list`, and quasiquote builds
    // its spine with `list` and `append`. Resolving those through
    // `Expr::Var` made them ordinary lookups, so a user binding of the
    // same name silently changed what the form meant. They compile to a
    // quoted prim value now, which no binding can intercept.
    assert_eq!(
        eval("(let ((list (lambda (a b) 'wrong))) (guard (e e) (error \"boom\")))"),
        "(error \"boom\")"
    );
    // Same bug, predating conditions: quasiquote against `list` …
    assert_eq!(
        eval("(let ((list (lambda (a b) 'wrong))) `(1 ,(+ 1 1)))"),
        "(1 2)"
    );
    // … and against `append`, on the splice path.
    assert_eq!(
        eval("(let ((append (lambda (a b) 'wrong))) `(1 ,@(list 2 3)))"),
        "(1 2 3)"
    );
    // A global shadow is the same hazard by another route.
    let mut vm = Vm::new();
    vm.eval_str("(define list (lambda (a b) 'wrong))").unwrap();
    assert_eq!(
        format!("{}", vm.eval_str("(guard (e e) (error \"boom\"))").unwrap()),
        "(error \"boom\")"
    );
}

#[test]
fn guard_rejects_malformed_forms_at_compile_time() {
    let mut vm = Vm::new();
    for (src, want) in [
        ("(guard (e) 1)", "guard: expected (var handler)"),
        ("(guard e 1)", "guard: expected (var handler)"),
        ("(guard (1 2) 3)", "guard: var must be a symbol"),
        (
            "(guard (e 1))",
            "guard: expected (guard (var handler) body)",
        ),
        ("(raise)", "raise: expected (raise expr)"),
        ("(error)", "error: expected (error msg irritant ...)"),
    ] {
        let err = vm.eval_str(src).expect_err(src);
        assert_eq!(err.msg, want, "for {src}");
    }
}
