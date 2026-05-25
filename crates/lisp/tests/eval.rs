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
fn multiple_top_level_evals_share_state() {
    // Sanity: Vm carries env across eval_str calls (currently the initial env
    // never changes, but this protects against a future `define`).
    assert_eq!(
        evals(&["(+ 1 2)", "(* 4 5)", "((lambda (x) x) 'hi)"]),
        "hi"
    );
}
