use macros::MacroVm;

fn mv() -> MacroVm {
    MacroVm::new()
}

#[test]
fn macro_expanding_to_ratio_works() {
    // Pre-extraction this was a regression test for val_to_datum
    // returning "can't convert 1/2 back to a datum" when a macro body
    // produced a literal rational. val_to_datum lives in the macros
    // crate now; this test continues to pin the same case.
    let mut vm = mv();
    vm.eval_str("(defmacro half () `1/2)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(half)").unwrap()), "1/2");
}

#[test]
fn macro_when_via_quote() {
    let mut vm = mv();
    vm.eval_str("(defmacro when args (list 'if (car args) (car (cdr args)) #f))")
        .unwrap();
    assert_eq!(format!("{}", vm.eval_str("(when #t 42)").unwrap()), "42");
    assert_eq!(format!("{}", vm.eval_str("(when #f 42)").unwrap()), "#f");
}

#[test]
fn macro_unless_via_quasiquote() {
    let mut vm = mv();
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
    let mut vm = mv();
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
    let mut vm = mv();
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
    let mut vm = mv();
    vm.eval_str("(defmacro twice (e) `(begin-list (list ,e ,e)))")
        .unwrap();
    vm.eval_str("(defmacro begin-list (xs) `(car (cdr ,xs)))")
        .unwrap();
    // (twice 7) → (begin-list (list 7 7)) → (car (cdr (list 7 7))) → 7
    assert_eq!(format!("{}", vm.eval_str("(twice 7)").unwrap()), "7");
}

#[test]
fn top_level_define_allowed_through_macro_vm() {
    // Pre-ADR-025 this failed: the expander rejected `define` at any
    // list head, including top level. The fix adds `expand_top_level`
    // which keeps the define form intact while still expanding its
    // body. Tests this stays fixed.
    let mut vm = mv();
    vm.eval_str("(define x 7)").expect("top-level define");
    assert_eq!(format!("{}", vm.eval_str("x").unwrap()), "7");
}

#[test]
fn macro_expanding_to_define_works_at_top_level() {
    // The defspell-style case: a macro whose expansion is itself a
    // `(define …)` form. Before the fix, the recursive expand_all
    // after macro expansion rejected the define. After the fix, the
    // expansion goes through `expand_top_level` and the define is
    // honored.
    let mut vm = mv();
    vm.eval_str("(defmacro defconst (name val) `(define ,name ,val))")
        .unwrap();
    vm.eval_str("(defconst answer 42)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("answer").unwrap()), "42");
}

#[test]
fn set_bang_name_position_not_macro_expanded() {
    // (set! NAME val) — NAME is a binding reference, not an
    // expression. If the expander treated it as one and the name
    // happened to match a macro, we'd silently rewrite the binding
    // reference. Pin the rule: items[1] of set! is left alone, but
    // items[2] (the value expr) is expanded normally.
    let mut vm = mv();
    // Register a macro named `target` that, if it ever fires in the
    // set! name slot, would expand to a non-symbol form and blow up.
    vm.eval_str("(defmacro target () 'expanded)").unwrap();
    // (set! target …) at parse time: name is the literal symbol
    // `target`. The expander must not treat it as a macro call.
    // We won't actually run set! on it (target isn't defined as a
    // variable); just confirm expansion doesn't error before parse.
    vm.eval_str("(define target 7)").unwrap();
    vm.eval_str("(set! target (+ target 1))").unwrap();
    assert_eq!(format!("{}", vm.eval_str("target").unwrap()), "8");
}

#[test]
fn nested_define_still_rejected() {
    // The fix is surgical: top-level define is OK, but nested define
    // inside expressions (let body, lambda body, etc.) still errors.
    let mut vm = mv();
    let r = vm.eval_str("(let ((x 1)) (define y 2))");
    assert!(r.is_err(), "nested define should error: {r:?}");
}

#[test]
fn macro_defmacro_only_top_level() {
    let mut vm = mv();
    let r = vm.eval_str("(let ((x 1)) (defmacro foo args 'bar))");
    assert!(r.is_err(), "nested defmacro should error: {r:?}");
}

#[test]
fn defmacro_unknown_to_raw_vm() {
    // Sanity: a raw lisp::Vm (without MacroVm wrapping) does NOT know
    // about defmacro. This pins the ADR-024 extraction — the engine
    // is macro-unaware; macros are an opt-in host layer.
    let mut vm = lisp::Vm::new();
    let r = vm.eval_str("(defmacro foo () 1)");
    assert!(
        r.is_err(),
        "lisp::Vm should reject defmacro forms post-ADR-024: {r:?}"
    );
}

#[test]
fn stdlib_begin_returns_last_value() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(begin 1 2 3)").unwrap()), "3");
    assert_eq!(
        format!("{}", vm.eval_str("(begin (+ 1 1) (* 2 3) 'final)").unwrap()),
        "final"
    );
}

