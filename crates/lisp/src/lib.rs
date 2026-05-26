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
//!
//! Macros: `defmacro` is a top-level form that registers a procedural macro.
//! Each subsequent `eval_str` expands macro calls in the source (pre-compile)
//! by evaluating the macro's closure against quoted arg datums and recursively
//! re-expanding the result. Two arg conventions:
//!
//! - `(defmacro name (a b c) body)` — fixed arity, named params
//! - `(defmacro name args body)` — variadic; `args` is bound to the full list
//!
//! Quasiquote (`\``, `,`, `,@`) is built in. Macros are *not* hygienic — write
//! symbols in macro bodies that don't shadow user code.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod env;
pub mod expr;
pub mod genes;
pub mod k;
pub mod parse;
pub mod prim;
pub mod step;
pub mod val;
pub mod world;
pub mod world_prim;

pub use env::Env;
pub use expr::{Expr, Sym};
pub use parse::Datum;
pub use step::{Step, run};
pub use val::Val;
pub use world::{Tile, World};

#[derive(Clone)]
struct Macro {
    closure: Val,
    /// True for `(defmacro name args body)` (single symbol after the name) —
    /// the macro receives all call-site args bundled as one list. False for
    /// `(defmacro name (a b c) body)` — fixed arity, args passed positionally.
    variadic: bool,
}

