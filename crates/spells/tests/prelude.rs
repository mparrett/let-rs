//! Pin the rune prelude's behavior end-to-end and verify the `defspell`
//! / `defparam` macros produce the same closures as the pre-ADR-025
//! hand-rolled defines. If anyone restructures the prelude, these tests
//! catch any drift in the observable ctx shape that callers depend on.

use std::cell::RefCell;
use std::rc::Rc;

use macros::MacroVm;
use world::World;

fn mvm() -> MacroVm {
    let mut vm = MacroVm::new();
    spells::install(&mut vm);
    vm
}

#[test]
fn defspell_produces_constant_ctx_setter() {
    let mut vm = mvm();
    // (fire '()) → ((element . fire))
    let r = vm.eval_str("(fire '())").expect("eval fire");
    assert_eq!(format!("{r}"), "((element . fire))");
}

#[test]
fn defparam_closes_over_arg() {
    let mut vm = mvm();
    // ((area 5) '()) → ((area . 5))
    let r = vm.eval_str("((area 5) '())").expect("eval (area 5)");
    assert_eq!(format!("{r}"), "((area . 5))");
}

#[test]
fn canonical_cast_threads_three_runes() {
    // Mirrors the example/spells.rs canonical cast: fire, area-3, ice.
    // Threaded right-to-left through assoc-set means ice is the most
    // recently written, so it sits at the head of the alist.
    let mut vm = mvm();
    let body = "(thread (start) (list fire (area 3) ice))";
    let r = vm.eval_str(body).expect("eval cast");
    assert_eq!(
        format!("{r}"),
        "((element . ice) (area . 3) (element . fire))"
    );
}

#[test]
fn defspell_and_defparam_are_local_macros() {
    // The macros are registered in the spells install but they're
    // ordinary defmacro forms, so they remain available for host code
    // to extend the vocabulary after the install.
    let mut vm = mvm();
    vm.eval_str("(defspell water element water)")
        .expect("user defspell");
    let r = vm.eval_str("(water '())").expect("eval water");
    assert_eq!(format!("{r}"), "((element . water))");
}

#[test]
fn install_with_world_wires_world_apply() {
    // Both halves of install_with_world land: the prelude + the world
    // prims. A canonical cast onto a 7×5 world should paint tiles.
    let world = Rc::new(RefCell::new(World::new(7, 5).expect("dims fit")));
    let mut vm = MacroVm::new();
    spells::install_with_world(&mut vm, world.clone());
    let src = "(world-apply! \
                 (assoc-set 'tx 3 \
                   (assoc-set 'ty 2 \
                     (thread (start) (list fire (area 1))))))";
    let painted = vm.eval_str(src).expect("world-apply!");
    // area 1 = (2·1+1)² = 9-tile box centered at (3, 2), all in-bounds.
    assert_eq!(format!("{painted}"), "9");
}
