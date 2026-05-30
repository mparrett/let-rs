//! Lock the ADR-017 promise: a host can register a closure-capturing
//! primitive that mutates state owned outside the Vm — no `World`
//! involvement. If `Val::Prim` ever drops the `Rc<dyn Fn>` shape, these
//! tests fail loudly.

use std::cell::RefCell;
use std::rc::Rc;

use lisp::val::Arity;
use lisp::{Val, Vm};

#[test]
fn closure_prim_can_mutate_captured_host_state() {
    // The host owns a counter. The prim captures an `Rc<RefCell<i64>>`
    // and bumps it on every call — the engine has no awareness this is
    // happening, which is the point.
    let counter: Rc<RefCell<i64>> = Rc::new(RefCell::new(0));
    let counter_clone = counter.clone();

    let mut vm = Vm::new();
    vm.register_prim("bump!", Arity::Exact(0), move |_args| {
        *counter_clone.borrow_mut() += 1;
        Ok(Val::Bool(true))
    });

    vm.eval_str("(bump!) (bump!) (bump!)").expect("three calls");
    assert_eq!(*counter.borrow(), 3);
}

#[test]
fn closure_prim_can_read_and_return_host_state() {
    let cell: Rc<RefCell<i64>> = Rc::new(RefCell::new(42));
    let cell_for_get = cell.clone();
    let cell_for_set = cell.clone();

    let mut vm = Vm::new();
    vm.register_prim("get-state", Arity::Exact(0), move |_args| {
        Ok(Val::Num(*cell_for_get.borrow()))
    });
    vm.register_prim("set-state!", Arity::Exact(1), move |args| match &args[0] {
        Val::Num(n) => {
            *cell_for_set.borrow_mut() = *n;
            Ok(Val::Bool(true))
        }
        other => Err(format!("set-state!: expected int, got {other}")),
    });

    let r = vm.eval_str("(get-state)").expect("read");
    assert_eq!(format!("{r}"), "42");

    vm.eval_str("(set-state! 99)").expect("write");
    assert_eq!(*cell.borrow(), 99);
}

#[test]
fn dropping_vm_releases_closure_prim_captures() {
    // ADR-017's analogue of ADR-015's `dropping_vm_releases_top_level_closures`:
    // when the Vm drops, its env chain drops, every `Val::Prim` drops,
    // every captured `Rc` refcount falls. A `Weak` to the captured cell
    // upgrades to `None` once nothing else holds it.
    let cell: Rc<RefCell<i64>> = Rc::new(RefCell::new(0));
    let weak = Rc::downgrade(&cell);
    let cell_for_closure = cell.clone();

    let mut vm = Vm::new();
    vm.register_prim("noop", Arity::Exact(0), move |_args| {
        let _read = cell_for_closure.borrow();
        Ok(Val::Bool(true))
    });

    // Drop the host's strong ref; only the closure keeps `cell` alive now.
    drop(cell);
    assert!(
        weak.upgrade().is_some(),
        "closure still holds cell while Vm lives"
    );

    drop(vm);
    assert!(
        weak.upgrade().is_none(),
        "dropping the Vm should release the closure's captured cell"
    );
}
