//! The pausable machine (ADR-040): `step::Machine` for a single
//! expression, `Vm::start` / `Vm::resume` for a batch of top-level forms.

use lisp::step::Machine;
use lisp::{Progress, Val, Vm, parse};

/// Drive a machine in `slice`-sized chunks, returning `(value, pauses)`.
fn run_sliced(src: &str, slice: u64) -> (String, u32) {
    let vm = Vm::new();
    let expr = parse::parse(src).expect("parse");
    let mut m = Machine::new(expr, vm.env().clone());
    let mut pauses = 0;
    loop {
        match m.run(slice).expect("eval") {
            Progress::Done(v) => return (format!("{v}"), pauses),
            Progress::Paused => pauses += 1,
        }
    }
}

#[test]
fn slicing_does_not_change_the_answer() {
    // The whole premise: where you stop is not observable in the result.
    for slice in [1, 2, 3, 7, 100, u64::MAX] {
        let (v, _) = run_sliced("(+ 1 (* 2 (- 10 4)))", slice);
        assert_eq!(v, "13", "slice size {slice} changed the answer");
    }
    // Smaller slices really are pausing, not silently running through.
    let (_, many) = run_sliced("(+ 1 (* 2 (- 10 4)))", 1);
    let (_, none) = run_sliced("(+ 1 (* 2 (- 10 4)))", u64::MAX);
    assert!(many > 5, "single-stepping should pause repeatedly: {many}");
    assert_eq!(none, 0, "an unbounded slice should never pause");
}

#[test]
fn step_once_advances_exactly_one_transition() {
    let vm = Vm::new();
    let mut m = Machine::new(parse::parse("(+ 1 2)").unwrap(), vm.env().clone());
    assert_eq!(m.steps(), 0);
    for expected in 1..=3 {
        m.step_once().expect("step");
        assert_eq!(m.steps(), expected);
    }
    // And running to completion keeps counting from there.
    let before = m.steps();
    while let Progress::Paused = m.run(u64::MAX).unwrap() {}
    assert!(m.steps() > before);
}

#[test]
fn a_finished_machine_refuses_to_run_again() {
    let vm = Vm::new();
    let mut m = Machine::new(parse::parse("7").unwrap(), vm.env().clone());
    while let Progress::Paused = m.run(u64::MAX).unwrap() {}
    assert!(m.is_done());
    let err = m.run(1).expect_err("already finished");
    assert_eq!(err.msg, "machine has already finished");
}

#[test]
fn a_paused_machine_reports_where_it_is() {
    let vm = Vm::new();
    let src = "(+ 1\n   (car '()))";
    let expr = parse::parse(src).unwrap();
    let mut m = Machine::new(expr, vm.env().clone());

    // Step until the machine is sitting on the inner call. Its position
    // is line 2, and the outer `+` call is on the continuation chain.
    let mut found = None;
    for _ in 0..20 {
        if let Some(p) = m.position()
            && p.line == 2
        {
            found = Some((p, m.depth(), m.backtrace()));
            break;
        }
        if let Progress::Done(_) = m.step_once().expect("step") {
            break;
        }
    }
    let (pos, depth, trace) = found.expect("machine should pass through the inner call");
    assert_eq!((pos.line, pos.col), (2, 4));
    assert!(depth >= 1, "the enclosing `+` call is still pending");
    assert_eq!(
        trace.first().map(|s| (s.line, s.col)),
        Some((1, 1)),
        "innermost enclosing call site is the outer `+` on line 1"
    );
}

#[test]
fn the_machine_holds_a_value_between_producing_and_consuming_it() {
    let vm = Vm::new();
    let mut m = Machine::new(parse::parse("(+ 1 2)").unwrap(), vm.env().clone());
    let mut seen = Vec::new();
    for _ in 0..12 {
        if let Some(v) = m.value() {
            seen.push(format!("{v}"));
        }
        if let Progress::Done(_) = m.step_once().expect("step") {
            break;
        }
    }
    // `+` resolves to a prim, then each literal argument passes through.
    assert!(seen.contains(&"1".to_string()), "saw {seen:?}");
    assert!(seen.contains(&"2".to_string()), "saw {seen:?}");
}

