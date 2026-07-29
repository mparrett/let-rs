use std::rc::Rc;

use crate::env::Env;
use crate::error::{LispErr, Span};
use crate::expr::Expr;
use crate::k::K;
use crate::val::Val;

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

pub fn step(s: State) -> Result<Step, LispErr> {
    match s.mode {
        Mode::Eval(c, env) => eval_expr(c, env, s.k),
        Mode::Apply(v) => apply_k(v, s.k),
    }
}

fn eval_expr(c: Rc<Expr>, env: Env, k: Rc<K>) -> Result<Step, LispErr> {
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
        Expr::Var(name, span) => {
            let v = env
                .lookup(name)
                .ok_or_else(|| LispErr::maybe_at(format!("unbound variable: {name}"), *span))?;
            Ok(Step::Continue(State {
                mode: Mode::Apply(v),
                k,
            }))
        }
        Expr::Lam(params, body) => {
            let clo = Val::Clo {
                params: Rc::clone(params),
                body: body.clone(),
                env: env.clone(),
            };
            Ok(Step::Continue(State {
                mode: Mode::Apply(clo),
                k,
            }))
        }
        Expr::If(cond, t, e) => {
            let new_k = Rc::new(K::If {
                then_branch: t.clone(),
                else_branch: e.clone(),
                env: env.clone(),
                k,
            });
            Ok(Step::Continue(State {
                mode: Mode::Eval(cond.clone(), env),
                k: new_k,
            }))
        }
        Expr::App(exprs, span) => {
            if exprs.is_empty() {
                return Err(LispErr::maybe_at("empty application", *span));
            }
            // Share the subexpression slice with the K rather than
            // copying it out; only `evaled` needs to be owned.
            let args = Rc::clone(exprs);
            let first = Rc::clone(&args[0]);
            let new_k = Rc::new(K::App {
                evaled: Vec::with_capacity(args.len()),
                args,
                env: env.clone(),
                span: *span,
                k,
            });
            Ok(Step::Continue(State {
                mode: Mode::Eval(first, env),
                k: new_k,
            }))
        }
        Expr::SetBang(name, val) => {
            let new_k = Rc::new(K::SetBang {
                name: name.clone(),
                env: env.clone(),
                k,
            });
            Ok(Step::Continue(State {
                mode: Mode::Eval(val.clone(), env),
                k: new_k,
            }))
        }
        Expr::Letrec(bindings, body) => {
            // Allocate placeholders for all names, then start evaluating the first init
            // in the recursive environment. With no bindings, just eval the body.
            // Post-ADR-023: placeholders live in the store; the frame just carries
            // the `Addr`, so closures capturing this env can no longer Rc-reach back
            // to their own cells.
            let mut env_rec = env.clone();
            let mut addrs = Vec::with_capacity(bindings.len());
            for (name, _) in bindings {
                let (next_env, addr) = env_rec.extend_placeholder(name.clone());
                env_rec = next_env;
                addrs.push(addr);
            }

            if bindings.is_empty() {
                return Ok(Step::Continue(State {
                    mode: Mode::Eval(body.clone(), env_rec),
                    k,
                }));
            }

            let inits: Rc<[Rc<Expr>]> = bindings.iter().map(|(_, e)| e.clone()).collect();
            let first = Rc::clone(&inits[0]);
            let new_k = Rc::new(K::Letrec {
                addrs: addrs.into(),
                inits,
                next: 0,
                body: body.clone(),
                env: env_rec.clone(),
                k,
            });
            Ok(Step::Continue(State {
                mode: Mode::Eval(first, env_rec),
                k: new_k,
            }))
        }
    }
}

