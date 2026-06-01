use lisp::Vm;

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
        matches!(&r, Err(e) if e.contains("numeric overflow")),
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
        matches!(&r, Err(e) if e.contains("step budget")),
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

#[test]
fn macro_expanding_to_ratio_works() {
    // Before: val_to_datum errored on Val::Ratio with "can't convert 1/2
    // back to a datum" when a macro body produced a literal rational.
    let mut vm = Vm::new();
    vm.eval_str("(defmacro half () `1/2)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(half)").unwrap()), "1/2");
}

#[test]
fn macro_when_via_quote() {
    let mut vm = Vm::new();
    vm.eval_str("(defmacro when args (list 'if (car args) (car (cdr args)) #f))")
        .unwrap();
    assert_eq!(format!("{}", vm.eval_str("(when #t 42)").unwrap()), "42");
    assert_eq!(format!("{}", vm.eval_str("(when #f 42)").unwrap()), "#f");
}

#[test]
fn macro_unless_via_quasiquote() {
    let mut vm = Vm::new();
    vm.eval_str("(defmacro unless (c body) `(if ,c #f ,body))")
        .unwrap();
    assert_eq!(
        format!("{}", vm.eval_str("(unless #f 'gotcha)").unwrap()),
        "gotcha"
    );
    assert_eq!(
        format!("{}", vm.eval_str("(unless #t 'nope)").unwrap()),
        "#f"
    );
}

#[test]
fn macro_thread_first() {
    let mut vm = Vm::new();
    // `->` thread-first: (-> x (f a) (g b)) → (g (f x a) b)
    vm.eval_str(
        r#"
        (defmacro -> args
          (letrec ((step (lambda (acc form)
                           (if (pair? form)
                               (cons (car form) (cons acc (cdr form)))
                               (list form acc))))
                   (loop (lambda (acc fs)
                           (if (null? fs) acc
                               (loop (step acc (car fs)) (cdr fs))))))
            (loop (car args) (cdr args))))
    "#,
    )
    .unwrap();
    // (-> 5 (+ 3) (* 2))  →  (* (+ 5 3) 2)  →  16
    assert_eq!(
        format!("{}", vm.eval_str("(-> 5 (+ 3) (* 2))").unwrap()),
        "16"
    );
    // Bare symbol form: (-> x f) → (f x)
    vm.eval_str("(defmacro inc (n) `(+ ,n 1))").unwrap();
    assert_eq!(
        format!("{}", vm.eval_str("(-> 10 inc inc inc)").unwrap()),
        "13"
    );
}

#[test]
fn macro_splicing() {
    let mut vm = Vm::new();
    vm.eval_str("(defmacro listof args `(list ,@args))")
        .unwrap();
    assert_eq!(
        format!("{}", vm.eval_str("(listof 1 2 3)").unwrap()),
        "(1 2 3)"
    );
}

#[test]
fn macro_calls_macro() {
    // A macro body can use other macros.
    let mut vm = Vm::new();
    vm.eval_str("(defmacro twice (e) `(begin-list (list ,e ,e)))")
        .unwrap();
    vm.eval_str("(defmacro begin-list (xs) `(car (cdr ,xs)))")
        .unwrap();
    // (twice 7) → (begin-list (list 7 7)) → (car (cdr (list 7 7))) → 7
    assert_eq!(format!("{}", vm.eval_str("(twice 7)").unwrap()), "7");
}

#[test]
fn macro_defmacro_only_top_level() {
    let mut vm = Vm::new();
    let r = vm.eval_str("(let ((x 1)) (defmacro foo args 'bar))");
    assert!(r.is_err(), "nested defmacro should error: {r:?}");
}

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
    use std::rc::Rc;
    let mut vm = lisp::Vm::new();
    spells::install(&mut vm);
    // Grab a weak handle to one of the installed closure cells.
    let weak = {
        let table = vm.globals.borrow();
        let cell = table.get("fire").expect("spells prelude defines fire");
        Rc::downgrade(cell)
    };
    assert!(
        weak.upgrade().is_some(),
        "sanity: cell is live while Vm is alive"
    );
    drop(vm);
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
        r.as_ref().is_err_and(|e| e.starts_with("not callable")),
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
fn letrec_cycle_persists_after_drop() {
    // ADR-021 diagnostic: documents the residual letrec Rc cycle.
    // A letrec closure that references its own name forms
    // `Frame → cell → Val::Clo → closure.env → Frame`. After every
    // user-side strong handle drops, the cycle keeps the cell alive
    // (leak). When the engine grows a real fix — closure conversion
    // or Y-style desugaring or a Store-reified CESK upgrade — this
    // assertion flips, and that flip is the regression signal.
    let mut vm = lisp::Vm::new();
    let v = vm
        .eval_str("(letrec ((f (lambda () (f)))) f)")
        .unwrap();
    // Reach into the returned closure's captured env for the "f" slot.
    let weak = match &v {
        lisp::Val::Clo { env, .. } => env
            .weak_slot("f")
            .expect("letrec frame should carry an `f` slot"),
        other => panic!("expected letrec body to return a closure, got {other}"),
    };
    drop(v);
    drop(vm);
    assert!(
        weak.upgrade().is_some(),
        "today: the cell.value → closure → env.frame → cell cycle \
         keeps the slot alive even after every external strong \
         handle drops. When this flips to is_none() the cycle has \
         been broken — update or remove this assertion at that point."
    );
}
