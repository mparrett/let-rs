use lisp::{LispErr, Span, Val, Vm};

fn eval(src: &str) -> String {
    let mut vm = Vm::new();
    format!("{}", vm.eval_str(src).expect("eval"))
}

fn evals(srcs: &[&str]) -> String {
    let mut vm = Vm::new();
    let mut last = String::new();
    for src in srcs {
        last = format!("{}", vm.eval_str(src).expect("eval"));
    }
    last
}

#[test]
fn literals() {
    assert_eq!(eval("42"), "42");
    assert_eq!(eval("#t"), "#t");
    assert_eq!(eval("#f"), "#f");
}

#[test]
fn arithmetic_variadic() {
    assert_eq!(eval("(+)"), "0");
    assert_eq!(eval("(+ 1 2 3 4 5)"), "15");
    assert_eq!(eval("(- 10)"), "-10");
    assert_eq!(eval("(- 10 1 2 3)"), "4");
    assert_eq!(eval("(*)"), "1");
    assert_eq!(eval("(* 2 3 4)"), "24");
    assert_eq!(eval("(/ 100 2 5)"), "10");
    assert_eq!(eval("(mod 17 5)"), "2");
}

#[test]
fn rational_literals_normalize_at_read_time() {
    // Lowest terms; denominator always positive.
    assert_eq!(eval("1/2"), "1/2");
    assert_eq!(eval("2/4"), "1/2");
    assert_eq!(eval("-3/6"), "-1/2");
    // Sign migrates to numerator. `-1/-2` isn't valid input — parser
    // wants a single sign on the numerator — but a Ratio constructed
    // from negative-den intermediates normalizes to positive-den form.
    // `1/1` collapses to an integer.
    assert_eq!(eval("1/1"), "1");
    assert_eq!(eval("4/2"), "2");
    assert_eq!(eval("-6/3"), "-2");
}

#[test]
fn rational_arithmetic() {
    assert_eq!(eval("(+ 1/2 1/3)"), "5/6");
    assert_eq!(eval("(- 1/2 1/3)"), "1/6");
    assert_eq!(eval("(* 2/3 3/4)"), "1/2");
    assert_eq!(eval("(/ 1/2 1/4)"), "2");
    // Negation of a ratio.
    assert_eq!(eval("(- 1/3)"), "-1/3");
}

#[test]
fn rational_add_with_huge_common_denominator() {
    // Pre-fix: cross-multiplied unreduced and overflowed i128 when both
    // denominators were near u64::MAX. The fix pre-gcd-reduces so common
    // denominators cancel cleanly.
    let n = u64::MAX; // = 18446744073709551615
    let src = format!("(+ 1/{n} 1/{n})");
    let expected = format!("2/{n}");
    assert_eq!(eval(&src), expected);
}

#[test]
fn rational_overflow_errors_cleanly() {
    // Three i64::MAX factors overflow even our i128 accumulator. We
    // want a clean Err — debug-mode `unwrap_or` panic in the previous
    // implementation is exactly what the codex review flagged.
    let mut vm = Vm::new();
    let r = vm.eval_str("(* 9223372036854775807 9223372036854775807 9223372036854775807)");
    assert!(
        matches!(&r, Err(e) if e.msg.contains("numeric overflow")),
        "expected overflow error, got {r:?}"
    );
}

#[test]
fn division_of_integers_produces_a_ratio() {
    // Breaking change from integer-div semantics: `/` is now exact.
    assert_eq!(eval("(/ 1 4)"), "1/4");
    assert_eq!(eval("(/ 10 4)"), "5/2");
    // Existing integer-divisible case still returns an int.
    assert_eq!(eval("(/ 100 2 5)"), "10");
}

#[test]
fn mixed_int_ratio_arithmetic() {
    assert_eq!(eval("(+ 1 1/2)"), "3/2");
    assert_eq!(eval("(+ 1/2 1)"), "3/2");
    assert_eq!(eval("(* 4 1/2)"), "2");
    assert_eq!(eval("(- 1 1/3)"), "2/3");
}

#[test]
fn rational_comparison_across_types() {
    assert_eq!(eval("(= 1 1/1)"), "#t");
    assert_eq!(eval("(= 1/2 2/4)"), "#t");
    assert_eq!(eval("(< 1/3 1/2)"), "#t");
    assert_eq!(eval("(< 1/3 1/2 2/3)"), "#t");
    assert_eq!(eval("(<= 1/2 1/2)"), "#t");
    assert_eq!(eval("(> 2/3 1/2 1/3)"), "#t");
    // Mixed int/ratio comparisons.
    assert_eq!(eval("(< 1/2 1)"), "#t");
    assert_eq!(eval("(< 0 1/2)"), "#t");
}

#[test]
fn rational_eq_q_and_number_q() {
    assert_eq!(eval("(eq? 1/2 1/2)"), "#t");
    // Normalized — these are the same Val::Ratio.
    assert_eq!(eval("(eq? 1/2 2/4)"), "#t");
    // An integer-valued ratio collapses to Num, so eq? to an int succeeds.
    assert_eq!(eval("(eq? 4 4/1)"), "#t");
    assert_eq!(eval("(number? 1/2)"), "#t");
    assert_eq!(eval("(number? 42)"), "#t");
    assert_eq!(eval("(number? 'foo)"), "#f");
}

#[test]
fn rational_accessors() {
    // numerator / denominator on ratios.
    assert_eq!(eval("(numerator 3/4)"), "3");
    assert_eq!(eval("(denominator 3/4)"), "4");
    assert_eq!(eval("(numerator -3/4)"), "-3");
    assert_eq!(eval("(denominator -3/4)"), "4");
    // Integers behave like (n / 1).
    assert_eq!(eval("(numerator 5)"), "5");
    assert_eq!(eval("(denominator 5)"), "1");
    assert_eq!(eval("(numerator -7)"), "-7");
    assert_eq!(eval("(denominator -7)"), "1");
}

