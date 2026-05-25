//! A tiny functional lisp built on a CEK abstract machine.
//!
//! - `expr`        — AST: literals, variables, lambdas, applications, `if`, `letrec`, quoted data
//! - `val`         — runtime values: numbers, booleans, symbols, nil, cons, closures, primitives
//! - `env`         — `Rc`-linked environment frames (immutable, structurally shared cells)
//! - `k`           — first-class continuations (the "stack" reified as data)
//! - `step`        — `step(State) -> Step` and the driver loop
//! - `prim`        — pure built-in primitives and the initial environment
//! - `world`       — minimal grid world used by the spell demo
//! - `world_prim`  — world-aware primitives that read/mutate the host world
//! - `parse`       — s-expression reader + special-form compiler

use std::cell::RefCell;
use std::rc::Rc;

pub mod env;
pub mod expr;
pub mod k;
pub mod parse;
pub mod prim;
pub mod step;
pub mod val;
pub mod world;
pub mod world_prim;

pub use env::Env;
pub use expr::Expr;
pub use step::{Step, run};
pub use val::Val;
pub use world::{Tile, World};

pub struct Vm {
    env: Env,
    pub world: Rc<RefCell<World>>,
}

impl Vm {
    pub fn new() -> Self {
        Self::with_world(World::empty())
    }

    pub fn with_world(world: World) -> Self {
        let world = Rc::new(RefCell::new(world));
        let mut env = prim::initial_env();
        for &(name, arity, f) in world_prim::WORLD_PRIMS {
            env = env.extend(name.into(), Val::WorldPrim { name, arity, f });
        }
        Vm { env, world }
    }

    pub fn eval_str(&mut self, src: &str) -> Result<Val, String> {
        let expr = parse::parse(src)?;
        run(expr, self.env.clone(), self.world.clone())
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
