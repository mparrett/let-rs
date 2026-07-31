//! Namespaces (ADR-042): per-pack binding tables chained to a root.

use lisp::{Val, Vm};

#[test]
fn two_packs_can_hold_different_definitions_of_one_name() {
    // The thing the flat table made impossible. No renaming, no
    // prefixes: both packs define `thread` and both are right.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    let b = vm.namespace("packB");
    vm.eval_str_in(&a, "(define thread (lambda (x) (list 'A x)))")
        .unwrap();
    vm.eval_str_in(&b, "(define thread (lambda (x) (list 'B x)))")
        .unwrap();

    assert_eq!(
        format!("{}", vm.eval_str_in(&a, "(thread 1)").unwrap()),
        "(A 1)"
    );
    assert_eq!(
        format!("{}", vm.eval_str_in(&b, "(thread 1)").unwrap()),
        "(B 1)"
    );
    // And root, which defines neither, sees neither.
    assert!(vm.global("thread").is_none());
}

#[test]
fn resolution_is_lexical_not_ambient() {
    // The property that makes this worth having. A pack's closure
    // resolves its own helpers no matter who calls it, because lookup
    // follows the closure's captured env rather than anything current at
    // the moment of the call. Under the old flat table, and under any
    // dynamic scheme, `entry` called from elsewhere would pick up the
    // caller's `helper`.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    let b = vm.namespace("packB");
    vm.eval_str_in(
        &a,
        "(define helper (lambda () 'a-helper))
                        (define entry (lambda () (helper)))",
    )
    .unwrap();
    vm.eval_str_in(&b, "(define helper (lambda () 'b-helper))")
        .unwrap();
    vm.export(&a, &["entry"]).unwrap();

    // Called from root, from inside B, and through a B closure.
    assert_eq!(format!("{}", vm.eval_str("(entry)").unwrap()), "a-helper");
    assert_eq!(
        format!("{}", vm.eval_str_in(&b, "(entry)").unwrap()),
        "a-helper"
    );
    vm.eval_str_in(&b, "(define call-it (lambda () (entry)))")
        .unwrap();
    assert_eq!(
        format!("{}", vm.eval_str_in(&b, "(call-it)").unwrap()),
        "a-helper"
    );
}

#[test]
fn recursion_stays_inside_its_own_pack() {
    // Sharper than the above, and the reason discipline at the call site
    // could not have contained the old behavior: a global-resolving
    // closure re-resolves its *own name* on each recursive call
    // (ADR-015). With one table, redefining `walk` anywhere hijacked an
    // in-flight recursion.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    vm.eval_str_in(
        &a,
        "(define walk (lambda (n acc) (if (= n 0) acc (walk (- n 1) (+ acc 1)))))",
    )
    .unwrap();
    vm.export(&a, &["walk"]).unwrap();
    // Root binds its own `walk` to something hostile.
    vm.eval_str("(define walk (lambda (n acc) 'hijacked))")
        .unwrap();
    // A's recursion is unaffected: its own binding still wins inside A.
    assert_eq!(
        format!("{}", vm.eval_str_in(&a, "(walk 5 0)").unwrap()),
        "5"
    );
}

#[test]
fn packs_see_builtins_through_the_root() {
    // Chaining to the root is what keeps a pack from having to
    // re-register the standard vocabulary.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    assert_eq!(format!("{}", vm.eval_str_in(&a, "(+ 1 2)").unwrap()), "3");
    assert_eq!(
        format!("{}", vm.eval_str_in(&a, "(car (list 'x 'y))").unwrap()),
        "x"
    );
}

#[test]
fn a_pack_may_shadow_a_builtin_without_disturbing_anyone() {
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    vm.eval_str_in(&a, "(define car (lambda (x) 'pack-car))")
        .unwrap();
    assert_eq!(
        format!("{}", vm.eval_str_in(&a, "(car '(1 2))").unwrap()),
        "pack-car"
    );
    // Root's `car` is untouched.
    assert_eq!(format!("{}", vm.eval_str("(car '(1 2))").unwrap()), "1");
}