#[test]
fn stdlib_begin_single_arg_passes_through() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(begin 42)").unwrap()), "42");
    assert_eq!(format!("{}", vm.eval_str("(begin (+ 1 2))").unwrap()), "3");
}

#[test]
fn stdlib_begin_evaluates_in_order() {
    // Side-effecting evaluation order: register a counter prim, use it
    // inside (begin a b c), and check that a runs before b before c.
    let mut vm = MacroVm::with_stdlib();
    let order = std::rc::Rc::new(std::cell::RefCell::new(Vec::<i64>::new()));
    let order_clone = order.clone();
    vm.vm
        .register_prim("note!", lisp::val::Arity::Exact(1), move |args| {
            if let lisp::Val::Num(n) = &args[0] {
                order_clone.borrow_mut().push(*n);
                Ok(lisp::Val::Num(*n))
            } else {
                Err("note!: expected num".into())
            }
        });
    vm.eval_str("(begin (note! 1) (note! 2) (note! 3))")
        .unwrap();
    assert_eq!(*order.borrow(), vec![1, 2, 3]);
}

#[test]
fn stdlib_install_idempotent() {
    // Calling install_stdlib twice shouldn't break — the second
    // defmacro just overwrites the first identical registration.
    let mut vm = MacroVm::new();
    macros::install_stdlib(&mut vm).unwrap();
    macros::install_stdlib(&mut vm).unwrap();
    assert_eq!(format!("{}", vm.eval_str("(begin 1 2)").unwrap()), "2");
}

#[test]
fn stdlib_not_present_without_install() {
    // A plain MacroVm::new() does NOT have begin — it's opt-in.
    let mut vm = MacroVm::new();
    let r = vm.eval_str("(begin 1 2)");
    assert!(
        r.is_err(),
        "MacroVm::new should not have stdlib pre-installed: {r:?}"
    );
}

// ── when / unless ─────────────────────────────────────────────────

#[test]
fn stdlib_when_truthy_runs_body() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(when #t 42)").unwrap()), "42");
    assert_eq!(
        format!("{}", vm.eval_str("(when (= 1 1) 'yes)").unwrap()),
        "yes"
    );
}

#[test]
fn stdlib_when_falsy_returns_false() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(when #f 42)").unwrap()), "#f");
}

#[test]
fn stdlib_when_multi_body_sequences_via_begin() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(when #t 1 2 3)").unwrap()), "3");
}

#[test]
fn stdlib_unless_inverts_when() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(unless #f 42)").unwrap()), "42");
    assert_eq!(format!("{}", vm.eval_str("(unless #t 42)").unwrap()), "#f");
    assert_eq!(
        format!("{}", vm.eval_str("(unless #f 1 2 3)").unwrap()),
        "3"
    );
}

// ── and / or ──────────────────────────────────────────────────────

#[test]
fn stdlib_and_empty_is_true() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(and)").unwrap()), "#t");
}

#[test]
fn stdlib_and_single_arg_returns_value() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(and 5)").unwrap()), "5");
    assert_eq!(format!("{}", vm.eval_str("(and #f)").unwrap()), "#f");
}

#[test]
fn stdlib_and_returns_last_truthy_or_false() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(and 1 2 3)").unwrap()), "3");
    assert_eq!(format!("{}", vm.eval_str("(and 1 #f 3)").unwrap()), "#f");
    assert_eq!(
        format!("{}", vm.eval_str("(and 1 2 'last)").unwrap()),
        "last"
    );
}

