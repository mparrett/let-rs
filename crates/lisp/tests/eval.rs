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
    assert_eq!(
        eval("(((lambda (x) (lambda (y) (+ x y))) 3) 4)"),
        "7"
    );
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
    assert_eq!(eval("(let* ((x 1) (y (+ x 1)) (z (+ y 1))) (+ x y z))"), "6");
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
fn tail_calls_dont_grow_the_stack() {
    let src = r#"
        (letrec ((loop (lambda (n)
                         (if (= n 0) 0 (loop (- n 1))))))
          (loop 100000))
    "#;
    assert_eq!(eval(src), "0");
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
    assert_eq!(eval("(let ((xs '(a b c))) `(begin ,@xs done))"), "(begin a b c done)");
    assert_eq!(eval("(let ((xs '())) `(x ,@xs y))"), "(x y)");
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
    vm.eval_str("(defmacro unless (c body) `(if ,c #f ,body))").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(unless #f 'gotcha)").unwrap()), "gotcha");
    assert_eq!(format!("{}", vm.eval_str("(unless #t 'nope)").unwrap()), "#f");
}

#[test]
fn macro_thread_first() {
    let mut vm = Vm::new();
    // `->` thread-first: (-> x (f a) (g b)) → (g (f x a) b)
    vm.eval_str(r#"
        (defmacro -> args
          (letrec ((step (lambda (acc form)
                           (if (pair? form)
                               (cons (car form) (cons acc (cdr form)))
                               (list form acc))))
                   (loop (lambda (acc fs)
                           (if (null? fs) acc
                               (loop (step acc (car fs)) (cdr fs))))))
            (loop (car args) (cdr args))))
    "#).unwrap();
    // (-> 5 (+ 3) (* 2))  →  (* (+ 5 3) 2)  →  16
    assert_eq!(format!("{}", vm.eval_str("(-> 5 (+ 3) (* 2))").unwrap()), "16");
    // Bare symbol form: (-> x f) → (f x)
    vm.eval_str("(defmacro inc (n) `(+ ,n 1))").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(-> 10 inc inc inc)").unwrap()), "13");
}

#[test]
fn macro_splicing() {
    let mut vm = Vm::new();
    vm.eval_str("(defmacro listof args `(list ,@args))").unwrap();
    assert_eq!(format!("{}", vm.eval_str("(listof 1 2 3)").unwrap()), "(1 2 3)");
}

#[test]
fn macro_calls_macro() {
    // A macro body can use other macros.
    let mut vm = Vm::new();
    vm.eval_str("(defmacro twice (e) `(begin-list (list ,e ,e)))").unwrap();
    vm.eval_str("(defmacro begin-list (xs) `(car (cdr ,xs)))").unwrap();
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
    // Sanity: Vm carries env across eval_str calls (currently the initial env
    // never changes, but this protects against a future `define`).
    assert_eq!(
        evals(&["(+ 1 2)", "(* 4 5)", "((lambda (x) x) 'hi)"]),
        "hi"
    );
}
