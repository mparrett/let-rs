use std::cell::RefCell;
use std::rc::Rc;

use crate::env::Env;
use crate::expr::Expr;
use crate::k::K;
use crate::val::Val;
use crate::world::World;

pub enum Mode {
    Eval(Rc<Expr>, Env),
    Apply(Val),
}

pub struct State {
    pub mode: Mode,
    pub k: Rc<K>,
}

pub enum Step {
    Continue(State),
    Done(Val),
}

pub fn step(s: State, world: &Rc<RefCell<World>>) -> Result<Step, String> {
    match s.mode {
        Mode::Eval(c, env) => eval_expr(c, env, s.k),
        Mode::Apply(v) => apply_k(v, s.k, world),
    }
}

fn eval_expr(c: Rc<Expr>, env: Env, k: Rc<K>) -> Result<Step, String> {
    match &*c {
        Expr::Num(n) => Ok(Step::Continue(State {
            mode: Mode::Apply(Val::Num(*n)),
            k,
        })),
        Expr::Bool(b) => Ok(Step::Continue(State {
            mode: Mode::Apply(Val::Bool(*b)),
            k,
        })),
        Expr::Quote(v) => Ok(Step::Continue(State {
            mode: Mode::Apply((**v).clone()),
            k,
        })),
        Expr::Var(name) => {
            let v = env.lookup(name).ok_or_else(|| format!("unbound variable: {name}"))?;
            Ok(Step::Continue(State { mode: Mode::Apply(v), k }))
        }
        Expr::Lam(params, body) => {
            let clo = Val::Clo {
                params: params.clone(),
                body: body.clone(),
                env: env.clone(),
            };
            Ok(Step::Continue(State { mode: Mode::Apply(clo), k }))
        }
        Expr::If(cond, t, e) => {
            let new_k = Rc::new(K::If {
                then_branch: t.clone(),
                else_branch: e.clone(),
                env: env.clone(),
                k,
            });
            Ok(Step::Continue(State { mode: Mode::Eval(cond.clone(), env), k: new_k }))
        }
        Expr::App(exprs) => {
            if exprs.is_empty() {
                return Err("empty application".into());
            }
            let mut remaining: Vec<Rc<Expr>> = exprs.to_vec();
            let first = remaining.remove(0);
            let new_k = Rc::new(K::App {
                evaled: Vec::with_capacity(exprs.len()),
                remaining,
                env: env.clone(),
                k,
            });
            Ok(Step::Continue(State { mode: Mode::Eval(first, env), k: new_k }))
        }
        Expr::Letrec(bindings, body) => {
            // Allocate placeholders for all names, then start evaluating the first init
            // in the recursive environment. With no bindings, just eval the body.
            let mut env_rec = env.clone();
            let mut cells = Vec::with_capacity(bindings.len());
            for (name, _) in bindings {
                let (next_env, slot) = env_rec.extend_placeholder(name.clone());
                env_rec = next_env;
                cells.push(slot);
            }

            if bindings.is_empty() {
                return Ok(Step::Continue(State { mode: Mode::Eval(body.clone(), env_rec), k }));
            }

            let mut remaining: Vec<Rc<Expr>> = bindings.iter().map(|(_, e)| e.clone()).collect();
            let first = remaining.remove(0);
            let new_k = Rc::new(K::Letrec {
                cells,
                next: 0,
                remaining,
                body: body.clone(),
                env: env_rec.clone(),
                k,
            });
            Ok(Step::Continue(State { mode: Mode::Eval(first, env_rec), k: new_k }))
        }
    }
}