#[test]
fn stdlib_and_short_circuits() {
    let mut vm = MacroVm::with_stdlib();
    let count = std::rc::Rc::new(std::cell::RefCell::new(0i64));
    let count_clone = count.clone();
    vm.vm
        .register_prim("bump!", lisp::val::Arity::Exact(0), move |_| {
            *count_clone.borrow_mut() += 1;
            Ok(lisp::Val::Num(*count_clone.borrow()))
        });
    // The second arg is #f, so the third (bump!) must NOT run.
    vm.eval_str("(and 1 #f (bump!))").unwrap();
    assert_eq!(*count.borrow(), 0, "and should short-circuit on first #f");
}

#[test]
fn stdlib_or_empty_is_false() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(or)").unwrap()), "#f");
}

#[test]
fn stdlib_or_single_arg_returns_value() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(or 5)").unwrap()), "5");
    assert_eq!(format!("{}", vm.eval_str("(or #f)").unwrap()), "#f");
}

#[test]
fn stdlib_or_returns_first_truthy() {
    let mut vm = MacroVm::with_stdlib();
    assert_eq!(format!("{}", vm.eval_str("(or #f #f 3)").unwrap()), "3");
    assert_eq!(format!("{}", vm.eval_str("(or 1 2 3)").unwrap()), "1");
    assert_eq!(format!("{}", vm.eval_str("(or #f #f #f)").unwrap()), "#f");
}

#[test]
fn stdlib_or_does_not_double_evaluate_args() {
    // The `or` macro binds each arg to a temp before testing it, so
    // side-effecting args run exactly once even when truthy.
    let mut vm = MacroVm::with_stdlib();
    let count = std::rc::Rc::new(std::cell::RefCell::new(0i64));
    let count_clone = count.clone();
    vm.vm
        .register_prim("bump!", lisp::val::Arity::Exact(0), move |_| {
            *count_clone.borrow_mut() += 1;
            Ok(lisp::Val::Num(*count_clone.borrow()))
        });
    // First call returns 1 (truthy) and `or` should NOT re-run it.
    vm.eval_str("(or (bump!) (bump!))").unwrap();
    assert_eq!(
        *count.borrow(),
        1,
        "or must evaluate its first arg exactly once"
    );
}

#[test]
fn stdlib_or_short_circuits() {
    let mut vm = MacroVm::with_stdlib();
    let count = std::rc::Rc::new(std::cell::RefCell::new(0i64));
    let count_clone = count.clone();
    vm.vm
        .register_prim("bump!", lisp::val::Arity::Exact(0), move |_| {
            *count_clone.borrow_mut() += 1;
            Ok(lisp::Val::Num(*count_clone.borrow()))
        });
    // First arg is truthy, so (bump!) in tail must NOT run.
    vm.eval_str("(or 1 (bump!))").unwrap();
    assert_eq!(
        *count.borrow(),
        0,
        "or should short-circuit on first truthy"
    );
}

#[test]
fn macro_expanding_to_string_literal() {
    // Mirrors `macro_expanding_to_ratio_works` — pins val_to_datum's
    // Val::Str arm so a macro body that returns a string literal
    // round-trips through expansion back into the source.
    let mut vm = mv();
    vm.eval_str("(defmacro greeting () `\"hello\")").unwrap();
    assert_eq!(
        format!("{}", vm.eval_str("(greeting)").unwrap()),
        "\"hello\""
    );
}

#[test]
fn quasiquote_splices_string_literal() {
    let mut vm = mv();
    let out = vm
        .eval_str("(let ((name \"world\")) `(\"hi\" ,name))")
        .unwrap();
    assert_eq!(format!("{out}"), "(\"hi\" \"world\")");
}

#[test]
fn self_referential_macro_errors_instead_of_overflowing() {
    // A macro that expands to a call to itself re-expands forever as native
    // recursion (the Vm step budget only bounds evaluation). The expansion
    // depth cap must convert that into a clean error, not a stack overflow.
    let mut vm = mv();
    vm.eval_str("(defmacro foo (x) `(foo ,x))").unwrap();
    let r = vm.eval_str("(foo 1)");
    assert!(
        matches!(&r, Err(e) if e.contains("expansion too deep")),
        "expected expansion-depth error, got {r:?}"
    );
    // A well-behaved macro alongside it still expands normally.
    vm.eval_str("(defmacro twice (x) `(+ ,x ,x))").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(twice 21)").unwrap()), "42");
}

