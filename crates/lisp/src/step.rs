use std::rc::Rc;

use crate::env::Env;
use crate::error::{LispErr, Span};
use crate::expr::Expr;
use crate::k::K;
use crate::val::Val;

pub enum Mode {
    Eval(Rc<Expr>, Env),
    Apply(Val),
    /// A condition travelling *outward*, discarding continuation frames
    /// until it reaches a `K::Guard` or the top (ADR-041).
    ///
    /// Every runtime failure enters this mode — a prim's complaint, an
    /// unbound variable, a bad arity — which is what makes them all
    /// catchable through one path rather than two. The exception is the
    /// step budget, which lives in `Machine::run` and never becomes a
    /// condition: a guard that could swallow it would make a runaway
    /// loop unkillable.
    ///
    /// The `Span` rides along unused unless the condition escapes to the
    /// top, where it becomes the position on the resulting `LispErr`
    /// (ADR-039).
    Raise(Val, Option<Span>),
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
        Mode::Raise(v, span) => unwind(v, span, s.k),
    }
}

/// Enter the raise mode with `cond` as the condition. Every runtime
/// error site funnels through here rather than returning `Err`, so
/// there's one definition of what a guard can catch.
fn raise(cond: Val, span: Option<Span>, k: Rc<K>) -> Result<Step, LispErr> {
    Ok(Step::Continue(State {
        mode: Mode::Raise(cond, span),
        k,
    }))
}

/// A condition from a message, in the shape `error` builds:
/// `(error "…")`. Engine failures and prim failures both arrive as
/// strings, so both become this — which is why `(guard (e …) (car '()))`
/// catches a prim error with no cooperation from the prim.
pub(crate) fn condition_from_msg(msg: impl Into<Rc<str>>) -> Val {
    Val::list_from(&[Val::Sym("error".into()), Val::Str(msg.into())])
}

/// Discard one continuation frame, or hand the condition to a guard.
///
/// One frame per step, rather than looping to the handler: unwinding is
/// then interruptible and budget-counted like everything else, and a
/// paused machine mid-unwind reports a shrinking `depth` (ADR-040).
/// Dropping each frame also drops the `Env` it held, so the store slots
/// those bindings owned come back through `Frame::drop` (ADR-033) —
/// unwinding reclaims as it goes, with no special handling.
fn unwind(cond: Val, span: Option<Span>, k: Rc<K>) -> Result<Step, LispErr> {
    let k = match Rc::try_unwrap(k) {
        Ok(owned) => owned,
        Err(shared) => (*shared).clone(),
    };
    match k {
        // Nothing caught it. Now it becomes a Rust-side error, carrying
        // the raise site's position.
        K::Halt => Err(LispErr::maybe_at(describe_condition(&cond), span)),

        K::Guard {
            var,
            handler,
            env,
            k: outer,
        } => {
            let env = env.extend_many(std::iter::once((var, cond)));
            Ok(Step::Continue(State {
                mode: Mode::Eval(handler, env),
                k: outer,
            }))
        }

        // Everything else is pending work that will never happen.
        K::App { k: outer, .. }
        | K::If { k: outer, .. }
        | K::Letrec { k: outer, .. }
        | K::SetBang { k: outer, .. }
        | K::Raise { k: outer, .. } => Ok(Step::Continue(State {
            mode: Mode::Raise(cond, span),
            k: outer,
        })),
    }
}

