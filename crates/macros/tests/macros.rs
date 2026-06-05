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
    assert_eq!(
        format!("{}", vm.eval_str("(begin (+ 1 2))").unwrap()),
        "3"
    );
}

#[test]
fn stdlib_begin_evaluates_in_order() {
    // Side-effecting evaluation order: register a counter prim, use it
    // inside (begin a b c), and check that a runs before b before c.
    let mut vm = MacroVm::with_stdlib();
    let order = std::rc::Rc::new(std::cell::RefCell::new(Vec::<i64>::new()));
    let order_clone = order.clone();
    vm.vm.register_prim("note!", lisp::val::Arity::Exact(1), move |args| {
        if let lisp::Val::Num(n) = &args[0] {
            order_clone.borrow_mut().push(*n);
            Ok(lisp::Val::Num(*n))
        } else {
            Err("note!: expected num".into())
        }
    });
    vm.eval_str("(begin (note! 1) (note! 2) (note! 3))").unwrap();
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
