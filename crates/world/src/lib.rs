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
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Tile::Floor => '.',
            Tile::Wall => '#',
            Tile::Fire => '*',
            Tile::Ice => 'o',
        }
    }

    pub fn from_sym(s: &str) -> Option<Tile> {
        match s {
            "floor" => Some(Tile::Floor),
            "wall" => Some(Tile::Wall),
            "fire" => Some(Tile::Fire),
            "ice" => Some(Tile::Ice),
            _ => None,
        }
    }

    pub fn as_sym(self) -> &'static str {
        match self {
            Tile::Floor => "floor",
            Tile::Wall => "wall",
            Tile::Fire => "fire",
            Tile::Ice => "ice",
        }
    }
}

#[derive(Debug)]
pub struct World {
    pub width: u32,
    pub height: u32,
    tiles: Vec<Tile>,
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
        Ok(World {
            width,
            height,
            tiles: vec![Tile::Floor; total as usize],
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

    pub fn set_tile(&mut self, x: u32, y: u32, t: Tile) -> bool {
        match self.idx(x, y) {
            Some(i) => {
                self.tiles[i] = t;
                true
            }
            None => false,
        }
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