#[test]
fn exporting_the_same_name_twice_is_an_error_that_names_both_packs() {
    // The diagnostic whose absence was the actual hazard. Silence is
    // what made the flat table dangerous, not shadowing as such.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    let b = vm.namespace("packB");
    vm.eval_str_in(&a, "(define entry 1)").unwrap();
    vm.eval_str_in(&b, "(define entry 2)").unwrap();
    vm.export(&a, &["entry"]).unwrap();
    let err = vm
        .export(&b, &["entry"])
        .expect_err("second export must fail");
    assert!(err.msg.contains("already bound"), "{err}");
    assert!(err.msg.contains("packB"), "should name the offender: {err}");
    // And the first pack's export still stands.
    assert_eq!(format!("{}", vm.eval_str("entry").unwrap()), "1");
}

#[test]
fn re_exporting_the_same_cell_is_a_no_op() {
    // Installing a pack twice must not be an error, or idempotent host
    // setup becomes a trap.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    vm.eval_str_in(&a, "(define entry 1)").unwrap();
    vm.export(&a, &["entry"]).unwrap();
    vm.export(&a, &["entry"])
        .expect("re-export of the same cell is fine");
}

#[test]
fn exporting_shares_the_cell_so_set_is_visible_both_ways() {
    // What lets a pack own mutable state that a host still reads from
    // root: the export aliases the cell rather than copying the value.
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    vm.eval_str_in(&a, "(define counter 10)").unwrap();
    vm.export(&a, &["counter"]).unwrap();

    vm.eval_str_in(&a, "(set! counter 7)").unwrap();
    assert!(matches!(vm.global("counter"), Some(Val::Num(7))));
    vm.eval_str("(set! counter 3)").unwrap();
    assert!(matches!(vm.global_in(&a, "counter"), Some(Val::Num(3))));
}

#[test]
fn exporting_a_name_the_pack_does_not_define_is_an_error() {
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    let err = vm
        .export(&a, &["nope"])
        .expect_err("cannot export what isn't there");
    assert!(err.msg.contains("does not define"), "{err}");
}

#[test]
fn namespaces_are_reachable_by_name_and_created_once() {
    let mut vm = Vm::new();
    let first = vm.namespace("packA");
    vm.eval_str_in(&first, "(define x 1)").unwrap();
    // Asking again returns the same table, not a fresh one.
    let again = vm.namespace("packA");
    assert!(matches!(vm.global_in(&again, "x"), Some(Val::Num(1))));
    assert!(vm.find_namespace("packA").is_some());
    assert!(vm.find_namespace("nope").is_none());
}

#[test]
fn a_failed_batch_rolls_back_only_its_own_namespace() {
    let mut vm = Vm::new();
    let a = vm.namespace("packA");
    vm.eval_str("(define root-name 'kept)").unwrap();
    let err = vm
        .eval_str_in(&a, "(define pack-name 1) (car 5)")
        .expect_err("batch fails");
    assert!(err.msg.contains("pair"), "{err}");
    // The pack's placeholder is gone …
    assert!(vm.global_in(&a, "pack-name").is_none());
    // … and root is untouched.
    assert_eq!(format!("{}", vm.eval_str("root-name").unwrap()), "kept");
}

#[test]
fn dropping_the_vm_releases_pack_namespaces() {
    // ADR-015's property, extended to packs: a closure reaches its
    // namespace through a `Weak`, so nothing a pack defines can root the
    // Vm that owns it.
    let weak = {
        let mut vm = Vm::new();
        let a = vm.namespace("packA");
        vm.eval_str_in(&a, "(define f (lambda (x) (f x)))").unwrap();
        vm.export(&a, &["f"]).unwrap();
        std::rc::Rc::downgrade(&a)
    };
    assert!(
        weak.upgrade().is_none(),
        "a pack namespace outlived its Vm — something rooted it"
    );
}