pub struct Vm {
    env: Env,
    pub world: Rc<RefCell<World>>,
    macros: Rc<RefCell<HashMap<String, Macro>>>,
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
        Vm {
            env,
            world,
            macros: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// Install a host primitive into the VM's initial env. Mirrors how
    /// `world_prim::WORLD_PRIMS` is installed in `with_world`, but for
    /// pure (non-world-touching) prims so demo binaries can extend the
    /// vocabulary without going through `eval_str`.
    pub fn register_prim(
        &mut self,
        name: &'static str,
        arity: val::Arity,
        f: fn(&[Val]) -> Result<Val, String>,
    ) {
        self.env = self
            .env
            .extend(name.into(), Val::Prim { name, arity, f });
    }

    pub fn eval_str(&mut self, src: &str) -> Result<Val, String> {
        let datum = parse::read(src)?;

        if self.try_register_defmacro(&datum)? {
            return Ok(Val::Bool(true));
        }

        let expanded = self.expand_all(datum)?;
        let expr = parse::compile(&expanded)?;
        run(expr, self.env.clone(), self.world.clone())
    }

    fn try_register_defmacro(&mut self, d: &Datum) -> Result<bool, String> {
        let items = match d {
            Datum::List(items) => items,
            _ => return Ok(false),
        };
        match items.first() {
            Some(Datum::Sym(s)) if &**s == "defmacro" => {}
            _ => return Ok(false),
        }
        if items.len() != 4 {
            return Err("defmacro: expected (defmacro name params body)".into());
        }
        let name: Sym = match &items[1] {
            Datum::Sym(s) => s.clone(),
            _ => return Err("defmacro: name must be a symbol".into()),
        };
        let (params, variadic): (Vec<Sym>, bool) = match &items[2] {
            Datum::Sym(s) => (vec![s.clone()], true),
            Datum::List(ps) => {
                let names: Result<Vec<Sym>, String> = ps
                    .iter()
                    .map(|p| match p {
                        Datum::Sym(s) => Ok(s.clone()),
                        _ => Err("defmacro: param must be a symbol".into()),
                    })
                    .collect();
                (names?, false)
            }
            _ => return Err("defmacro: params must be a symbol or a list".into()),
        };
        // Expand macros inside the body so macros can use other macros.
        let body_datum = self.expand_all(items[3].clone())?;
        let body_expr = parse::compile(&body_datum)?;
        let closure = Val::Clo {
            params,
            body: Rc::new(body_expr),
            env: self.env.clone(),
        };
        self.macros
            .borrow_mut()
            .insert(name.to_string(), Macro { closure, variadic });
        Ok(true)
    }

    fn expand_all(&mut self, d: Datum) -> Result<Datum, String> {
        if let Datum::List(items) = &d
            && !items.is_empty()
            && let Datum::Sym(head) = &items[0]
        {
            let name = head.clone();
            let name_str = &*name;

            // Opaque: contents are data.
            if name_str == "quote" {
                return Ok(d);
            }
            // Quasiquote: descend, but unquoted parts get full macro expansion.
            if name_str == "quasiquote" && items.len() == 2 {
                let inside = self.expand_in_qq(items[1].clone(), 1)?;
                return Ok(Datum::List(vec![items[0].clone(), inside]));
            }
            // Lambda: don't macro-expand the params list (it's symbols).
            if (name_str == "lambda" || name_str == "λ") && items.len() >= 3 {
                let mut out = vec![items[0].clone(), items[1].clone()];
                for i in &items[2..] {
                    out.push(self.expand_all(i.clone())?);
                }
                return Ok(Datum::List(out));
            }
            // Let-family: don't macro-expand binding-name positions.
            if matches!(name_str, "let" | "let*" | "letrec") && items.len() >= 3 {
                let bindings_out = match &items[1] {
                    Datum::List(pairs) => {
                        let mut new_pairs = Vec::with_capacity(pairs.len());
                        for p in pairs {
                            if let Datum::List(pair) = p
                                && pair.len() == 2
                            {
                                new_pairs.push(Datum::List(vec![
                                    pair[0].clone(),
                                    self.expand_all(pair[1].clone())?,
                                ]));
                            } else {
                                new_pairs.push(p.clone());
                            }
                        }
                        Datum::List(new_pairs)
                    }
                    other => other.clone(),
                };
                let mut out = vec![items[0].clone(), bindings_out];
                for i in &items[2..] {
                    out.push(self.expand_all(i.clone())?);
                }
                return Ok(Datum::List(out));
            }
            // Defmacro inside other code: refuse so silent misregistration
            // doesn't bite us.
            if name_str == "defmacro" {
                return Err("defmacro only valid at top level".into());
            }

            // Macro lookup
            let mac = self.macros.borrow().get(name_str).cloned();
            if let Some(m) = mac {
                let expansion = self.expand_macro_call(&m, &items[1..])?;
                return self.expand_all(expansion);
            }
        }

        match d {
            Datum::List(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for i in items {
                    new_items.push(self.expand_all(i)?);
                }
                Ok(Datum::List(new_items))
            }
            other => Ok(other),
        }
    }

    fn expand_in_qq(&mut self, d: Datum, depth: usize) -> Result<Datum, String> {
        if depth == 0 {
            return self.expand_all(d);
        }
        if let Datum::List(items) = &d
            && !items.is_empty()
        {
            if let Datum::Sym(s) = &items[0] {
                let name = &**s;
                if name == "quasiquote" && items.len() == 2 {
                    let inside = self.expand_in_qq(items[1].clone(), depth + 1)?;
                    return Ok(Datum::List(vec![items[0].clone(), inside]));
                }
                if (name == "unquote" || name == "unquote-splicing") && items.len() == 2 {
                    let inside = self.expand_in_qq(items[1].clone(), depth - 1)?;
                    return Ok(Datum::List(vec![items[0].clone(), inside]));
                }
            }
            let mut new_items = Vec::with_capacity(items.len());
            for i in items {
                new_items.push(self.expand_in_qq(i.clone(), depth)?);
            }
            return Ok(Datum::List(new_items));
        }
        Ok(d)
    }

    fn expand_macro_call(&mut self, m: &Macro, raw_args: &[Datum]) -> Result<Datum, String> {
        let arg_vals: Vec<Val> = raw_args.iter().map(parse::datum_to_val).collect();
        let args_to_pass = if m.variadic {
            vec![Val::list_from(&arg_vals)]
        } else {
            arg_vals
        };
        let result = self.call_value(&m.closure, args_to_pass)?;
        val_to_datum(&result)
    }

    fn call_value(&self, f: &Val, args: Vec<Val>) -> Result<Val, String> {
        let mut app: Vec<Rc<Expr>> = vec![Rc::new(Expr::Quote(Rc::new(f.clone())))];
        for a in args {
            app.push(Rc::new(Expr::Quote(Rc::new(a))));
        }
        run(Expr::App(app), self.env.clone(), self.world.clone())
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

fn val_to_datum(v: &Val) -> Result<Datum, String> {
    match v {
        Val::Num(n) => Ok(Datum::Num(*n)),
        Val::Bool(b) => Ok(Datum::Bool(*b)),
        Val::Sym(s) => Ok(Datum::Sym(s.clone())),
        Val::Nil => Ok(Datum::List(vec![])),
        Val::Cons(_, _) => {
            let mut items = Vec::new();
            let mut cur = v;
            loop {
                match cur {
                    Val::Cons(h, t) => {
                        items.push(val_to_datum(h)?);
                        cur = t;
                    }
                    Val::Nil => break,
                    other => return Err(format!("non-proper list in macro expansion: {other}")),
                }
            }
            Ok(Datum::List(items))
        }
        other => Err(format!("can't convert {other} back to a datum")),
    }
}