/// Render an escaped condition as an error message. `(error "msg")` —
/// the shape the engine and `error` produce — reads back as just `msg`,
/// so an uncaught prim failure looks exactly as it did before conditions
/// existed. Anything else a user chose to `raise` is printed whole.
fn describe_condition(cond: &Val) -> String {
    if let Val::Cons(head, tail) = cond
        && matches!(&**head, Val::Sym(s) if &**s == "error")
        && let Val::Cons(msg, rest) = &**tail
    {
        let irritants = match &**rest {
            Val::Nil => String::new(),
            other => format!(" {other}"),
        };
        if let Val::Str(m) = &**msg {
            return format!("{m}{irritants}");
        }
        return format!("{msg}{irritants}");
    }
    format!("raised: {cond}")
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
        Expr::Var(name, span) => match env.lookup(name) {
            Some(v) => Ok(Step::Continue(State {
                mode: Mode::Apply(v),
                k,
            })),
            None => raise(
                condition_from_msg(format!("unbound variable: {name}")),
                *span,
                k,
            ),
        },
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
                return raise(condition_from_msg("empty application"), *span, k);
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
        Expr::Raise(inner, span) => {
            let new_k = Rc::new(K::Raise { span: *span, k });
            Ok(Step::Continue(State {
                mode: Mode::Eval(Rc::clone(inner), env),
                k: new_k,
            }))
        }
        Expr::Guard { var, handler, body } => {
            let new_k = Rc::new(K::Guard {
                var: var.clone(),
                handler: Rc::clone(handler),
                env: env.clone(),
                k,
            });
            Ok(Step::Continue(State {
                mode: Mode::Eval(Rc::clone(body), env),
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
        } => match env.set(&name, v.clone()) {
            Ok(()) => Ok(Step::Continue(State {
                mode: Mode::Apply(v),
                k: outer,
            })),
            Err(msg) => raise(condition_from_msg(msg), None, outer),
        },

        // The condition expression finished evaluating; now it raises.
        K::Raise { span, k: outer } => raise(v, span, outer),

        // The body finished without raising, so the guard has no work:
        // drop the frame and keep the value moving outward.
        K::Guard { k: outer, .. } => Ok(Step::Continue(State {
            mode: Mode::Apply(v),
            k: outer,
        })),

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
                return raise(
                    condition_from_msg(format!(
                        "arity: closure expected {}, got {}",
                        params.len(),
                        args.len()
                    )),
                    span,
                    k,
                );
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
                return raise(
                    condition_from_msg(format!(
                        "{name}: arity {}, got {}",
                        arity.describe(),
                        args.len()
                    )),
                    span,
                    k,
                );
            }
            // A prim reports failure as a `String` — `PrimFn`'s signature
            // is the interface every host crate implements against, and
            // ADR-039 kept it that way. Turning that string into a
            // condition here is what makes host prims catchable without
            // any host knowing conditions exist.
            match prim(&args) {
                Ok(v) => Ok(Step::Continue(State {
                    mode: Mode::Apply(v),
                    k,
                })),
                Err(msg) => raise(condition_from_msg(msg), span, k),
            }
        }
        other => raise(
            condition_from_msg(format!("not callable: {other}")),
            span,
            k,
        ),
    }
}

/// What a bounded run did with the steps it was given.
#[derive(Debug)]
pub enum Progress {
    /// Evaluation finished and produced this value.
    Done(Val),
    /// The step allowance ran out with work still to do. The machine is
    /// unchanged and can be resumed.
    Paused,
}

/// A CEK machine you can stop and restart.
///
/// The engine has always been a state machine whose entire state is one
/// `State` value — that's what a CEK machine *is* — but the only way to
/// drive it was a loop that ran to completion or returned an error, so
/// running out of budget meant `Err("execution exceeded step budget")`
/// and the work was gone. A `Machine` hands the loop to the caller
/// instead, which is what makes the substrate useful rather than merely
/// tidy: a host can evaluate in slices without blocking (the browser
/// isn't allowed to block), single-step for a debugger, or abandon a
/// runaway computation and keep the `Vm`.
///
/// ```
/// # use lisp::{Vm, step::{Machine, Progress}, parse};
/// let vm = Vm::new();
/// let expr = parse::parse("(+ 1 2)").unwrap();
/// let mut m = Machine::new(expr, vm.env().clone());
/// loop {
///     match m.run(4).unwrap() {
///         Progress::Done(v) => { assert_eq!(format!("{v}"), "3"); break }
///         Progress::Paused => continue,
///     }
/// }
/// ```
///
/// **No post-mortem inspection.** The introspection below works while a
/// machine is *paused*, not after it errors: `step` consumes its `State`
/// by value, and retaining a copy per step would make every `K` shared
/// and silently defeat ADR-035's `Rc::try_unwrap` fast path — turning
/// n-argument application back into O(n²). Errors carry a span (ADR-039);
/// that's the position information, and it costs nothing.
pub struct Machine {
    /// `None` once evaluation has finished.
    state: Option<State>,
    steps: u64,
}

impl Machine {
    pub fn new(expr: Expr, env: Env) -> Machine {
        Machine {
            state: Some(State {
                mode: Mode::Eval(Rc::new(expr), env),
                k: Rc::new(K::Halt),
            }),
            steps: 0,
        }
    }

    /// Take at most `budget` steps. `u64::MAX` runs to completion.
    ///
    /// Returns `Paused` if the allowance runs out first — not an error:
    /// the caller decides whether to resume, and how soon.
    pub fn run(&mut self, budget: u64) -> Result<Progress, LispErr> {
        let mut left = budget;
        while left > 0 {
            left -= 1;
            if let Progress::Done(v) = self.step_once()? {
                return Ok(Progress::Done(v));
            }
        }
        Ok(Progress::Paused)
    }

