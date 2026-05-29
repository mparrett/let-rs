//! Regression coverage for the host-world primitives. The codex review
//! flagged `world-apply!` as effectively unbounded and `coord` as silently
//! wrapping `i64 → u32`; these tests pin both behaviors.

use lisp::{Vm, World};

fn vm_with_world(w: u32, h: u32) -> Vm {
    let mut vm = Vm::with_world(World::new(w, h).expect("dims fit"));
    spells::install(&mut vm); // gives us assoc-set, fire, etc.
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