fn apply_k(v: Val, k: Rc<K>, world: &Rc<RefCell<World>>) -> Result<Step, String> {
    match &*k {
        K::Halt => Ok(Step::Done(v)),

        K::App { evaled, remaining, env, k: outer } => {
            let mut evaled = evaled.clone();
            evaled.push(v);
            if remaining.is_empty() {
                apply(evaled, outer.clone(), world)
            } else {
                let mut remaining = remaining.clone();
                let next = remaining.remove(0);
                let env = env.clone();
                let new_k = Rc::new(K::App {
                    evaled,
                    remaining,
                    env: env.clone(),
                    k: outer.clone(),
                });
                Ok(Step::Continue(State { mode: Mode::Eval(next, env), k: new_k }))
            }
        }

        K::If { then_branch, else_branch, env, k: outer } => {
            let branch = if v.is_truthy() { then_branch.clone() } else { else_branch.clone() };
            Ok(Step::Continue(State {
                mode: Mode::Eval(branch, env.clone()),
                k: outer.clone(),
            }))
        }

        K::Letrec { cells, next, remaining, body, env, k: outer } => {
            *cells[*next].borrow_mut() = v;
            if remaining.is_empty() {
                Ok(Step::Continue(State {
                    mode: Mode::Eval(body.clone(), env.clone()),
                    k: outer.clone(),
                }))
            } else {
                let mut remaining = remaining.clone();
                let next_expr = remaining.remove(0);
                let new_k = Rc::new(K::Letrec {
                    cells: cells.clone(),
                    next: next + 1,
                    remaining,
                    body: body.clone(),
                    env: env.clone(),
                    k: outer.clone(),
                });
                Ok(Step::Continue(State {
                    mode: Mode::Eval(next_expr, env.clone()),
                    k: new_k,
                }))
            }
        }
    }
}

fn apply(evaled: Vec<Val>, k: Rc<K>, world: &Rc<RefCell<World>>) -> Result<Step, String> {
    let mut it = evaled.into_iter();
    let f = it.next().expect("apply with no fn");
    let args: Vec<Val> = it.collect();
    match f {
        Val::Clo { params, body, env } => {
            if params.len() != args.len() {
                return Err(format!(
                    "arity: closure expected {}, got {}",
                    params.len(),
                    args.len()
                ));
            }
            let env = env.extend_many(params.into_iter().zip(args));
            // Tail-call note: we pass `k` through unchanged. No frame pushed for
            // entering the closure body, so a tail call grows nothing.
            Ok(Step::Continue(State { mode: Mode::Eval(body, env), k }))
        }
        Val::Prim { name, arity, f } => {
            if !arity.accepts(args.len()) {
                return Err(format!(
                    "{name}: arity {}, got {}",
                    arity.describe(),
                    args.len()
                ));
            }
            let v = f(&args)?;
            Ok(Step::Continue(State { mode: Mode::Apply(v), k }))
        }
        Val::WorldPrim { name, arity, f } => {
            if !arity.accepts(args.len()) {
                return Err(format!(
                    "{name}: arity {}, got {}",
                    arity.describe(),
                    args.len()
                ));
            }
            let v = {
                let mut w = world.borrow_mut();
                f(&args, &mut w)?
            };
            Ok(Step::Continue(State { mode: Mode::Apply(v), k }))
        }
        other => Err(format!("not callable: {other}")),
    }
}

pub fn run(expr: Expr, env: Env, world: Rc<RefCell<World>>) -> Result<Val, String> {
    run_bounded(expr, env, world, u64::MAX)
}

/// Like `run`, but errors after `budget` CEK steps. `u64::MAX` is
/// effectively unbounded. The budget guards against nonterminating
/// expressions in hosted environments (REPL, WASM bridge) where the
/// caller can't otherwise interrupt evaluation.
pub fn run_bounded(
    expr: Expr,
    env: Env,
    world: Rc<RefCell<World>>,
    budget: u64,
) -> Result<Val, String> {
    let mut s = State {
        mode: Mode::Eval(Rc::new(expr), env),
        k: Rc::new(K::Halt),
    };
    let mut steps_left = budget;
    loop {
        if steps_left == 0 {
            return Err("execution exceeded step budget".into());
        }
        steps_left -= 1;
        match step(s, &world)? {
            Step::Continue(next) => s = next,
            Step::Done(v) => return Ok(v),
        }
    }
}