#[test]
fn floor_and_ceiling() {
    assert_eq!(eval("(floor 7/2)"), "3");
    assert_eq!(eval("(ceiling 7/2)"), "4");
    // Negative rationals: floor rounds toward -infinity, ceiling toward 0.
    assert_eq!(eval("(floor -7/2)"), "-4");
    assert_eq!(eval("(ceiling -7/2)"), "-3");
    // Exact integers pass through unchanged.
    assert_eq!(eval("(floor 4)"), "4");
    assert_eq!(eval("(ceiling 4)"), "4");
    assert_eq!(eval("(floor -4)"), "-4");
    assert_eq!(eval("(ceiling -4)"), "-4");
    // Composes with arithmetic.
    assert_eq!(eval("(* 4 (floor 7/2))"), "12");
    assert_eq!(eval("(+ (floor 5/3) (ceiling 5/3))"), "3");
}

#[test]
fn rational_errors() {
    let mut vm = Vm::new();
    // Division by zero (any form).
    assert!(vm.eval_str("(/ 1 0)").is_err());
    assert!(vm.eval_str("(/ 1/2 0)").is_err());
    // mod stays integer-only.
    assert!(vm.eval_str("(mod 1/2 2)").is_err());
    assert!(vm.eval_str("(mod 4 1/2)").is_err());
    // `1/0` in source is a symbol (parser rejects ratio with zero den).
    // So it's an unbound variable at eval, not a ratio error.
    let r = vm.eval_str("1/0");
    assert!(r.is_err(), "1/0 should be rejected (unbound symbol): {r:?}");
}

#[test]
fn comparison_chains() {
    assert_eq!(eval("(< 1 2 3)"), "#t");
    assert_eq!(eval("(< 1 2 2)"), "#f");
    assert_eq!(eval("(<= 1 2 2)"), "#t");
    assert_eq!(eval("(= 4 4 4)"), "#t");
    assert_eq!(eval("(> 3 2 1)"), "#t");
}

#[test]
fn lambda_application() {
    assert_eq!(eval("((lambda (x) (+ x 1)) 5)"), "6");
    assert_eq!(eval("((λ (x) (* x x)) 7)"), "49");
}

#[test]
fn closure_captures_lexical_env() {
    assert_eq!(eval("(((lambda (x) (lambda (y) (+ x y))) 3) 4)"), "7");
}

#[test]
fn if_form() {
    assert_eq!(eval("(if #t 1 2)"), "1");
    assert_eq!(eval("(if #f 1 2)"), "2");
    assert_eq!(eval("(if (= 3 3) (+ 1 1) 99)"), "2");
}

#[test]
fn quote_atoms() {
    assert_eq!(eval("'foo"), "foo");
    assert_eq!(eval("(quote bar)"), "bar");
    assert_eq!(eval("'()"), "()");
    assert_eq!(eval("'42"), "42");
}

#[test]
fn quote_lists() {
    assert_eq!(eval("'(1 2 3)"), "(1 2 3)");
    assert_eq!(eval("'(a (b c) d)"), "(a (b c) d)");
}

#[test]
fn cons_car_cdr() {
    assert_eq!(eval("(cons 1 2)"), "(1 . 2)");
    assert_eq!(eval("(cons 1 (cons 2 (cons 3 '())))"), "(1 2 3)");
    assert_eq!(eval("(car '(a b c))"), "a");
    assert_eq!(eval("(cdr '(a b c))"), "(b c)");
    assert_eq!(eval("(list 1 2 3)"), "(1 2 3)");
}

#[test]
fn predicates() {
    assert_eq!(eval("(null? '())"), "#t");
    assert_eq!(eval("(null? '(1))"), "#f");
    assert_eq!(eval("(pair? '(1))"), "#t");
    assert_eq!(eval("(pair? '())"), "#f");
    assert_eq!(eval("(number? 42)"), "#t");
    assert_eq!(eval("(symbol? 'foo)"), "#t");
    assert_eq!(eval("(eq? 'a 'a)"), "#t");
    assert_eq!(eval("(eq? 'a 'b)"), "#f");
    assert_eq!(eval("(eq? 1 1)"), "#t");
}

#[test]
fn let_form() {
    assert_eq!(eval("(let ((x 1) (y 2)) (+ x y))"), "3");
    // RHS is evaluated in the OUTER env — referring to a sibling fails
    let mut vm = Vm::new();
    assert!(vm.eval_str("(let ((x 1) (y x)) y)").is_err());
}

#[test]
fn let_star_form() {
    // let* allows each binding to see earlier ones
    assert_eq!(
        eval("(let* ((x 1) (y (+ x 1)) (z (+ y 1))) (+ x y z))"),
        "6"
    );
}

