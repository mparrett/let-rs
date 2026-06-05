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

// ── mana model (ADR-028) ──────────────────────────────────────────

fn vm_with_world(w: u32, h: u32) -> MacroVm {
    let world = Rc::new(RefCell::new(World::new(w, h).expect("dims fit")));
    let mut vm = MacroVm::new();
    spells::install_with_world(&mut vm, world);
    vm
}

#[test]
fn mana_starts_at_max() {
    let mut vm = mvm();
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "10");
    assert_eq!(format!("{}", vm.eval_str("max-mana").unwrap()), "10");
}

#[test]
fn cast_decrements_mana_by_cost() {
    // cost = 1 + power + area. A bare (fire) ctx has neither, so
    // cost = 1; mana goes 10 → 9.
    let mut vm = vm_with_world(5, 5);
    vm.eval_str(
        "(cast! (assoc-set 'tx 2 (assoc-set 'ty 2 (thread (start) (list fire)))))",
    )
    .unwrap();
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "9");
}

#[test]
fn cast_with_area_and_power_costs_more() {
    // cost = 1 + power(2) + area(1) = 4. mana 10 → 6.
    let mut vm = vm_with_world(5, 5);
    vm.eval_str(
        "(cast! (assoc-set 'tx 2 (assoc-set 'ty 2 \
                  (thread (start) (list fire (area 1) (power 2))))))",
    )
    .unwrap();
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "6");
}

#[test]
fn cast_refuses_when_mana_insufficient() {
    // Drain mana to 2, then attempt a cost-4 cast — refused.
    // Mana unchanged, returns 0 painted.
    let mut vm = vm_with_world(5, 5);
    vm.eval_str("(set! mana 2)").unwrap();
    let r = vm
        .eval_str(
            "(cast! (assoc-set 'tx 2 (assoc-set 'ty 2 \
                      (thread (start) (list fire (power 3))))))",
        )
        .unwrap();
    assert_eq!(format!("{r}"), "0");
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "2");
}

#[test]
fn cast_succeeds_at_exact_mana() {
    // mana = cost = 3 (1 + power 2). Cast goes through, mana drops to 0.
    let mut vm = vm_with_world(5, 5);
    vm.eval_str("(set! mana 3)").unwrap();
    let r = vm
        .eval_str(
            "(cast! (assoc-set 'tx 2 (assoc-set 'ty 2 \
                      (thread (start) (list fire (power 2))))))",
        )
        .unwrap();
    // 1 tile painted (area = 0, just the center).
    assert_eq!(format!("{r}"), "1");
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "0");
}

#[test]
fn tick_regens_one_mana() {
    let mut vm = vm_with_world(5, 5);
    vm.eval_str("(set! mana 5)").unwrap();
    vm.eval_str("(tick!)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "6");
}

#[test]
fn tick_caps_mana_at_max() {
    // Mana already at max → tick! leaves it alone (no overflow).
    let mut vm = vm_with_world(5, 5);
    vm.eval_str("(tick!)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "10");
}

#[test]
fn tick_returns_decay_count_and_regens_mana() {
    // Cast (cost 2), tick: decay count = 0 because lifetime hasn't
    // expired yet, but mana should regen by 1.
    let mut vm = vm_with_world(5, 5);
    vm.eval_str(
        "(cast! (assoc-set 'tx 2 (assoc-set 'ty 2 \
                  (thread (start) (list fire (power 3))))))",
    )
    .unwrap();
    // After cast: mana = 10 - (1+3) = 6.
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "6");
    let r = vm.eval_str("(tick!)").unwrap();
    // Lifetime 3 → 2, no decay; tick returns 0.
    assert_eq!(format!("{r}"), "0");
    // Mana regen'd: 6 → 7.
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "7");
}

#[test]
fn reset_mana_restores_max() {
    let mut vm = mvm();
    vm.eval_str("(set! mana 0)").unwrap();
    vm.eval_str("(reset-mana!)").unwrap();
    assert_eq!(format!("{}", vm.eval_str("mana").unwrap()), "10");
}

#[test]
fn spell_cost_formula() {
    let mut vm = mvm();
    // The reader doesn't parse dotted-pair literals — alist entries
    // are built via assoc-set so the cons structure matches what
    // assoc-get expects (head-key, tail-val cons pair).
    //
    // Empty ctx → cost 1.
    assert_eq!(format!("{}", vm.eval_str("(spell-cost '())").unwrap()), "1");
    // Just power 5 → cost 6.
    assert_eq!(
        format!(
            "{}",
            vm.eval_str("(spell-cost (assoc-set 'power 5 '()))").unwrap()
        ),
        "6"
    );
    // Power 2 + area 3 → cost 6.
    assert_eq!(
        format!(
            "{}",
            vm.eval_str("(spell-cost (assoc-set 'power 2 (assoc-set 'area 3 '())))")
                .unwrap()
        ),
        "6"
    );
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
