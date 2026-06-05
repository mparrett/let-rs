//! Regression coverage for the host-world primitives. The codex review
//! flagged `world-apply!` as effectively unbounded and `coord` as silently
//! wrapping `i64 → u32`; these tests pin both behaviors.

use std::cell::RefCell;
use std::rc::Rc;

use macros::MacroVm;
use world::World;

fn vm_with_world(w: u32, h: u32) -> MacroVm {
    let world = Rc::new(RefCell::new(World::new(w, h).expect("dims fit")));
    let mut vm = MacroVm::new();
    spells::install_with_world(&mut vm, world);
    vm
}

#[test]
fn world_apply_clamps_area_to_grid() {
    // area = 1_000_000_000 on a 7×5 grid pre-fix would iterate ~4e18 cells.
    // Clamping the rect to the grid intersection keeps it at 35 cells.
    let mut vm = vm_with_world(7, 5);
    let src = "(world-apply! \
                 (assoc-set 'tx 3 \
                   (assoc-set 'ty 2 \
                     (assoc-set 'area 1000000000 \
                       (assoc-set 'element 'fire '())))))";
    let painted = vm.eval_str(src).expect("world-apply!");
    // Whole 7×5 grid = 35 tiles painted.
    assert_eq!(format!("{painted}"), "35");
}

#[test]
fn world_apply_with_zero_dim_world_is_safe() {
    // 0×0 world: nothing to paint, no panic.
    let mut vm = vm_with_world(0, 0);
    let src = "(world-apply! \
                 (assoc-set 'tx 0 \
                   (assoc-set 'ty 0 \
                     (assoc-set 'area 5 \
                       (assoc-set 'element 'fire '())))))";
    let painted = vm.eval_str(src).expect("world-apply!");
    assert_eq!(format!("{painted}"), "0");
}

#[test]
fn world_set_tile_rejects_out_of_range_coord() {
    // Pre-fix `*n as u32` silently wrapped for n > u32::MAX, painting a
    // tile at an unrelated location. Now coord errors cleanly.
    let mut vm = vm_with_world(7, 5);
    let r = vm.eval_str("(world-set-tile! 5000000000 0 'fire)");
    assert!(
        matches!(&r, Err(e) if e.contains("u32 range")),
        "expected u32-range error, got {r:?}"
    );
}

#[test]
fn world_new_rejects_unaddressable_dims() {
    // u32::MAX × u32::MAX would wrap to 1 in the old code, leaving width
    // and height huge but tiles size 1 — a fully-broken World.
    let r = World::new(u32::MAX, u32::MAX);
    assert!(
        matches!(&r, Err(e) if e.contains("addressable")),
        "expected addressable-cells error, got {r:?}"
    );
}

// ── tile decay via lisp (ADR-027) ─────────────────────────────────

fn vm_with_world_handle(w: u32, h: u32) -> (macros::MacroVm, Rc<RefCell<World>>) {
    let world = Rc::new(RefCell::new(World::new(w, h).expect("dims fit")));
    let mut vm = macros::MacroVm::new();
    spells::install_with_world(&mut vm, world.clone());
    (vm, world)
}

#[test]
fn world_apply_with_power_writes_lifetime() {
    // power 3 → lifetime 3 on every painted tile.
    let (mut vm, world) = vm_with_world_handle(5, 5);
    vm.eval_str(
        "(world-apply! \
           (assoc-set 'tx 2 \
             (assoc-set 'ty 2 \
               (assoc-set 'power 3 \
                 (assoc-set 'element 'fire '())))))",
    )
    .unwrap();
    assert_eq!(world.borrow().lifetime_at(2, 2), Some(3));
}

#[test]
fn world_apply_without_power_uses_default_lifetime() {
    // No power in ctx → DEFAULT_LIFETIME (5). The default isn't a magic
    // number; if we change it, this test catches the drift.
    let (mut vm, world) = vm_with_world_handle(3, 3);
    vm.eval_str(
        "(world-apply! \
           (assoc-set 'tx 1 \
             (assoc-set 'ty 1 \
               (assoc-set 'element 'fire '()))))",
    )
    .unwrap();
    assert_eq!(world.borrow().lifetime_at(1, 1), Some(5));
}

#[test]
fn world_tick_decays_over_time() {
    // Cast with power 2, then tick twice — first tick keeps the tile,
    // second tick reverts. world-tick! returns the revert count.
    let (mut vm, world) = vm_with_world_handle(3, 3);
    vm.eval_str(
        "(world-apply! \
           (assoc-set 'tx 1 \
             (assoc-set 'ty 1 \
               (assoc-set 'power 2 \
                 (assoc-set 'element 'fire '())))))",
    )
    .unwrap();
    let r = vm.eval_str("(world-tick!)").unwrap();
    assert_eq!(format!("{r}"), "0");
    assert_eq!(
        format!("{}", world.borrow().tile_at(1, 1).unwrap().as_sym()),
        "fire"
    );
    let r = vm.eval_str("(world-tick!)").unwrap();
    assert_eq!(format!("{r}"), "1");
    assert_eq!(
        format!("{}", world.borrow().tile_at(1, 1).unwrap().as_sym()),
        "floor"
    );
}

#[test]
fn world_tick_on_empty_world_returns_zero() {
    let (mut vm, _world) = vm_with_world_handle(3, 3);
    let r = vm.eval_str("(world-tick!)").unwrap();
    assert_eq!(format!("{r}"), "0");
}

#[test]
fn world_apply_duration_takes_priority_over_power() {
    // duration 2, power 9 → lifetime should be 2 (duration wins).
    // Verifies the duration > power > default fallback chain (ADR-027
    // refinement after the ᛃ rune landed).
    let (mut vm, world) = vm_with_world_handle(3, 3);
    vm.eval_str(
        "(world-apply! \
           (assoc-set 'tx 1 \
             (assoc-set 'ty 1 \
               (assoc-set 'duration 2 \
                 (assoc-set 'power 9 \
                   (assoc-set 'element 'fire '()))))))",
    )
    .unwrap();
    assert_eq!(world.borrow().lifetime_at(1, 1), Some(2));
}

#[test]
fn world_apply_with_zero_power_paints_permanently() {
    // power = 0 in ctx → lifetime 0 (permanent). Lets a caller opt out
    // of decay without rebuilding the model. Mostly an edge-case nicety
    // for tests / future fancier spells.
    let (mut vm, world) = vm_with_world_handle(3, 3);
    vm.eval_str(
        "(world-apply! \
           (assoc-set 'tx 1 \
             (assoc-set 'ty 1 \
               (assoc-set 'power 0 \
                 (assoc-set 'element 'fire '())))))",
    )
    .unwrap();
    assert_eq!(world.borrow().lifetime_at(1, 1), Some(0));
    vm.eval_str("(world-tick!)").unwrap();
    vm.eval_str("(world-tick!)").unwrap();
    vm.eval_str("(world-tick!)").unwrap();
    assert_eq!(
        format!("{}", world.borrow().tile_at(1, 1).unwrap().as_sym()),
        "fire"
    );
}
