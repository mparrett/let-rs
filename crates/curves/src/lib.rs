//! L-system DSL host support: turtle state, side-effecting prims,
//! pure-lisp rewrite engine, and an ASCII canvas renderer.
//!
//! Sibling to [`spells`](../spells/index.html) and [`genes`](../genes/index.html)
//! — paired with the stroke-tape alphabet in `crates/strokes/`. See
//! ADR-019 for the design rationale (8-direction turtle, symbol-list
//! tape, host-owned turtle state via `Rc<RefCell<Turtle>>`).
//!
//! Depends only on `lisp` (`Vm`, `Val`, `Arity`). The CEK engine itself
//! is untouched — the turtle lives in a closure captured by the prims
//! at install time, the same pattern `world::world_prim::install` uses.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use lisp::NsHandle;
use lisp::Vm;
use lisp::val::{Arity, Val};

/// Seed-independent half of the curves prelude — pure-lisp helpers for
/// L-system rewrite. `expand` does one rewrite pass over a symbol list;
/// `grow` iterates `expand` `n` times. Rules format is a list of lists
/// where each entry's car is the symbol to replace and cdr is the
/// replacement sequence — e.g. `((F F + F - F))` rewrites `F → F+F-F`.
///
/// `expand-one` falls back to the singleton list of the input symbol
/// when no rule matches (identity rewrite), so `+`, `-`, `[`, `]` pass
/// through every rewrite pass unchanged unless the user explicitly
/// rules them.
pub const PRELUDE_DEFINES: &str = r#"
(define expand-one
  (lambda (sym rules)
    (cond ((null? rules) (list sym))
          ((eq? sym (car (car rules))) (cdr (car rules)))
          (else (expand-one sym (cdr rules))))))

(define expand
  (lambda (tape rules)
    (if (null? tape)
        '()
        (append (expand-one (car tape) rules)
                (expand (cdr tape) rules)))))

(define grow
  (lambda (axiom rules n)
    (if (= n 0) axiom
        (grow (expand axiom rules) rules (- n 1)))))
"#;

/// 8-direction turtle. Heading is `0..8` with 45° per tick, starting
/// at East and incrementing counter-clockwise (so `+` / turn-left
/// bumps the index). The canvas is sparse — `cells` only contains
/// positions a `forward!` has stamped — and auto-sizes to its bbox
/// at render time.
pub struct Turtle {
    x: i32,
    y: i32,
    heading: u8,
    stack: Vec<(i32, i32, u8)>,
    /// Stamped cells; value is the heading-glyph at the moment of
    /// stamping. Last-write-wins on crossings (the second pass
    /// overwrites the first, picking the second pass's heading).
    cells: HashMap<(i32, i32), char>,
}

impl Default for Turtle {
    fn default() -> Self {
        Turtle::new()
    }
}