#[test]
fn cond_form() {
    let src = r#"
        (cond ((= 1 2) 'a)
              ((= 1 1) 'b)
              (else 'c))
    "#;
    assert_eq!(eval(src), "b");
    assert_eq!(eval("(cond ((= 1 1) 99))"), "99");
    assert_eq!(eval("(cond (else 7))"), "7");
}

#[test]
fn failed_define_does_not_corrupt_env() {
    // Pre-fix the pre-pass cell for `+` was already installed when the
    // body `(/ 1 0)` failed, leaving env with `+` bound to the placeholder
    // (Val::Bool(false)) — every subsequent call to `+` then errored with
    // "not callable: #f". After the fix, env rolls back atomically.
    let mut vm = Vm::new();
    assert_eq!(format!("{}", vm.eval_str("(+ 1 2)").unwrap()), "3");
    let r = vm.eval_str("(define + (/ 1 0))");
    assert!(r.is_err(), "define with failing body should error: {r:?}");
    // The builtin must still work after the failed redefinition.
    assert_eq!(format!("{}", vm.eval_str("(+ 1 2)").unwrap()), "3");
}

#[test]
fn failed_define_in_batch_rolls_back_earlier_defines() {
    // Both defines in one eval_str — atomicity means a later failure
    // also rolls back the earlier success.
    let mut vm = Vm::new();
    let r = vm.eval_str("(define foo 1) (define bar (/ 1 0))");
    assert!(r.is_err());
    let r2 = vm.eval_str("foo");
    assert!(
        r2.is_err(),
        "foo should be undefined after rollback: {r2:?}"
    );
}

#[test]
fn cond_else_must_be_terminal() {
    // Before: returned 'wrong because reverse iteration filled tail with
    // 'right then overwrote with else. After: rejected at compile time.
    let mut vm = Vm::new();
    let r = vm.eval_str("(cond (else 'wrong) (#t 'right))");
    assert!(r.is_err(), "non-terminal else should reject: {r:?}");
}

#[test]
fn letrec_self_recursion() {
    // factorial without Y
    let src = r#"
        (letrec ((fact (lambda (n)
                         (if (= n 0) 1 (* n (fact (- n 1)))))))
          (fact 6))
    "#;
    assert_eq!(eval(src), "720");
}

#[test]
fn letrec_mutual_recursion() {
    let src = r#"
        (letrec ((even? (lambda (n) (if (= n 0) #t (odd?  (- n 1)))))
                 (odd?  (lambda (n) (if (= n 0) #f (even? (- n 1))))))
          (list (even? 10) (odd? 10) (even? 7) (odd? 7)))
    "#;
    assert_eq!(eval(src), "(#t #f #f #t)");
}

#[test]
fn step_budget_catches_nonterminating_eval() {
    // (f) self-recursive in tail position — loops forever in CEK steps.
    // Default Vm budget is unlimited; we cap it explicitly here.
    let mut vm = Vm::new();
    vm.set_step_budget(10_000);
    let r = vm.eval_str("(letrec ((f (lambda () (f)))) (f))");
    assert!(
        matches!(&r, Err(e) if e.msg.contains("step budget")),
        "expected step-budget error, got {r:?}"
    );
}

#[test]
fn step_budget_default_unlimited_preserves_existing_tests() {
    // Sanity: the 100k tail-call test below must still pass without
    // setting any budget — default is u64::MAX.
    let mut vm = Vm::new();
    let src = "(letrec ((loop (lambda (n) (if (= n 0) 0 (loop (- n 1)))))) (loop 1000))";
    assert_eq!(format!("{}", vm.eval_str(src).unwrap()), "0");
}

#[test]
fn step_budget_resets_per_eval_str_call() {
    // After a budget-exhausted call, the next call gets a fresh budget.
    let mut vm = Vm::new();
    vm.set_step_budget(5_000);
    let r = vm.eval_str("(letrec ((f (lambda () (f)))) (f))");
    assert!(r.is_err());
    // Subsequent simple eval succeeds — budget didn't carry over and env
    // was rolled back (the failed letrec didn't pollute self.env either).
    assert_eq!(format!("{}", vm.eval_str("(+ 1 2)").unwrap()), "3");
}

#[test]
fn tail_calls_dont_grow_the_stack() {
    let src = r#"
        (letrec ((loop (lambda (n)
                         (if (= n 0) 0 (loop (- n 1))))))
          (loop 100000))
    "#;
    assert_eq!(eval(src), "0");
}

#[test]
fn assoc_get_prim() {
    // The reader has no dotted-pair syntax, so build pairs with `cons`
    // (matches the shape both demo preludes use in practice).
    let alist = "(list (cons 'a 1) (cons 'b 2) (cons 'c 3))";
    assert_eq!(eval(&format!("(assoc-get 'b {alist})")), "2");
    // Earlier key shadows later one.
    assert_eq!(
        eval("(assoc-get 'k (list (cons 'k 'first) (cons 'k 'second)))"),
        "first"
    );
    // Missing key returns '().
    assert_eq!(eval(&format!("(assoc-get 'z {alist})")), "()");
    // Empty alist returns '().
    assert_eq!(eval("(assoc-get 'a '())"), "()");
}

#[test]
fn map_via_letrec() {
    // Demonstrates closures + letrec + list ops together
    let src = r#"
        (letrec ((map (lambda (f xs)
                        (if (null? xs)
                            '()
                            (cons (f (car xs)) (map f (cdr xs)))))))
          (map (lambda (n) (* n n)) '(1 2 3 4 5)))
    "#;
    assert_eq!(eval(src), "(1 4 9 16 25)");
}

#[test]
fn quasiquote_basics() {
    assert_eq!(eval("`5"), "5");
    assert_eq!(eval("`foo"), "foo");
    assert_eq!(eval("`(1 2 3)"), "(1 2 3)");
    assert_eq!(eval("`(1 ,(+ 1 1) 3)"), "(1 2 3)");
}

#[test]
fn quasiquote_splice() {
    assert_eq!(eval("(let ((xs '(2 3))) `(1 ,@xs 4))"), "(1 2 3 4)");
    assert_eq!(
        eval("(let ((xs '(a b c))) `(begin ,@xs done))"),
        "(begin a b c done)"
    );
    assert_eq!(eval("(let ((xs '())) `(x ,@xs y))"), "(x y)");
}

#[test]
fn quasiquote_nested_depth_preserved() {
    // Inner ,x must NOT fire — it's nested under a second quasiquote.
    // Before the depth-tracking fix the inner unquote leaked through and
    // produced (outer (quasiquote (inner 7))).
    let result = eval("(let ((x 7)) `(outer `(inner ,x)))");
    assert!(
        result.contains("inner") && result.contains('x'),
        "nested quasiquote dropped its inner shape: {result}"
    );
    assert!(
        !result.contains('7'),
        "inner unquote fired prematurely: {result}"
    );
}

// Macro tests moved to crates/macros/tests/macros.rs (ADR-024).
// What stays here are parser-level quasiquote tests above — those
// test list-construction syntax that works without macros.

#[test]
fn multiple_top_level_evals_share_state() {
    // Sanity: Vm carries env across eval_str calls.
    assert_eq!(evals(&["(+ 1 2)", "(* 4 5)", "((lambda (x) x) 'hi)"]), "hi");
}

#[test]
fn multiple_forms_in_one_eval_str() {
    // eval_str now accepts a sequence of top-level forms; returns the
    // value of the last value-producing one.
    assert_eq!(eval("(+ 1 1) (+ 2 2) (+ 3 3)"), "6");
}

#[test]
fn define_extends_env() {
    let mut vm = Vm::new();
    vm.eval_str("(define x 7)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(* x 6)").unwrap()), "42");
}

#[test]
fn define_self_recursion() {
    // The classic test: define a recursive lambda referring to its own name.
    let src = r#"
        (define fact (lambda (n) (if (= n 0) 1 (* n (fact (- n 1))))))
        (fact 6)
    "#;
    assert_eq!(eval(src), "720");
}

#[test]
fn define_persists_across_eval_str_calls() {
    let mut vm = Vm::new();
    vm.eval_str("(define greet (lambda (name) (list 'hello name)))")
        .unwrap();
    assert_eq!(
        format!("{}", vm.eval_str("(greet 'world)").unwrap()),
        "(hello world)"
    );
}

#[test]
fn define_returns_marker_then_expression_wins() {
    // A mixed sequence — defines + expressions — returns the last expr.
    assert_eq!(eval("(define x 10) (define y 20) (+ x y) 'done"), "done");
}

#[test]
fn defines_in_one_eval_str_are_mutually_recursive() {
    // Two defines in the same source — each refers to the other.
    // The pre-pass allocates both placeholder cells before either
    // body evaluates, so their lambdas capture an env containing
    // both names.
    let src = r#"
        (define even? (lambda (n) (if (= n 0) #t (odd?  (- n 1)))))
        (define odd?  (lambda (n) (if (= n 0) #f (even? (- n 1)))))
        (list (even? 10) (odd? 10) (even? 7) (odd? 7))
    "#;
    assert_eq!(eval(src), "(#t #f #f #t)");
}

#[test]
fn three_way_mutual_recursion() {
    let src = r#"
        (define a (lambda (n) (if (= n 0) 'done-a (b (- n 1)))))
        (define b (lambda (n) (if (= n 0) 'done-b (c (- n 1)))))
        (define c (lambda (n) (if (= n 0) 'done-c (a (- n 1)))))
        (list (a 0) (a 1) (a 2) (a 3))
    "#;
    assert_eq!(eval(src), "(done-a done-b done-c done-a)");
}

#[test]
fn mutual_recursion_works_across_eval_str_calls() {
    // Top-level defines live in a Vm-owned globals table (ADR-015);
    // every closure looks names up through a shared back-edge to that
    // table, not via a snapshot of its capture-time env. So a forward
    // reference resolved at call time finds the later definition.
    let mut vm = Vm::new();
    vm.eval_str("(define foo (lambda () (bar)))").unwrap();
    vm.eval_str("(define bar (lambda () 42))").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(foo)").unwrap()), "42");
}

#[test]
fn define_only_valid_at_top_level() {
    let mut vm = Vm::new();
    let r = vm.eval_str("(let ((x 1)) (define y 2))");
    assert!(r.is_err(), "nested define should error: {r:?}");
}

#[test]
fn dropping_vm_releases_top_level_closures() {
    // issue_2: pre-fix, every top-level `define` of a lambda formed an
    // Rc cycle (env-frame slot → closure → captured env → frame →
    // slot), so installing a prelude permanently anchored its
    // closures. The globals-table redesign (ADR-015) replaces the
    // back-edge with a `Weak`. After the Vm is dropped, no strong ref
    // path keeps a prelude cell alive — `Weak::upgrade` returns None.
    let mut mvm = macros::MacroVm::new();
    spells::install(&mut mvm);
    // Grab a weak handle to one of the installed closure cells.
    let weak = mvm
        .vm
        .global_cell_weak("fire")
        .expect("spells prelude defines fire");
    assert!(
        weak.upgrade().is_some(),
        "sanity: cell is live while Vm is alive"
    );
    drop(mvm);
    assert!(
        weak.upgrade().is_none(),
        "dropping the Vm should release every prelude closure cell"
    );
}

// ADR-020: prims live in the same globals table as user defines.
// `(define + 5)` overwrites the prim slot. Lexical `(let ((+ 5)) …)`
// still shadows via the frame walk because lookup walks frames before
// falling through to globals.

#[test]
fn define_over_prim_overwrites_globals_slot() {
    let mut vm = Vm::new();
    vm.eval_str("(define + 5)").unwrap();
    // After the overwrite, looking up `+` returns the new value, not
    // the original prim. Pre-ADR-020 this returned `+`'s Prim repr
    // because lookup walked the prim frame chain first.
    assert_eq!(format!("{}", vm.eval_str("+").unwrap()), "5");
}

#[test]
fn define_over_prim_then_call_errors() {
    let mut vm = Vm::new();
    vm.eval_str("(define + 5)").unwrap();
    let r = vm.eval_str("(+ 1 2)");
    assert!(
        r.as_ref().is_err_and(|e| e.msg.starts_with("not callable")),
        "expected 'not callable' error, got {r:?}",
    );
}

#[test]
fn prim_still_callable_in_lexical_scope() {
    // `let` bindings live in env frames; lookup walks frames before
    // falling through to globals. So a `let`-shadowed `+` resolves to
    // the let binding, exactly as it did pre-ADR-020 when the prim
    // itself was in the base frame chain.
    assert_eq!(eval("(let ((+ 100)) +)"), "100");
}

#[test]
fn letrec_does_not_leak() {
    // ADR-023 regression: the letrec cycle that ADR-021 pinned
    // (Frame → cell → Val::Clo → closure.env → Frame) is dissolved
    // by the CESK store. Frame slots are now `Addr` (Copy) indices
    // into a Vm-owned `Store`; closures Rc-reach the env, which holds
    // a `Weak<Store>`, so no closure can root its own store. When the
    // Vm drops, the store drops in one shot — observable through the
    // probe taken before the Vm dropped.
    let mut vm = lisp::Vm::new();
    let store = vm.store_probe();
    let v = vm.eval_str("(letrec ((f (lambda () (f)))) f)").unwrap();
    // Sanity: the letrec did allocate at least one slot.
    assert_eq!(
        store.is_empty(),
        Some(false),
        "letrec should have allocated at least one store slot"
    );
    drop(v);
    drop(vm);
    assert!(
        !store.is_alive(),
        "after Vm drop, the store must release. If this fires, some \
         closure is rooting the store — the ADR-021 cycle has come back."
    );
}

#[test]
fn store_reclaims_frame_slots() {
    // ADR-033 regression. Before reclamation the CESK store was
    // append-only: every binding ever created stayed allocated for the
    // life of the Vm, so a 10k-iteration loop cost 20k slots and a
    // second run cost 20k more. A long-lived host (the Spell Lab's
    // tick interval, the web REPL) grew without bound.
    //
    // The property that matters isn't "small" — it's "flat". For a
    // binding that doesn't outlive its frame, slot count must be a
    // function of live environment depth, not of how much evaluation
    // has happened.
    //
    // Each case below is a *different* frame shape, because the first
    // version of this test only exercised the top-level-define one —
    // whose closure captures the root env and so can't form the
    // retention cycle that `recursive_closures_retain_their_slot`
    // pins. Reviewing that gap is what found the cycle.
    let cases = [
        // Closure params, called from a top-level define. The closure
        // captures the *root* env: no frame involved in the capture.
        (
            "(define count (lambda (n acc) (if (= n 0) acc (count (- n 1) (+ acc 1)))))",
            "(count 500 0)",
        ),
        // A closure created and called inside a lexical frame. The
        // frame is real, but nothing self-references, so it dies.
        ("", "(let ((g (lambda (x) (+ x 1)))) (g 1))"),
        // A closure that captures an *enclosing* frame and mutates it.
        // Still no self-reference — the slot holding `f` isn't reached
        // from `f`'s own env.
        ("", "(let ((s 0)) (let ((f (lambda () (set! s 1)))) (f)))"),
        // letrec whose bindings aren't closures at all.
        ("", "(letrec ((a 1) (b 2)) (+ a b))"),
        // Nested lexical frames, several deep.
        ("", "(let ((a 1)) (let ((b 2)) (let ((c 3)) (+ a b c))))"),
    ];

    for (setup, body) in cases {
        let mut vm = Vm::new();
        let store = vm.store_probe();
        if !setup.is_empty() {
            vm.eval_str(setup).unwrap();
        }

        vm.eval_str(body).unwrap();
        let alive = || store.len().expect("store is alive while the Vm is");
        let live = alive();
        let high_water = store.slots().expect("store is alive while the Vm is");

        for _ in 0..20 {
            vm.eval_str(body).unwrap();
        }

        assert_eq!(
            alive(),
            live,
            "{body}: live slot count grew across repeated evaluation — \
             frame slots are no longer being reclaimed (ADR-033)"
        );
        assert_eq!(
            store.slots().expect("store is alive while the Vm is"),
            high_water,
            "{body}: the store's high-water mark grew across repeated \
             evaluation — freed slots are not being reused (ADR-033)"
        );
        assert!(
            high_water < 32,
            "{body}: high-water mark {high_water} is far above live env depth"
        );
    }
}

#[test]
fn recursive_closures_retain_their_slot() {
    // ADR-033's *known residual*, pinned the way ADR-021 pinned the
    // cycle it descends from. This test asserts a limitation, not a
    // feature: if a future change (the trial-deletion sweep sketched in
    // ADR-038) makes these collectable, this test fails loudly and
    // should be rewritten as a reclamation test, not deleted.
    //
    // The shape:
    //
    //   store slot -> Val::Clo -> captured Env -> Rc<Frame> -> owns slot
    //
    // `Frame::drop` frees the slot, but the closure sitting *in* that
    // slot holds the frame alive, so the drop never runs. Refcounting
    // cannot break this without tracing; ADR-033 chose the frame
    // destructor and inherits the gap.
    //
    // This is not a regression — the append-only store retained these
    // too, along with everything else. It is narrower than ADR-033
    // originally claimed, which is why the claim was amended.
    let cases: [(&str, usize); 3] = [
        // Self-recursive letrec: one retained slot per evaluation.
        ("(letrec ((f (lambda () (f)))) 0)", 1),
        // Mutual recursion: one per binding in the cycle.
        (
            "(letrec ((e (lambda (n) (if (= n 0) #t (o (- n 1)))))
                      (o (lambda (n) (if (= n 0) #f (e (- n 1))))))
               (e 4))",
            2,
        ),
        // The same cycle built with set! instead of letrec.
        ("(let ((f 0)) (let ((_ (set! f (lambda () f)))) 0))", 1),
    ];

    for (src, per_eval) in cases {
        let mut vm = Vm::new();
        let store = vm.store_probe();
        vm.eval_str(src).unwrap();
        let live = || store.len().expect("store is alive while the Vm is");
        let after_one = live();
        for _ in 0..9 {
            vm.eval_str(src).unwrap();
        }
        assert_eq!(
            live(),
            after_one + per_eval * 9,
            "{src}: expected {per_eval} retained slot(s) per evaluation. \
             If this now retains *fewer*, the cycle is being collected — \
             good news; see ADR-038 and rewrite this test."
        );
    }
}

#[test]
fn escaped_closures_survive_slot_reuse() {
    // The other half of ADR-033: reclamation must not free a slot that
    // a live closure still names. `burn` churns the free list between
    // captures, so if `mk`'s parameter slot were released while the
    // returned closure still referenced it, `a` and `b` would read
    // recycled bindings and the sum would come out wrong (or equal).
    assert_eq!(
        eval(
            "(letrec ((mk   (lambda (n) (lambda () n)))
                      (burn (lambda (k) (if (= k 0) 0 (burn (- k 1))))))
               (let ((a (mk 1)))
                 (let ((_ (burn 200)))
                   (let ((b (mk 2)))
                     (let ((__ (burn 200)))
                       (+ (a) (b)))))))"
        ),
        "3"
    );
}

// ── set! (ADR-026) ────────────────────────────────────────────────

// Engine-level tests can't use `begin` (it lives in the macros crate)
// so sequencing happens via the `(let ((_ side-effect)) body)` pattern
// that was the standing workaround before ADR-024 shipped begin as a
// macro. set! returns the new value, which is also frequently the
// easiest observation point.

#[test]
fn set_bang_returns_new_value() {
    // (set! x 42) evaluates val, writes, returns val. The let body is
    // the set! expression itself, so the test observes the return.
    assert_eq!(eval("(let ((x 1)) (set! x 42))"), "42");
}

#[test]
fn set_bang_mutates_let_binding() {
    // (let ((_ (set! x 5))) x) — _ binds the set! return value (5);
    // the body then reads x, which has been written. Observes that the
    // store slot for x was actually mutated, not just shadowed.
    assert_eq!(eval("(let ((x 1)) (let ((_ (set! x 5))) x))"), "5");
}

#[test]
fn set_bang_mutates_global() {
    // No frame binding — frame walk falls through to the globals table.
    assert_eq!(evals(&["(define x 1)", "(set! x 99)", "x"]), "99");
}

#[test]
fn set_bang_unbound_errors() {
    let mut vm = Vm::new();
    let r = vm.eval_str("(set! nope 5)");
    assert!(
        matches!(&r, Err(e) if e.msg.contains("unbound") && e.msg.contains("nope")),
        "expected unbound error, got {r:?}"
    );
}

#[test]
fn set_bang_inside_closure_persists_across_calls() {
    // The classic counter: a closure over a let binding that mutates
    // itself. Pre-set!, the only way to get a counter was to thread
    // the value through every call.
    let mut vm = Vm::new();
    vm.eval_str(
        "(define counter \
           (let ((n 0)) \
             (lambda () (let ((_ (set! n (+ n 1)))) n))))",
    )
    .unwrap();
    assert_eq!(format!("{}", vm.eval_str("(counter)").unwrap()), "1");
    assert_eq!(format!("{}", vm.eval_str("(counter)").unwrap()), "2");
    assert_eq!(format!("{}", vm.eval_str("(counter)").unwrap()), "3");
}

#[test]
fn set_bang_lexical_scoping_inner_shadows() {
    // An inner let shadows the outer x; set! inside hits the inner
    // slot, leaving the outer slot intact.
    assert_eq!(
        eval(
            "(let ((x 1)) \
               (let ((_ (let ((x 10)) (set! x 99)))) \
                 x))"
        ),
        "1"
    );
}

#[test]
fn set_bang_evaluates_value_in_current_env() {
    // The value position is a normal expression. The reference to x in
    // (* x 2) resolves against the env at the set! site, including the
    // about-to-be-mutated binding.
    assert_eq!(eval("(let ((x 7)) (let ((_ (set! x (* x 2)))) x))"), "14");
}

#[test]
fn set_bang_malformed_errors() {
    let mut vm = Vm::new();
    assert!(vm.eval_str("(set!)").is_err());
    assert!(vm.eval_str("(set! x)").is_err());
    assert!(vm.eval_str("(set! 5 10)").is_err());
    assert!(vm.eval_str("(set! x 1 2)").is_err());
}

#[test]
fn string_literal_is_self_evaluating_and_displays_quoted() {
    // Display matches `write`: surrounded by quotes with escapes re-applied.
    assert_eq!(eval("\"hi\""), "\"hi\"");
    assert_eq!(eval("\"\""), "\"\"");
}

#[test]
fn string_escapes_round_trip_through_display() {
    // \\, \", \n, \t are the only escapes the tokenizer accepts.
    assert_eq!(eval("\"a\\\"b\""), "\"a\\\"b\"");
    assert_eq!(eval("\"line\\nbreak\""), "\"line\\nbreak\"");
    assert_eq!(eval("\"tab\\there\""), "\"tab\\there\"");
    assert_eq!(eval("\"back\\\\slash\""), "\"back\\\\slash\"");
}

#[test]
fn string_unknown_escape_errors() {
    let mut vm = Vm::new();
    assert!(vm.eval_str("\"bad \\x escape\"").is_err());
}

#[test]
fn unclosed_string_literal_errors() {
    let mut vm = Vm::new();
    assert!(vm.eval_str("\"oops").is_err());
}

#[test]
fn string_predicate() {
    assert_eq!(eval("(string? \"hi\")"), "#t");
    assert_eq!(eval("(string? 'hi)"), "#f");
    assert_eq!(eval("(string? 42)"), "#f");
}

#[test]
fn string_length_counts_chars() {
    assert_eq!(eval("(string-length \"\")"), "0");
    assert_eq!(eval("(string-length \"hello\")"), "5");
    // Multi-byte char counts as 1 grapheme/codepoint, not 3 bytes.
    assert_eq!(eval("(string-length \"é\")"), "1");
}

#[test]
fn string_append_variadic() {
    assert_eq!(eval("(string-append)"), "\"\"");
    assert_eq!(eval("(string-append \"foo\")"), "\"foo\"");
    assert_eq!(eval("(string-append \"foo\" \"-\" \"bar\")"), "\"foo-bar\"");
}

#[test]
fn symbol_string_round_trip() {
    assert_eq!(eval("(symbol->string 'hello)"), "\"hello\"");
    assert_eq!(eval("(string->symbol \"hello\")"), "hello");
    assert_eq!(
        eval("(eq? 'foo (string->symbol (symbol->string 'foo)))"),
        "#t"
    );
}

#[test]
fn number_to_string_handles_num_and_ratio() {
    assert_eq!(eval("(number->string 42)"), "\"42\"");
    assert_eq!(eval("(number->string -7)"), "\"-7\"");
    assert_eq!(eval("(number->string 1/2)"), "\"1/2\"");
}

#[test]
fn eq_q_compares_strings_by_contents() {
    // Strings aren't interned; the engine compares by content the same
    // way it does for symbols (both are Rc<str>).
    assert_eq!(eval("(eq? \"foo\" \"foo\")"), "#t");
    assert_eq!(eval("(eq? \"foo\" \"bar\")"), "#f");
}

// ── robustness at the untrusted boundary ──────────────────────────

#[test]
fn deeply_nested_input_errors_instead_of_overflowing() {
    // `((((…` used to recurse read_datum per level and abort the process
    // with a stack overflow; now it returns a clean error.
    let src = "(".repeat(100_000);
    let mut vm = Vm::new();
    let r = vm.eval_str(&src);
    assert!(
        matches!(&r, Err(e) if e.msg.contains("nesting too deep")),
        "expected nesting error, got {r:?}"
    );
    // A modestly nested (legal) form still evaluates.
    assert_eq!(eval("(car (cdr (cons 1 (cons 2 '()))))"), "2");

    // Reader prefixes nest as freely as parens, and used to recurse the
    // same way. `'`×N is the shape that has no closing token to count.
    let r = vm.eval_str(&"'".repeat(100_000));
    assert!(
        matches!(&r, Err(e) if e.msg.contains("nesting too deep")),
        "expected nesting error, got {r:?}"
    );

    // Reading is a heap bound now, not a stack one (ADR-039). Before
    // `read_datum` became iterative, 1024 levels of *spanned* datums no
    // longer fit in the 2 MiB a Rust test thread gets, and this test
    // aborted the binary rather than failing.
    //
    // 500 rather than something near MAX_DEPTH on purpose: `compile` is
    // still recursive, and on a 2 MiB stack it gives out somewhere
    // between 500 and 750 levels — measured identical before and after
    // this change, so it's a standing limit and not a regression. That
    // means the reader's 1024 cap is *not* a bound the rest of the
    // pipeline can honor; see the residual note in `core-followups.md`.
    let deep = format!("{}1{}", "(list ".repeat(500), ")".repeat(500));
    assert!(
        vm.eval_str(&deep).is_ok(),
        "500 levels should read, compile, and run"
    );
}

#[test]
fn mod_min_by_neg_one_errors_instead_of_panicking() {
    // i64::MIN % -1 overflows inside rem_euclid; must surface as an error.
    let mut vm = Vm::new();
    let r = vm.eval_str("(mod -9223372036854775808 -1)");
    assert!(
        matches!(&r, Err(e) if e.msg.contains("overflow")),
        "expected overflow error, got {r:?}"
    );
    // Ordinary modulo still works.
    assert_eq!(eval("(mod 7 3)"), "1");
    assert_eq!(eval("(mod -7 3)"), "2");
}

#[test]
fn printing_a_long_list_does_not_overflow_the_stack() {
    // write_pair walks the cons spine iteratively; a 200k-element list that
    // previously overflowed the printer now formats fine.
    let src = "(letrec ((loop (lambda (n acc) \
                 (if (= n 0) acc (loop (- n 1) (cons n acc)))))) \
                 (loop 200000 '()))";
    let mut vm = Vm::new();
    let out = format!("{}", vm.eval_str(src).expect("eval"));
    assert!(out.starts_with("(1 2 3 "));
    assert!(out.ends_with(" 200000)"));
}

/// The globals rollback on a failed batch restores the *table*, not the
/// contents of cells that already existed. `set!` (ADR-026) writes
/// through the shared cell, so its effect survives. Locking this in
/// because the rollback predates `set!` and the two read as if they
/// contradict each other.
#[test]
fn failed_batch_restores_bindings_but_not_set_bang_effects() {
    let mut vm = Vm::new();
    vm.eval_str("(define x 1)").unwrap();

    // A failed define can't leave a placeholder shadowing the binding…
    assert!(vm.eval_str("(define x 2) (car 5)").is_err());
    assert_eq!(format!("{}", vm.eval_str("x").unwrap()), "1");

    // …but a set! that ran before the failure is not undone.
    assert!(vm.eval_str("(set! x 99) (car 5)").is_err());
    assert_eq!(format!("{}", vm.eval_str("x").unwrap()), "99");
}

/// The step budget bounds a single top-level form, not an `eval_str`
/// batch: N forms can each spend the full budget.
#[test]
fn step_budget_applies_per_form_not_per_batch() {
    let loop_form = "(letrec ((f (lambda (n) (if (= n 0) 0 (f (- n 1)))))) (f 4000))";

    // One form over budget fails.
    let mut tight = Vm::new();
    tight.set_step_budget(1_000);
    assert!(tight.eval_str(loop_form).is_err());

    // Fifty forms, each under budget, all pass — total steps spent far
    // exceed the budget itself.
    let mut vm = Vm::new();
    vm.set_step_budget(100_000);
    let batch = vec![loop_form; 50].join(" ");
    assert!(vm.eval_str(&batch).is_ok());
}

// ── Vm::global (ADR-037) ──────────────────────────────────────────

#[test]
fn global_reads_top_level_bindings() {
    let mut vm = Vm::new();
    vm.eval_str("(define x 41) (define f (lambda (n) (+ n 1)))")
        .unwrap();
    assert_eq!(format!("{}", vm.global("x").expect("x is defined")), "41");
    assert!(vm.global("nope").is_none());
    // Prims live in the same table (ADR-020), so they're readable too.
    assert_eq!(
        format!("{}", vm.global("+").expect("+ is a builtin")),
        "#<prim +/at least 0>"
    );
    // A closure comes back callable, which is what lets a host invoke
    // prelude entry points without evaluating source.
    let f = vm.global("f").expect("f is defined");
    assert_eq!(
        format!("{}", vm.call_value(&f, vec![Val::Num(1)]).unwrap()),
        "2"
    );
}

#[test]
fn global_sees_set_bang_updates() {
    // The mana case: a host polls a lisp-owned counter that `set!`
    // mutates. `global` must read the cell's current contents, not a
    // snapshot from definition time.
    let mut vm = Vm::new();
    vm.eval_str("(define counter 10)").unwrap();
    assert_eq!(format!("{}", vm.global("counter").unwrap()), "10");
    vm.eval_str("(set! counter (- counter 3))").unwrap();
    assert_eq!(format!("{}", vm.global("counter").unwrap()), "7");
}

// ── Source spans (ADR-039, implementing ADR-022) ──────────────────
//
// These replace `parse_errors_carry_no_source_position`, which pinned
// ADR-022 as designed-and-never-implemented. It said to delete it when
// Phase 1 landed and to update ADR-022's status banner plus
// `core-followups.md`; all three happened together.

#[test]
fn unmatched_open_paren_reports_the_paren_that_never_closed() {
    // ADR-022's own motivating example. The interesting part is *which*
    // position: the end of input is where the reader noticed, but the
    // opening paren on line 1 is the thing to go fix.
    let mut vm = Vm::new();
    let err = vm
        .eval_str("(let ((a 1)\n      (b 2)\n  (+ a b))")
        .expect_err("unbalanced parens should fail");
    assert_eq!(err.msg, "unclosed (");
    let span = err.span.expect("parse errors carry a span");
    assert_eq!((span.line, span.col), (1, 1));
    assert_eq!(err.to_string(), "1:1: unclosed (");
}

#[test]
fn reader_errors_carry_positions() {
    let mut vm = Vm::new();
    for (src, msg, line, col) in [
        // Stray close paren: at the paren itself.
        ("(+ 1 2))", "unexpected )", 1u32, 8u32),
        // Unclosed string: at the opening quote, not end of input.
        ("(+ 1\n   \"oops)", "unclosed string literal", 2, 4),
        // Bad escape: at the backslash.
        ("(+ 1 \"a\\q\")", "unknown string escape \\q", 1, 8),
        // Nested unclosed parens report the *innermost* one — that's the
        // form the reader was still filling when input ran out.
        ("(define x\n  (+ 1\n", "unclosed (", 2, 3),
        // Nothing open to blame: a dangling reader prefix points at the
        // end of input, which is all that's known.
        ("(+ 1 2)\n'", "unexpected eof", 2, 2),
    ] {
        let err = vm.eval_str(src).expect_err("should fail");
        assert_eq!(err.msg, msg, "for {src:?}");
        let span = err.span.unwrap_or_else(|| panic!("no span for {src:?}"));
        assert_eq!((span.line, span.col), (line, col), "for {src:?}");
    }
}

#[test]
fn compile_errors_report_the_innermost_form() {
    // The whole point of `LispErr::with_span` filling only empty spans:
    // the bad `if` is on line 3, and the enclosing `define` and `lambda`
    // must not overwrite that as the error propagates out to line 1.
    let mut vm = Vm::new();
    let err = vm
        .eval_str("(define f\n  (lambda (x)\n    (if x 1)))")
        .expect_err("two-armed if should fail");
    assert_eq!(err.msg, "if: expected (if cond then else)");
    let span = err.span.expect("compile errors carry a span");
    assert_eq!((span.line, span.col), (3, 5));
}

#[test]
fn unbound_variable_carries_its_own_position() {
    // ADR-022 deferred runtime spans to Phase 2 and its planned test
    // (`runtime_error_has_no_span_yet`) asserted this was *absent*.
    // ADR-039 shipped the slice of Phase 2 that covers `Var` and `App`,
    // because "unbound variable" with no position was the error that
    // actually made a 40-line prelude hard to debug.
    let mut vm = Vm::new();
    let err = vm
        .eval_str("(define n 1)\n(+ n\n   mama)")
        .expect_err("unbound variable should fail");
    assert_eq!(err.msg, "unbound variable: mama");
    let span = err.span.expect("runtime var errors carry a span");
    assert_eq!((span.line, span.col, span.len), (3, 4, 4));
}

#[test]
fn call_site_errors_carry_the_call_position() {
    let mut vm = Vm::new();

    // Arity mismatch on a closure: reported at the call, not the lambda.
    let err = vm
        .eval_str("(define f (lambda (a b) a))\n(f 1)")
        .expect_err("arity mismatch should fail");
    assert_eq!(err.msg, "arity: closure expected 2, got 1");
    assert_eq!(err.span.map(|s| (s.line, s.col)), Some((2, 1)));

    // Non-callable head.
    let err = vm
        .eval_str("(define x 5)\n(x 1)")
        .expect_err("5 isn't callable");
    assert_eq!(err.msg, "not callable: 5");
    assert_eq!(err.span.map(|s| (s.line, s.col)), Some((2, 1)));

    // A prim's own complaint. Prims return bare strings and have no
    // source of their own; `apply` attaches the call site, which is the
    // position that helps anyway.
    let err = vm
        .eval_str("(car\n  '())")
        .expect_err("car of nil should fail");
    assert_eq!(err.span.map(|s| (s.line, s.col)), Some((1, 1)));
}

#[test]
fn host_built_forms_report_without_a_position() {
    // `None` is a real answer, not a gap: pointing at a caller's line for
    // a form the caller never wrote would be worse than saying nothing.
    let mut vm = Vm::new();
    let f = vm.eval_str("(lambda (a b) a)").unwrap();
    let err = vm.call_value(&f, vec![Val::Num(1)]).expect_err("arity");
    assert_eq!(err.span, None);
    assert_eq!(err.to_string(), "arity: closure expected 2, got 1");
}

#[test]
fn render_underlines_the_offending_source() {
    let mut vm = Vm::new();
    let src = "(define n 1)\n(+ n mama)";
    let err = vm.eval_str(src).expect_err("unbound");
    assert_eq!(
        err.render(src),
        "2:6: unbound variable: mama\n  |\n2 | (+ n mama)\n  |      ^^^^"
    );
}

#[test]
fn render_falls_back_when_there_is_nothing_to_point_at() {
    // No span, or a span naming a line this source doesn't have: fall
    // back to plain Display rather than panicking or drawing a caret
    // under the wrong text.
    let plain = LispErr::new("boom");
    assert_eq!(plain.render("(+ 1 2)"), "boom");
    let far = LispErr::at("boom", Span::new(99, 1, 1));
    assert_eq!(far.render("(+ 1 2)"), "99:1: boom");
}

#[test]
fn columns_count_characters_not_bytes() {
    // The rune and trigram tapes this project reads are multi-byte, so a
    // byte column would put the caret in the wrong place — and every
    // host renders line:col directly.
    let mut vm = Vm::new();
    let src = "(list '☰ '☱ nope)";
    let err = vm.eval_str(src).expect_err("unbound");
    assert_eq!(err.msg, "unbound variable: nope");
    // `nope` is the 13th character but the 19th byte.
    assert_eq!(err.span.map(|s| s.col), Some(13));
    let rendered = err.render(src);
    let caret_line = rendered.lines().last().unwrap();
    assert_eq!(
        caret_line,
        format!("  | {}^^^^", " ".repeat(12)),
        "caret misaligned in:\n{rendered}"
    );
}
