//! Minimal tile grid + event log used by the spell demo. A reusable
//! host-state building block — the lisp engine has no awareness it
//! exists (ADR-017, ADR-018). Hosts wire it in by calling
//! `world::world_prim::install(&mut vm, world.clone())`.
//!
//! Depends only on `lisp` (`Vm`, `Val`, `Arity`); zero other deps.

use std::fmt;

pub mod world_prim;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tile {
    Floor,
    Wall,
    Fire,
    Ice,
    Earth,
    Water,
    Mud,
    Lava,
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Tile::Floor => '.',
            Tile::Wall => '#',
            Tile::Fire => '*',
            Tile::Ice => 'o',
            Tile::Earth => '%',
            Tile::Water => '~',
            Tile::Mud => '&',
            Tile::Lava => '^',
        }
    }

    pub fn from_sym(s: &str) -> Option<Tile> {
        match s {
            "floor" => Some(Tile::Floor),
            "wall" => Some(Tile::Wall),
            "fire" => Some(Tile::Fire),
            "ice" => Some(Tile::Ice),
            "earth" => Some(Tile::Earth),
            "water" => Some(Tile::Water),
            "mud" => Some(Tile::Mud),
            "lava" => Some(Tile::Lava),
            _ => None,
        }
    }

    pub fn as_sym(self) -> &'static str {
        match self {
            Tile::Floor => "floor",
            Tile::Wall => "wall",
            Tile::Fire => "fire",
            Tile::Ice => "ice",
            Tile::Earth => "earth",
            Tile::Water => "water",
            Tile::Mud => "mud",
            Tile::Lava => "lava",
        }
    }
}

/// A cast scheduled to fire on a future tick. Aftershock effect from
/// the `ᛃ` rune (ADR-029): the caller pays the mana cost up front,
/// the world reschedules the same area-paint to land later. The
/// pending cast carries everything `paint_area` needs to replay the
/// effect (element, target, area, lifetime); it does NOT carry its
/// own aftershock count, so an aftershock cannot itself spawn another
/// aftershock — chains terminate after exactly one re-strike.
#[derive(Debug, Clone)]
pub struct PendingCast {
    pub countdown: u8,
    pub tile: Tile,
    pub tx: i64,
    pub ty: i64,
    pub area: i64,
    pub lifetime: u8,
}

#[derive(Debug)]
pub struct World {
    pub width: u32,
    pub height: u32,
    tiles: Vec<Tile>,
    /// Parallel to `tiles`. `0` = permanent (never decays); positive =
    /// ticks remaining before the tile reverts to Floor. Stored
    /// separately so the `Tile` enum stays simple — most callers
    /// (rendering, tile_at) don't care about lifetime. See ADR-027.
    lifetimes: Vec<u8>,
    /// Scheduled aftershocks (ADR-029). Each `world-tick!` decrements
    /// every entry; entries that hit zero fire (paint the area) and
    /// drop out. Limited to scheduled-at-cast-time only — re-strikes
    /// don't recursively spawn more.
    pending: Vec<PendingCast>,
    pub log: Vec<String>,
}