impl Turtle {
    pub fn new() -> Self {
        Turtle {
            x: 0,
            y: 0,
            heading: 0,
            stack: Vec::new(),
            cells: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.x = 0;
        self.y = 0;
        self.heading = 0;
        self.stack.clear();
        self.cells.clear();
    }

    /// Move one cell in the current heading. If `draw` is true, stamp
    /// the destination cell with the heading-glyph (one of `─ ╱ │ ╲`).
    pub fn forward(&mut self, draw: bool) {
        let (dx, dy) = HEADING_DELTAS[self.heading as usize];
        self.x += dx;
        self.y += dy;
        if draw {
            self.cells
                .insert((self.x, self.y), heading_glyph(self.heading));
        }
    }

    /// Rotate. `delta` is in 45° ticks: `+1` = left (CCW),
    /// `-1` = right (CW). Wraps mod 8.
    pub fn turn(&mut self, delta: i8) {
        self.heading = (self.heading as i8 + delta).rem_euclid(8) as u8;
    }

    pub fn push(&mut self) {
        self.stack.push((self.x, self.y, self.heading));
    }

    /// Returns `Err` on underflow so a tape with unmatched `]` surfaces
    /// as a clean evaluator error instead of silently no-opping.
    pub fn pop(&mut self) -> Result<(), String> {
        match self.stack.pop() {
            Some((x, y, h)) => {
                self.x = x;
                self.y = y;
                self.heading = h;
                Ok(())
            }
            None => Err("pop on empty turtle stack (unmatched ']')".into()),
        }
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }
}

const HEADING_DELTAS: [(i32, i32); 8] = [
    (1, 0),   // 0: East
    (1, -1),  // 1: NE
    (0, -1),  // 2: North
    (-1, -1), // 3: NW
    (-1, 0),  // 4: West
    (-1, 1),  // 5: SW
    (0, 1),   // 6: South
    (1, 1),   // 7: SE
];

/// Pick a glyph that visually matches the heading direction. Diagonals
/// use `╱` (NE / SW) and `╲` (NW / SE); cardinals use `─` and `│`.
fn heading_glyph(h: u8) -> char {
    match h {
        0 | 4 => '─',
        1 | 5 => '╱',
        2 | 6 => '│',
        3 | 7 => '╲',
        _ => '?', // unreachable: heading is always 0..8
    }
}

/// Upper bound on the *dense* canvas a render will materialize. The
/// turtle's `cells` map is sparse, but the rendered grid is not: a
/// diagonal walk of N cells spans an N×N bbox, so cell count is a poor
/// proxy for output size. Without this cap, `(draw! (grow '(+ F)
/// '((F F F)) 14))` — well inside the wasm host's 10M step budget —
/// asks for a 16384² grid (~1 GB) and takes the page down.
///
/// Two million cells permits every cheatsheet curve with room to spare
/// (the widest is well under 1.1M) while keeping the returned string
/// inside what a `<pre>` can hold without janking. Mirrors the
/// `World::new` `MAX_CELLS` precedent.
const MAX_CANVAS_CELLS: u64 = 2_000_000;

/// Render the turtle's stamped cells to a multi-line ASCII string.
/// Auto-sizes to the bbox of visited cells; unvisited interior cells
/// become spaces. Empty turtle → empty string. Errors if the bbox
/// exceeds [`MAX_CANVAS_CELLS`] rather than attempting the allocation.
pub fn render(turtle: &Turtle) -> Result<String, String> {
    if turtle.cells.is_empty() {
        return Ok(String::new());
    }
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;
    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    for &(x, y) in turtle.cells.keys() {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    // i64 so a bbox spanning the full i32 range can't wrap the subtraction.
    let w64 = (max_x as i64 - min_x as i64 + 1) as u64;
    let h64 = (max_y as i64 - min_y as i64 + 1) as u64;
    let cells = w64.saturating_mul(h64);
    if cells > MAX_CANVAS_CELLS {
        return Err(format!(
            "render!: canvas {w64}×{h64} = {cells} cells exceeds {MAX_CANVAS_CELLS} \
             (the curve spans too wide a bounding box — try fewer iterations)"
        ));
    }
    let (w, h) = (w64 as usize, h64 as usize);
    let mut grid: Vec<Vec<char>> = vec![vec![' '; w]; h];
    for (&(x, y), &c) in &turtle.cells {
        grid[(y - min_y) as usize][(x - min_x) as usize] = c;
    }
    Ok(grid
        .iter()
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// `(draw! sym-list)` — walk a list of stroke symbols and dispatch each
/// to the corresponding turtle action. Returns the number of `F`/`G`
/// strokes processed (so a caller can tell whether anything was drawn).
///
/// Single ASCII letters other than `F`/`G` (e.g. `X`, `Y`, `A`, `B`) are
/// silently skipped — this is the L-system "non-terminal" convention,
/// where auxiliary symbols only exist to drive the rewrite rules and
/// don't correspond to a turtle action. Truly unknown symbols
/// (multi-char, punctuation, non-ASCII) still error so typos surface.
fn draw_prim(args: &[Val], t: &mut Turtle) -> Result<Val, String> {
    let mut cur = &args[0];
    let mut count = 0i64;
    loop {
        match cur {
            Val::Cons(head, tail) => {
                let sym = match head.as_ref() {
                    Val::Sym(s) => s,
                    other => return Err(format!("draw!: expected symbol, got {other}")),
                };
                match &**sym {
                    "F" => {
                        t.forward(true);
                        count += 1;
                    }
                    "G" => {
                        t.forward(false);
                        count += 1;
                    }
                    "+" => t.turn(1),
                    "-" => t.turn(-1),
                    "[" => t.push(),
                    "]" => t.pop()?,
                    other if is_nonterminal(other) => {}
                    other => return Err(format!("draw!: unknown stroke symbol '{other}'")),
                }
                cur = tail;
            }
            Val::Nil => break,
            other => return Err(format!("draw!: expected list, got {other}")),
        }
    }
    Ok(Val::Num(count))
}

/// True for a single-letter ASCII symbol that isn't a defined stroke.
/// Used by `draw!` to silently pass over L-system non-terminals (X, Y,
/// A, B, etc.) while still erroring on multi-char typos or punctuation.
fn is_nonterminal(s: &str) -> bool {
    let mut chars = s.chars();
    let (first, rest) = (chars.next(), chars.next());
    matches!((first, rest), (Some(c), None) if c.is_ascii_alphabetic())
}

/// `(render!)` — return the canvas as a symbol whose Display is the
/// rendered multi-line string. Wrapping in `Val::Sym` (rather than
/// adding a string type) is consistent with the engine staying
/// dep-free; the REPL's default printer surfaces the newlines
/// correctly because `Val::Sym(s) => write!(f, "{s}")` is verbatim.
fn render_prim(_args: &[Val], t: &mut Turtle) -> Result<Val, String> {
    Ok(Val::Sym(render(t)?.into()))
}

fn reset_prim(_args: &[Val], t: &mut Turtle) -> Result<Val, String> {
    t.reset();
    Ok(Val::Bool(true))
}

/// Install the turtle prims and the pure-lisp rewrite prelude. Hosts
/// own the `Rc<RefCell<Turtle>>` so they can read state directly (for
/// renderers, tests, the WASM bridge) — same shape as
/// `world::world_prim::install`.
/// Names this pack publishes to the root namespace (ADR-042). Curves
/// shares no name with the other two packs, so its whole surface is
/// public; `expand` / `expand-one` are the rewrite engine's internals
/// but the Curve Lab cheatsheet shows them, so they go out too.
pub const EXPORTS: &[&str] = &["draw!", "render!", "reset!", "grow", "expand", "expand-one"];

/// Install the L-system vocabulary into its own namespace and publish
/// [`EXPORTS`] to the root. Returns the namespace.
pub fn install(vm: &mut Vm, turtle: Rc<RefCell<Turtle>>) -> NsHandle {
    let ns = vm.namespace("curves");
    {
        let t = turtle.clone();
        vm.register_prim_in(&ns, "draw!", Arity::Exact(1), move |args| {
            let mut tt = t.borrow_mut();
            draw_prim(args, &mut tt)
        });
    }
    {
        let t = turtle.clone();
        vm.register_prim_in(&ns, "render!", Arity::Exact(0), move |args| {
            let mut tt = t.borrow_mut();
            render_prim(args, &mut tt)
        });
    }
    {
        let t = turtle.clone();
        vm.register_prim_in(&ns, "reset!", Arity::Exact(0), move |args| {
            let mut tt = t.borrow_mut();
            reset_prim(args, &mut tt)
        });
    }
    vm.eval_str_in(&ns, PRELUDE_DEFINES)
        .expect("curves prelude failed to install");
    vm.export(&ns, EXPORTS)
        .expect("curves exports collided with another pack");
    ns
}