    /// Take exactly one CEK transition. The unit a stepping debugger
    /// advances by.
    pub fn step_once(&mut self) -> Result<Progress, LispErr> {
        let s = self
            .state
            .take()
            .ok_or_else(|| LispErr::new("machine has already finished"))?;
        self.steps += 1;
        match step(s)? {
            Step::Continue(next) => {
                self.state = Some(next);
                Ok(Progress::Paused)
            }
            Step::Done(v) => Ok(Progress::Done(v)),
        }
    }

    /// Transitions taken so far, across every slice.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn is_done(&self) -> bool {
        self.state.is_none()
    }

    /// Depth of the continuation chain — how much work is stacked up
    /// waiting on the current expression. `0` means the next value
    /// produced is the answer.
    ///
    /// Note this counts *all* pending frames, not just calls: `if` and
    /// `letrec` push frames too. It's the machine's own notion of depth,
    /// which is why tail calls visibly don't grow it.
    pub fn depth(&self) -> usize {
        let Some(s) = &self.state else { return 0 };
        let mut n = 0;
        let mut k = &s.k;
        loop {
            match &**k {
                K::Halt => return n,
                K::App { k: outer, .. }
                | K::If { k: outer, .. }
                | K::Letrec { k: outer, .. }
                | K::SetBang { k: outer, .. }
                | K::Raise { k: outer, .. }
                | K::Guard { k: outer, .. } => {
                    n += 1;
                    k = outer;
                }
            }
        }
    }

    /// Where in the source the machine is, when that's known. `Var` and
    /// `App` are the only expressions carrying a position (ADR-039), so
    /// this is `None` while the machine sits on a literal or is handing a
    /// value back to a continuation.
    pub fn position(&self) -> Option<Span> {
        match &self.state.as_ref()?.mode {
            Mode::Eval(e, _) => match &**e {
                Expr::Var(_, span) | Expr::App(_, span) | Expr::Raise(_, span) => *span,
                _ => None,
            },
            // A condition in flight reports where it was raised, which is
            // the position a debugger wants while watching an unwind.
            Mode::Raise(_, span) => *span,
            Mode::Apply(_) => None,
        }
    }

    /// The value the machine is about to hand to its continuation, or
    /// `None` while it's still evaluating.
    pub fn value(&self) -> Option<&Val> {
        match &self.state.as_ref()?.mode {
            Mode::Apply(v) | Mode::Raise(v, _) => Some(v),
            Mode::Eval(_, _) => None,
        }
    }

    /// Enclosing call sites, innermost first — a backtrace, read straight
    /// off the continuation chain. Only `K::App` frames have positions,
    /// so `if`/`letrec` frames are skipped rather than reported blank.
    pub fn backtrace(&self) -> Vec<Span> {
        let Some(s) = &self.state else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut k = &s.k;
        loop {
            match &**k {
                K::Halt => return out,
                K::App { span, k: outer, .. } => {
                    out.extend(*span);
                    k = outer;
                }
                K::If { k: outer, .. }
                | K::Letrec { k: outer, .. }
                | K::SetBang { k: outer, .. }
                | K::Raise { k: outer, .. }
                | K::Guard { k: outer, .. } => k = outer,
            }
        }
    }
}

/// Progress, not contents: `State` holds an `Env`, which has no `Debug`
/// (it's a chain of frames pointing into the store), and dumping the
/// continuation chain would be noise in a test failure anyway.
impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Machine")
            .field("steps", &self.steps)
            .field("depth", &self.depth())
            .field("done", &self.is_done())
            .field("position", &self.position())
            .finish()
    }
}

pub fn run(expr: Expr, env: Env) -> Result<Val, LispErr> {
    run_bounded(expr, env, u64::MAX)
}

/// Like `run`, but errors after `budget` CEK steps. `u64::MAX` is
/// effectively unbounded. The budget guards against nonterminating
/// expressions in hosted environments (REPL, WASM bridge) where the
/// caller can't otherwise interrupt evaluation.
///
/// This is [`Machine`] with the pause turned back into an error, for
/// callers that want a value or nothing. Hosts that would rather resume
/// — anything on a main thread it can't block — should drive a `Machine`
/// directly, or go through `Vm::start` / `Vm::resume` to get the same
/// treatment for a whole batch of top-level forms.
pub fn run_bounded(expr: Expr, env: Env, budget: u64) -> Result<Val, LispErr> {
    let mut m = Machine::new(expr, env);
    match m.run(budget)? {
        Progress::Done(v) => Ok(v),
        Progress::Paused => Err(LispErr::new("execution exceeded step budget")),
    }
}
