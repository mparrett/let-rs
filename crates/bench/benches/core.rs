//! Core lisp engine benchmarks. No DSL vocabulary — these exercise
//! the CEK machine, parser, and pure prims directly so a regression
//! can be traced to an engine change rather than a demo prelude.
//!
//! Run with `just bench`. Criterion saves baselines under
//! `target/criterion/` (gitignored); use `cargo bench -- --save-baseline X`
//! to lock a baseline before a refactor and compare after.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lisp::Vm;

// ─── engine: tail calls, recursion, env chain ───────────────────

fn bench_tail_call_loop(c: &mut Criterion) {
    // The canonical "doesn't grow the stack" loop, sized to a number
    // big enough to dominate setup cost. Times one full descent.
    let src = "(letrec ((loop (lambda (n) (if (= n 0) 0 (loop (- n 1)))))) \
                 (loop 10000))";
    c.bench_function("tail_call_loop_10k", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(src)).unwrap())
        })
    });
}

fn bench_letrec_mutual(c: &mut Criterion) {
    // even?/odd? at N=500. Stresses letrec's placeholder-cell mechanism
    // and cross-binding lookups.
    let src = "(letrec ((even? (lambda (n) (if (= n 0) #t (odd?  (- n 1))))) \
                        (odd?  (lambda (n) (if (= n 0) #f (even? (- n 1)))))) \
                 (even? 500))";
    c.bench_function("letrec_mutual_500", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(src)).unwrap())
        })
    });
}

fn bench_env_deep_lookup(c: &mut Criterion) {
    // 30 nested `let` bindings, look up the outermost var. Worst-case
    // env-chain traversal — every Var lookup walks the full chain.
    let mut src = String::new();
    for i in 0..30 {
        src.push_str(&format!("(let ((x{i} {i})) "));
    }
    src.push_str("x0");
    for _ in 0..30 {
        src.push(')');
    }
    c.bench_function("env_deep_lookup_30", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(&src)).unwrap())
        })
    });
}

// ─── data structures: list + alist ──────────────────────────────

fn bench_list_map_1000(c: &mut Criterion) {
    // Square 1000 numbers via a user-defined `map`. Stresses closure
    // application + cons allocation under the CEK machine.
    let nums: String = (1..=1000)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!(
        "(letrec ((map (lambda (f xs) \
                         (if (null? xs) '() \
                             (cons (f (car xs)) (map f (cdr xs))))))) \
           (map (lambda (n) (* n n)) (list {nums})))"
    );
    c.bench_function("list_map_1000", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(&src)).unwrap())
        })
    });
}

fn bench_assoc_get_at_depth(c: &mut Criterion) {
    // 50-entry alist, look up the last key. Stresses the new
    // assoc-get prim's loop + cons traversal.
    let mut alist = String::from("(list ");
    for i in 0..50 {
        alist.push_str(&format!("(cons 'k{i} {i}) "));
    }
    alist.push(')');
    let src = format!("(assoc-get 'k49 {alist})");
    c.bench_function("assoc_get_at_50", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(&src)).unwrap())
        })
    });
}

// ─── numeric tower ──────────────────────────────────────────────

fn bench_arith_int_fold(c: &mut Criterion) {
    // `(+ 1 2 3 ... 100)`. All-integer path through the numeric
    // tower — should stay in the `(n, 1)` ratio form throughout.
    let nums: String = (1..=100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("(+ {nums})");
    c.bench_function("arith_int_fold_100", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(&src)).unwrap())
        })
    });
}

fn bench_arith_ratio_fold(c: &mut Criterion) {
    // `(+ 1/2 1/3 1/4 ... 1/100)`. Exercises the i128 promote +
    // gcd normalize path on every step. The denominator grows fast
    // here, so this is a more interesting stress than int-fold.
    let parts: String = (2..=100)
        .map(|d| format!("1/{d}"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("(+ {parts})");
    c.bench_function("arith_ratio_fold_99", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(&src)).unwrap())
        })
    });
}

// ─── pipeline: parser ──────────────────────────────────────────

fn bench_parser_only(c: &mut Criterion) {
    // ~50-line source through read_many + macro expansion + compile,
    // without running the result. Separates "parse got slower" from
    // "eval got slower" in regression hunting. We force the parse-and-
    // compile work by calling eval_str on something whose evaluation is
    // trivial (a number literal at the end), with the parsing bulk
    // being all the defines.
    let mut src = String::new();
    for i in 0..40 {
        src.push_str(&format!("(define f{i} (lambda (x) (+ x {i})))\n"));
    }
    src.push_str("(f0 0)\n");
    c.bench_function("parser_define_chain_40", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(&src)).unwrap())
        })
    });
}

// ─── macros ─────────────────────────────────────────────────────

fn bench_macro_thread_first(c: &mut Criterion) {
    // The classic `->` (thread-first) macro applied to a chain.
    // Stresses the expand_all + expand_macro_call loop, plus the
    // macro body itself running through the interpreter on every
    // expansion.
    let src = "\
        (defmacro -> args \
          (letrec ((step (lambda (acc form) \
                           (if (pair? form) \
                               (cons (car form) (cons acc (cdr form))) \
                               (list form acc)))) \
                   (loop (lambda (acc fs) \
                           (if (null? fs) acc \
                               (loop (step acc (car fs)) (cdr fs)))))) \
            (loop (car args) (cdr args))))\n\
        (define inc (lambda (x) (+ x 1)))\n\
        (define dbl (lambda (x) (* x 2)))\n\
        (-> 1 inc dbl inc dbl inc dbl inc dbl)";
    c.bench_function("macro_thread_chain_8", |b| {
        b.iter(|| {
            let mut vm = Vm::new();
            black_box(vm.eval_str(black_box(src)).unwrap())
        })
    });
}

criterion_group!(
    benches,
    bench_tail_call_loop,
    bench_letrec_mutual,
    bench_env_deep_lookup,
    bench_list_map_1000,
    bench_assoc_get_at_depth,
    bench_arith_int_fold,
    bench_arith_ratio_fold,
    bench_parser_only,
    bench_macro_thread_first,
);
criterion_main!(benches);