/// A macro that returns `Val::Nil` renders as `()` on the way back
/// through source text, which is invalid in expression position. There
/// is no context at serialization time to tell an evaluated `()` from
/// a `(lambda () …)` binder, so the macro has to quote it. Lock in both
/// halves: the error names the fix, and the quoted form works.
#[test]
fn macro_returning_nil_errors_with_a_pointer_to_the_workaround() {
    let mut vm = MacroVm::with_stdlib();
    let err = vm
        .eval_str("(defmacro nada () '()) (nada)")
        .expect_err("a macro expanding to () should error");
    assert!(err.contains("'()"), "message should name the fix: {err}");
}

#[test]
fn macro_can_emit_the_empty_list_via_quote() {
    let mut vm = MacroVm::with_stdlib();
    let v = vm
        .eval_str("(defmacro nada () '(quote ())) (cons 1 (nada))")
        .expect("quoted nil should expand cleanly");
    assert_eq!(format!("{v}"), "(1)");
}

/// Empty lists in *binder* position must keep working — they never
/// reach `compile`, so the sharpened error must not have caught them.
#[test]
fn empty_binder_lists_in_macro_output_still_work() {
    let mut vm = MacroVm::with_stdlib();
    let thunk = vm
        .eval_str("(defmacro thunk (b) `(lambda () ,b)) ((thunk 42))")
        .expect("empty lambda params should survive the round trip");
    assert_eq!(format!("{thunk}"), "42");
    let nolet = vm
        .eval_str("(defmacro nolet (b) `(let () ,b)) (nolet 7)")
        .expect("empty let bindings should survive the round trip");
    assert_eq!(format!("{nolet}"), "7");
}

/// And `()` nested inside quoted *data* stays data — the sharpened
/// error is an expression-position rule, not a reader rule.
#[test]
fn empty_list_inside_quoted_data_is_untouched() {
    let mut vm = MacroVm::with_stdlib();
    let v = vm
        .eval_str("(defmacro d () `(quote (a ()))) (d)")
        .expect("quoted data containing () should expand cleanly");
    assert_eq!(format!("{v}"), "(a ())");
}

/// ADR-034 regression. Expanded datums used to reach the engine by
/// being re-serialized to source text and read a second time, which
/// made expansion only as faithful as the printer. `string->symbol`
/// can build a symbol the reader cannot round-trip: printing
/// `(quote |a b|)` emits `(quote a b)`, which re-reads as a two-argument
/// quote and dies with "quote: expected (quote datum)".
///
/// Handing datums to `Vm::eval_datums` directly removes the printer
/// from the path, so the symbol survives intact.
#[test]
fn macro_output_survives_symbols_the_printer_cannot_round_trip() {
    let mut vm = MacroVm::with_stdlib();
    vm.eval_str("(defmacro weird () (list 'quote (string->symbol \"a b\")))")
        .expect("defmacro should register");
    let v = vm
        .eval_str("(weird)")
        .expect("a symbol containing a space must survive expansion");
    assert_eq!(format!("{v}"), "a b");
}

/// The same property one level up: a macro that emits a *list* built
/// from such symbols. Nothing here is exotic — it's the shape any
/// `defsomething` macro takes when it derives names from strings.
///
/// The expected `(x y x y)` is two symbols, each `x y`. That the
/// printed form is ambiguous is the whole point: a printer that can't
/// distinguish them can't be the transport between expander and engine.
#[test]
fn derived_symbol_names_survive_expansion() {
    let mut vm = MacroVm::with_stdlib();
    let v = vm
        .eval_str(
            "(defmacro pair-of (s) (list 'quote (list (string->symbol s) (string->symbol s))))
             (pair-of \"x y\")",
        )
        .expect("derived names must survive expansion");
    assert_eq!(format!("{v}"), "(x y x y)");
}