fn apply_k(v: Val, k: Rc<K>) -> Result<Step, LispErr> {
    // Take the continuation by value. This engine has no first-class
    // continuations — each `K` is owned by exactly one `State` or one
    // child `K`, and the frame we're consuming here is at the end of
    // that chain — so `try_unwrap` succeeds and every field below can
    // be *moved* out. Pre-ADR-035 this matched on `&*k` and cloned:
    // `evaled` and `remaining` were copied on every single argument,
    // making an n-argument call O(n²) in `Val` clones.
    //
    // The clone arm is a correctness fallback, not a hot path. If
    // `call/cc` or continuation marks ever land, `K`s become shared
    // and this keeps working (more slowly) rather than misbehaving.
    let k = match Rc::try_unwrap(k) {
        Ok(owned) => owned,
        Err(shared) => (*shared).clone(),
    };

    match k {
        K::Halt => Ok(Step::Done(v)),

        K::App {
            mut evaled,
            args,
            env,
            span,
            k: outer,
        } => {
            evaled.push(v);
            // `evaled` is filled left to right, so its length is the
            // index of the next subexpression to evaluate.
            if evaled.len() == args.len() {
                apply(evaled, span, outer)
            } else {
                let next = Rc::clone(&args[evaled.len()]);
                let new_k = Rc::new(K::App {
                    evaled,
                    args,
                    env: env.clone(),
                    span,
                    k: outer,
                });
                Ok(Step::Continue(State {
                    mode: Mode::Eval(next, env),
                    k: new_k,
                }))
            }
        }

        K::If {
            then_branch,
            else_branch,
            env,
            k: outer,
        } => {
            let branch = if v.is_truthy() {
                then_branch
            } else {
                else_branch
            };
            Ok(Step::Continue(State {
                mode: Mode::Eval(branch, env),
                k: outer,
            }))
        }

        K::SetBang {
            name,
            env,
            k: outer,
        } => {
            env.set(&name, v.clone())?;
            Ok(Step::Continue(State {
                mode: Mode::Apply(v),
                k: outer,
            }))
        }

        K::Letrec {
            addrs,
            inits,
            next,
            body,
            env,
            k: outer,
        } => {
            let store = env
                .store_handle()
                .expect("store dropped during letrec patch");
            store.set(addrs[next], v);
            let next = next + 1;
            if next == inits.len() {
                Ok(Step::Continue(State {
                    mode: Mode::Eval(body, env),
                    k: outer,
                }))
            } else {
                let next_expr = Rc::clone(&inits[next]);
                let new_k = Rc::new(K::Letrec {
                    addrs,
                    inits,
                    next,
                    body,
                    env: env.clone(),
                    k: outer,
                });
                Ok(Step::Continue(State {
                    mode: Mode::Eval(next_expr, env),
                    k: new_k,
                }))
            }
        }
    }
}

/// Apply the callee in `evaled[0]` to the rest. `span` is the call site,
/// used for every error raised here: an arity mismatch, a non-callable
/// head, and — the important one — whatever the prim itself returns.
/// Prims keep the `Result<Val, String>` signature that host crates
/// implement against (`PrimFn`), so they have no way to report a
/// position; the position that helps is the call site anyway, and only
/// the machine knows it. `with_span` rather than `at`, so if a prim ever
/// does grow a way to report its own span, the inner one wins.
fn apply(mut evaled: Vec<Val>, span: Option<Span>, k: Rc<K>) -> Result<Step, LispErr> {
    // Shift the callee off the front and reuse the same allocation for
    // the argument vector, rather than draining into a second one.
    assert!(!evaled.is_empty(), "apply with no fn");
    let f = evaled.remove(0);
    let args = evaled;
    // Borrow rather than move out of `f`: `Val` implements `Drop` (to
    // dismantle cons spines iteratively), which forbids by-value
    // destructuring. Clo/Prim aren't cons cells, so their `Drop` is a
    // no-op; the only cost here is a few cheap `Rc` clones per call.
    match &f {
        Val::Clo { params, body, env } => {
            if params.len() != args.len() {
                return Err(LispErr::maybe_at(
                    format!(
                        "arity: closure expected {}, got {}",
                        params.len(),
                        args.len()
                    ),
                    span,
                ));
            }
            let env = env.extend_many(params.iter().cloned().zip(args));
            // Tail-call note: we pass `k` through unchanged. No frame pushed for
            // entering the closure body, so a tail call grows nothing.
            Ok(Step::Continue(State {
                mode: Mode::Eval(Rc::clone(body), env),
                k,
            }))
        }
        Val::Prim {
            name,
            arity,
            f: prim,
        } => {
            if !arity.accepts(args.len()) {
                return Err(LispErr::maybe_at(
                    format!("{name}: arity {}, got {}", arity.describe(), args.len()),
                    span,
                ));
            }
            let v = prim(&args).map_err(|m| LispErr::new(m).with_span(span))?;
            Ok(Step::Continue(State {
                mode: Mode::Apply(v),
                k,
            }))
        }
        other => Err(LispErr::maybe_at(format!("not callable: {other}"), span)),
    }
}

pub fn run(expr: Expr, env: Env) -> Result<Val, LispErr> {
    run_bounded(expr, env, u64::MAX)
}

/// Like `run`, but errors after `budget` CEK steps. `u64::MAX` is
/// effectively unbounded. The budget guards against nonterminating
/// expressions in hosted environments (REPL, WASM bridge) where the
/// caller can't otherwise interrupt evaluation.
pub fn run_bounded(expr: Expr, env: Env, budget: u64) -> Result<Val, LispErr> {
    let mut s = State {
        mode: Mode::Eval(Rc::new(expr), env),
        k: Rc::new(K::Halt),
    };
    let mut steps_left = budget;
    loop {
        if steps_left == 0 {
            return Err(LispErr::new("execution exceeded step budget"));
        }
        steps_left -= 1;
        match step(s)? {
            Step::Continue(next) => s = next,
            Step::Done(v) => return Ok(v),
        }
    }
}
