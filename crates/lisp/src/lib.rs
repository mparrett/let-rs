//! A tiny functional lisp built on a CEK abstract machine.
//!
//! - `expr`  — AST: literals, variables, lambdas, applications, `if`
//! - `val`   — runtime values: numbers, booleans, closures, primitives
//! - `env`   — `Rc`-linked environment frames (immutable, structurally shared)
//! - `k`     — first-class continuations (the "stack" reified as data)
//! - `step`  — `step(State) -> Step` and the driver loop
//! - `prim`  — built-in primitives and the initial environment
//! - `parse` — s-expression reader + special-form compiler

pub mod env;
pub mod expr;
pub mod k;
pub mod parse;
pub mod prim;
pub mod step;
pub mod val;

pub use env::Env;
pub use expr::Expr;
pub use step::{Step, run};
pub use val::Val;

pub struct Vm {
    env: Env,
}

impl Vm {
    pub fn new() -> Self {
        Vm { env: prim::initial_env() }
    }

    pub fn eval_str(&mut self, src: &str) -> Result<Val, String> {
        let expr = parse::parse(src)?;
        run(expr, self.env.clone())
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
