//! Tile decay model (ADR-027): every painted tile carries a `u8`
//! lifetime, `world-apply!` writes it from ctx `power`, `(world-tick!)`
//! decrements and reverts at zero. These tests exercise the World struct
//! directly — the lisp-level integration lives in `crates/lisp/tests/
//! world.rs`.

use world::{Tile, World};

fn w(width: u32, height: u32) -> World {
    World::new(width, height).expect("dims fit")
}

#[test]
fn fresh_world_tick_reverts_nothing() {
    let mut w = w(4, 4);
    assert_eq!(w.tick(), 0);
}

#[test]
fn permanent_tile_survives_many_ticks() {
    let mut w = w(4, 4);
    assert!(w.set_tile(1, 1, Tile::Wall));
    for _ in 0..100 {
        assert_eq!(w.tick(), 0);
    }
    assert_eq!(w.tile_at(1, 1), Some(Tile::Wall));
}

#[test]
fn finite_lifetime_decrements_until_revert() {
    let mut w = w(4, 4);
    assert!(w.set_tile_with_lifetime(2, 2, Tile::Fire, 3));
    assert_eq!(w.lifetime_at(2, 2), Some(3));
    assert_eq!(w.tick(), 0); // lifetime 3 → 2
    assert_eq!(w.lifetime_at(2, 2), Some(2));
    assert_eq!(w.tile_at(2, 2), Some(Tile::Fire));
    assert_eq!(w.tick(), 0); // 2 → 1
    assert_eq!(w.tick(), 1); // 1 → 0 + revert
    assert_eq!(w.tile_at(2, 2), Some(Tile::Floor));
    assert_eq!(w.lifetime_at(2, 2), Some(0));
}

#[test]
fn lifetime_zero_is_permanent() {
    // set_tile_with_lifetime(_, 0) is the same shape as set_tile —
    // permanent. The "lifetime 0 = permanent" convention keeps the
    // legacy set_tile path working unchanged.
    let mut w = w(4, 4);
    assert!(w.set_tile_with_lifetime(0, 0, Tile::Fire, 0));
    assert_eq!(w.tick(), 0);
    assert_eq!(w.tile_at(0, 0), Some(Tile::Fire));
}

#[test]
fn set_tile_resets_lifetime_to_zero() {
    // Painting over an existing decay-tile with set_tile (the
    // permanent path) zeros the lifetime — the tile becomes
    // permanent again. Prevents a "left-over decay" surprise.
    let mut w = w(4, 4);
    w.set_tile_with_lifetime(0, 0, Tile::Fire, 5);
    w.set_tile(0, 0, Tile::Wall);
    assert_eq!(w.lifetime_at(0, 0), Some(0));
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.tile_at(0, 0), Some(Tile::Wall));
}

#[test]
fn tick_counts_only_this_tick_reverts() {
    // Three tiles with lifetimes 1, 2, 3. After tick 1, only the
    // lifetime-1 tile reverts (count = 1). After tick 2, the
    // lifetime-2 tile reverts (count = 1). Then 3 reverts on tick 3.
    let mut w = w(8, 1);
    w.set_tile_with_lifetime(0, 0, Tile::Fire, 1);
    w.set_tile_with_lifetime(1, 0, Tile::Fire, 2);
    w.set_tile_with_lifetime(2, 0, Tile::Fire, 3);
    assert_eq!(w.tick(), 1);
    assert_eq!(w.tile_at(0, 0), Some(Tile::Floor));
    assert_eq!(w.tile_at(1, 0), Some(Tile::Fire));
    assert_eq!(w.tick(), 1);
    assert_eq!(w.tile_at(1, 0), Some(Tile::Floor));
    assert_eq!(w.tile_at(2, 0), Some(Tile::Fire));
    assert_eq!(w.tick(), 1);
    assert_eq!(w.tile_at(2, 0), Some(Tile::Floor));
    assert_eq!(w.tick(), 0);
}

#[test]
fn lifetime_at_out_of_bounds_is_none() {
    let w = w(2, 2);
    assert_eq!(w.lifetime_at(5, 5), None);
}

#[test]
fn tile_sym_roundtrip_covers_alchemy_tiles() {
    // Sanity: every Tile variant has matching from_sym/as_sym arms.
    // Added when ADR-030 introduced earth/water/mud/lava — the
    // alchemy mechanic depends on Tile::from_sym recognizing the
    // mixed symbol names emitted by the spells prelude.
    for name in [
        "floor", "wall", "fire", "ice", "earth", "water", "mud", "lava",
    ] {
        let t = Tile::from_sym(name).unwrap_or_else(|| panic!("missing from_sym arm: {name}"));
        assert_eq!(t.as_sym(), name);
    }
}