#[test]
fn tail_calls_do_not_grow_machine_depth() {
    // `tail_calls_dont_grow_the_stack` asserts this indirectly, by not
    // overflowing. With the machine exposed, the property is directly
    // observable: the continuation chain stays flat across the loop.
    let vm = Vm::new();
    let src = "(letrec ((loop (lambda (n) (if (= n 0) 'done (loop (- n 1))))))
                 (loop 500))";
    let mut m = Machine::new(parse::parse(src).unwrap(), vm.env().clone());
    let mut max_depth = 0;
    let result = loop {
        max_depth = max_depth.max(m.depth());
        match m.step_once().expect("step") {
            Progress::Done(v) => break format!("{v}"),
            Progress::Paused => continue,
        }
    };
    assert_eq!(result, "done");
    assert!(
        max_depth < 10,
        "500 tail calls should not stack frames; peak depth was {max_depth}"
    );
}

// ── Vm-level sessions ─────────────────────────────────────────────────

#[test]
fn a_session_runs_a_batch_across_many_slices() {
    let mut vm = Vm::new();
    let mut s = vm
        .start("(define a 2) (define b (* a 3)) (+ a b)")
        .expect("start");
    assert_eq!(s.forms_total(), 3);
    assert_eq!(s.forms_done(), 0);

    let mut pauses = 0;
    let v = loop {
        match vm.resume(&mut s, 2).expect("resume") {
            Progress::Done(v) => break format!("{v}"),
            Progress::Paused => pauses += 1,
        }
    };
    assert_eq!(v, "8");
    assert!(pauses > 1, "2-step slices should pause repeatedly");
}

#[test]
fn completed_forms_are_visible_mid_session() {
    // Bindings live in the Vm, not the session, so a host can render
    // progress as a prelude installs rather than waiting for the batch.
    let mut vm = Vm::new();
    let mut s = vm.start("(define a 1) (define b 2) (+ a b)").unwrap();

    // Every define in the batch gets its cell up front, so mutual
    // recursion resolves; the cells hold `#f` until their bodies run.
    // Presence therefore says nothing about progress — the value does.
    assert!(matches!(vm.global("b"), Some(Val::Bool(false))));

    while !matches!(vm.global("a"), Some(Val::Num(1))) {
        match vm.resume(&mut s, 1).expect("resume") {
            Progress::Done(_) => panic!("finished before `a` was assigned"),
            Progress::Paused => {}
        }
    }
    assert!(s.forms_done() >= 1);
    assert!(
        matches!(vm.global("b"), Some(Val::Bool(false))),
        "`b`'s body hasn't run yet"
    );
}

#[test]
fn a_slice_is_not_the_form_budget() {
    // The distinction that makes the two limits coexist: a host pumping
    // tiny slices to stay responsive must not thereby trip the runaway
    // guard. Same expression, budget well above its real cost, slices far
    // below it.
    let mut vm = Vm::new();
    vm.set_step_budget(100_000);
    let src = "(letrec ((loop (lambda (n acc) (if (= n 0) acc (loop (- n 1) (+ acc n))))))
                 (loop 200 0))";
    let mut s = vm.start(src).unwrap();
    let v = loop {
        match vm
            .resume(&mut s, 5)
            .expect("slices must not exhaust the budget")
        {
            Progress::Done(v) => break format!("{v}"),
            Progress::Paused => continue,
        }
    };
    assert_eq!(v, "20100");
}

#[test]
fn the_form_budget_still_catches_a_runaway() {
    let mut vm = Vm::new();
    vm.set_step_budget(500);
    let mut s = vm
        .start("(letrec ((spin (lambda () (spin)))) (spin))")
        .unwrap();
    let err = loop {
        match vm.resume(&mut s, 10) {
            Ok(Progress::Paused) => continue,
            Ok(Progress::Done(_)) => panic!("an infinite loop should not finish"),
            Err(e) => break e,
        }
    };
    assert_eq!(err.msg, "execution exceeded step budget");
}

#[test]
fn the_budget_is_still_per_form_not_per_session() {
    // Pins the documented `step_budget` semantics through the new driver:
    // three forms that each fit the budget are fine even though their sum
    // does not.
    let mut vm = Vm::new();
    vm.set_step_budget(60);
    let one = "(+ 1 (+ 2 (+ 3 4)))";
    let src = format!("{one} {one} {one}");
    let mut s = vm.start(&src).unwrap();
    let v = loop {
        match vm.resume(&mut s, u64::MAX).expect("each form fits") {
            Progress::Done(v) => break format!("{v}"),
            Progress::Paused => continue,
        }
    };
    assert_eq!(v, "10");
}