impl World {
    /// Construct a grid of `width × height` floor tiles. Errors if the
    /// product exceeds `usize::MAX` — the previous `u32 * u32 as usize`
    /// wrapped silently, leaving a small tile vec with width/height
    /// stored as the unwrapped values (so subsequent `tile_at` indexed
    /// past the allocation).
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        // Vec's capacity ceiling for a non-ZST is `isize::MAX` bytes —
        // anything past that aborts inside Vec, so we check against the
        // tighter bound here rather than `usize::MAX`.
        let total = (width as u64)
            .checked_mul(height as u64)
            .filter(|&n| n <= isize::MAX as u64)
            .ok_or_else(|| format!("World::new: {width}×{height} exceeds addressable cells"))?;
        let n = total as usize;
        Ok(World {
            width,
            height,
            tiles: vec![Tile::Floor; n],
            lifetimes: vec![0; n],
            pending: Vec::new(),
            log: Vec::new(),
        })
    }

    pub fn empty() -> Self {
        // 0×0 is always representable.
        Self::new(0, 0).expect("0×0 is in range")
    }

    fn idx(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y * self.width + x) as usize)
        } else {
            None
        }
    }

    pub fn tile_at(&self, x: u32, y: u32) -> Option<Tile> {
        self.idx(x, y).map(|i| self.tiles[i])
    }

    pub fn lifetime_at(&self, x: u32, y: u32) -> Option<u8> {
        self.idx(x, y).map(|i| self.lifetimes[i])
    }

    pub fn set_tile(&mut self, x: u32, y: u32, t: Tile) -> bool {
        match self.idx(x, y) {
            Some(i) => {
                self.tiles[i] = t;
                self.lifetimes[i] = 0;
                true
            }
            None => false,
        }
    }

    /// Paint a tile with a finite lifetime. `lifetime = 0` is treated
    /// as permanent (matches `set_tile`); positive values count down
    /// on each `tick`, reverting to Floor at zero. Used by
    /// `world-apply!` to make Fire/Ice decay; `world-set-tile!`
    /// continues to paint permanently for tape-painted walls.
    pub fn set_tile_with_lifetime(&mut self, x: u32, y: u32, t: Tile, lifetime: u8) -> bool {
        match self.idx(x, y) {
            Some(i) => {
                self.tiles[i] = t;
                self.lifetimes[i] = lifetime;
                true
            }
            None => false,
        }
    }

    /// Paint a square neighborhood of radius `area` around `(tx, ty)`
    /// with `tile` + `lifetime`. Clamps to the grid intersection so an
    /// `area` past the world bounds is cheap. Returns the count of
    /// tiles actually painted. Shared between the immediate-cast path
    /// (world-apply!) and the aftershock fire path (tick).
    pub fn paint_area(&mut self, tile: Tile, tx: i64, ty: i64, area: i64, lifetime: u8) -> u32 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        let area = area.max(0);
        let w_max = (self.width - 1) as i64;
        let h_max = (self.height - 1) as i64;
        let x_lo = tx.saturating_sub(area).max(0);
        let x_hi = tx.saturating_add(area).min(w_max);
        let y_lo = ty.saturating_sub(area).max(0);
        let y_hi = ty.saturating_add(area).min(h_max);
        if x_lo > x_hi || y_lo > y_hi {
            return 0;
        }
        let mut painted = 0u32;
        for y in y_lo..=y_hi {
            for x in x_lo..=x_hi {
                if self.set_tile_with_lifetime(x as u32, y as u32, tile, lifetime) {
                    painted += 1;
                }
            }
        }
        painted
    }

    /// Schedule a delayed re-cast (ADR-029 aftershock). Called by
    /// `world-apply!` when ctx carries an `aftershock` > 0. The same
    /// `area` and `lifetime` ride along so the fire replays the
    /// original effect, not a different one.
    pub fn schedule_aftershock(&mut self, cast: PendingCast) {
        self.pending.push(cast);
    }

    /// Advance the world by one tick:
    ///  1. Every tile with a positive lifetime decrements; tiles
    ///     that hit zero revert to Floor (ADR-027).
    ///  2. Every pending aftershock decrements; entries that hit
    ///     zero fire (`paint_area`) and drop out (ADR-029).
    ///
    /// Returns the number of tiles that reverted (kept this shape
    /// for back-compat with the world-tick! prim — fired aftershocks
    /// show up via grid changes + log entries instead).
    pub fn tick(&mut self) -> u32 {
        let mut reverted = 0u32;
        for i in 0..self.tiles.len() {
            if self.lifetimes[i] > 0 {
                self.lifetimes[i] -= 1;
                if self.lifetimes[i] == 0 {
                    self.tiles[i] = Tile::Floor;
                    reverted += 1;
                }
            }
        }
        // Pending aftershocks: decrement first, then split into
        // "fires this tick" vs "still pending" so paint_area can
        // take its own &mut self after the pending borrow is done.
        let mut fired_now = Vec::new();
        let mut still_pending = Vec::with_capacity(self.pending.len());
        for mut p in self.pending.drain(..) {
            if p.countdown > 0 {
                p.countdown -= 1;
            }
            if p.countdown == 0 {
                fired_now.push(p);
            } else {
                still_pending.push(p);
            }
        }
        self.pending = still_pending;
        for p in fired_now {
            let painted = self.paint_area(p.tile, p.tx, p.ty, p.area, p.lifetime);
            self.log.push(format!(
                "aftershock {} at ({},{}) → {painted} tiles",
                p.tile.as_sym(),
                p.tx,
                p.ty
            ));
        }
        reverted
    }

    /// Number of pending aftershocks. Useful for tests + a future UI
    /// indicator ("3 aftershocks queued").
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn log_event(&mut self, msg: String) {
        self.log.push(msg);
    }
}

impl fmt::Display for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                write!(f, "{}", self.tile_at(x, y).unwrap().glyph())?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
