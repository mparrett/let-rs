use std::fmt;

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
    pub fn new(width: u32, height: u32) -> Self {
        World {
            width,
            height,
            tiles: vec![Tile::Floor; (width * height) as usize],
            log: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self::new(0, 0)
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