#[test]
fn a_failed_session_rolls_back_the_globals_table() {
    // Parity with `eval_datums`: a failed define must not leave a
    // placeholder cell shadowing a builtin.
    let mut vm = Vm::new();
    let mut s = vm.start("(define + 5) (car 1)").unwrap();
    let err = loop {
        match vm.resume(&mut s, 3) {
            Ok(Progress::Paused) => continue,
            Ok(Progress::Done(_)) => panic!("(car 1) should fail"),
            Err(e) => break e,
        }
    };
    assert!(err.msg.contains("pair"), "unexpected: {err}");
    // `+` is a prim again, not 5.
    assert_eq!(format!("{}", vm.eval_str("(+ 1 2)").unwrap()), "3");
}

#[test]
fn an_abandoned_session_keeps_what_already_ran() {
    // Abandoning is a decision, not a failure: no rollback. This is the
    // "cancel a runaway cast and keep the Vm" case.
    let mut vm = Vm::new();
    let mut s = vm.start("(define kept 42) (define spin 1) 99").unwrap();
    while vm.global("kept").map(|v| format!("{v}")).as_deref() != Some("42") {
        match vm.resume(&mut s, 1).expect("resume") {
            Progress::Done(_) => break,
            Progress::Paused => {}
        }
    }
    drop(s);
    assert_eq!(format!("{}", vm.global("kept").expect("kept")), "42");
    // And the Vm is still usable afterwards.
    assert_eq!(format!("{}", vm.eval_str("(+ kept 1)").unwrap()), "43");
}

#[test]
fn eval_str_still_behaves_exactly_as_before() {
    // `eval_str` is now `start` + one unbounded `resume`, so its whole
    // contract is worth re-pinning here: last expression's value, `#t`
    // for an all-defines batch, mutual recursion across the batch.
    let mut vm = Vm::new();
    assert_eq!(format!("{}", vm.eval_str("1 2 3").unwrap()), "3");
    assert_eq!(
        format!("{}", vm.eval_str("(define x 1) (define y 2)").unwrap()),
        "#t"
    );
    let src = "(define even? (lambda (n) (if (= n 0) #t (odd? (- n 1)))))
               (define odd?  (lambda (n) (if (= n 0) #f (even? (- n 1)))))
               (even? 10)";
    assert_eq!(format!("{}", vm.eval_str(src).unwrap()), "#t");
}

#[test]
fn a_syntax_error_surfaces_from_start_not_resume() {
    // Reading and the define pre-pass both happen up front, so a host
    // doesn't have to begin evaluating to find out the source is bad.
    let mut vm = Vm::new();
    let err = vm.start("(+ 1").expect_err("unbalanced");
    assert_eq!(err.msg, "unclosed (");
    assert_eq!(err.span.map(|s| s.col), Some(1));
}

#[test]
fn a_malformed_define_does_not_leave_sibling_cells_behind() {
    // The pre-pass installs cells for every define in the batch before
    // any runs, so bailing partway has to undo the ones already added.
    let mut vm = Vm::new();
    let err = vm
        .start("(define good 1) (define)")
        .expect_err("bad define");
    assert!(err.msg.contains("define:"), "unexpected: {err}");
    assert!(
        vm.global("good").is_none(),
        "a cell from the aborted pre-pass is still bound"
    );
}

#[test]
fn a_session_can_be_introspected_while_paused() {
    let mut vm = Vm::new();
    let mut s = vm.start("(+ 1 2) (* 3 4)").unwrap();
    vm.resume(&mut s, 2).expect("resume");
    let m = s.machine().expect("a form is in flight");
    assert!(m.steps() > 0);
    assert!(!m.is_done());
    // First form still running, so nothing has produced a value yet.
    assert_eq!(format!("{}", s.last_value()), "#t");
}

#[test]
fn sessions_hold_no_borrow_of_the_vm() {
    // The property that makes this usable from an event loop: a session
    // can be parked in a struct field and resumed across turns, with the
    // Vm free in between.
    struct Host {
        vm: Vm,
        pending: Option<lisp::Session>,
    }
    let mut host = Host {
        vm: Vm::new(),
        pending: None,
    };
    host.pending = Some(host.vm.start("(define n 6) (* n 7)").unwrap());
    let answer = loop {
        let mut s = host.pending.take().expect("pending");
        // Unrelated Vm work between slices, as a host would do.
        let _ = host.vm.global("n");
        match host.vm.resume(&mut s, 1).expect("resume") {
            Progress::Done(v) => break format!("{v}"),
            Progress::Paused => host.pending = Some(s),
        }
    };
    assert_eq!(answer, "42");
    assert!(matches!(host.vm.global("n"), Some(Val::Num(6))));
}
