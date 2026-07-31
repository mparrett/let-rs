//! Procedural macros for the `lisp` engine, extracted to its own
//! crate (ADR-024). The engine itself is macro-unaware: hosts that
//! want `defmacro` + quasiquote-with-macros wrap their `lisp::Vm`
//! in a [`MacroVm`] (or thread an [`Expander`] manually).
//!
//! Macros here are *procedural* and *unhygienic* — the macro body
//! is a lisp closure that takes datum args and returns a datum that
//! replaces the call site. Quasiquote (` `` `, `,`, `,@`) is built in.
//! Write macro bodies that don't shadow user code; there's no
//! gensym/renaming pass.
//!
//! ## Forms recognized
//!
//! - `(defmacro name (a b c) body)` — fixed arity, named params
//! - `(defmacro name args body)`    — variadic; `args` is bound to the full list
//!
//! ## Relationship to `lisp`
//!
//! Parser-level `quasiquote` / `unquote` / `unquote-splicing` stay in
//! `lisp::parse` because they're list-construction syntax — they work
//! without macros installed. What lives here is the *macro expansion*
//! pass that walks datums, calls macro closures, and handles macro
//! invocations *inside* unquoted positions.

use std::collections::HashMap;

use lisp::{Datum, DatumKind, LispErr, Namespace, Session, Span, Sym, Val, Vm, parse};
use std::rc::Rc;

/// Recursion ceiling for macro expansion. Sits above the reader's own
/// nesting cap (so a max-depth but legal form still expands) and well below
/// the stack-overflow point, so runaway self-referential macros abort cleanly.
const MAX_EXPANSION_DEPTH: usize = 1024;

#[derive(Clone)]
struct Macro {
    closure: Val,
    /// True for `(defmacro name args body)` (single symbol after the name) —
    /// the macro receives all call-site args bundled as one list. False for
    /// `(defmacro name (a b c) body)` — fixed arity, args passed positionally.
    variadic: bool,
}

/// Macro registry + expansion logic. Borrows `&mut Vm` per call so the
/// expander can evaluate macro closures against the Vm's env.
#[derive(Default)]
pub struct Expander {
    macros: HashMap<String, Macro>,
}

impl Expander {
    pub fn new() -> Self {
        Expander {
            macros: HashMap::new(),
        }
    }

    /// Top-level expansion entry point. Differs from [`expand_all`] in
    /// two ways: `(define name body…)` is allowed (its body forms get
    /// expanded but the define itself stays as the top-level head), and
    /// a macro call whose expansion is itself a `(define …)` form is
    /// re-processed at top level rather than rejected. Macros like
    /// `defspell` (which expand to `(define name (lambda …))`) need
    /// this — otherwise the expander's invariant "no nested define"
    /// would forbid the expansion even at the top of an `eval_str`
    /// batch.
    pub fn expand_top_level(&mut self, vm: &mut Vm, d: Datum) -> Result<Datum, LispErr> {
        self.expand_top_level_at(vm, d, 0)
    }

    fn expand_top_level_at(
        &mut self,
        vm: &mut Vm,
        d: Datum,
        depth: usize,
    ) -> Result<Datum, LispErr> {
        if depth > MAX_EXPANSION_DEPTH {
            return Err(LispErr::maybe_at("macro expansion too deep", d.span));
        }
        if let Some(items) = d.as_list()
            && let Some(head) = items.first().and_then(Datum::as_sym)
        {
            let name_str = &**head;
            // (define name body...) at top level: keep the define form
            // intact, but recursively expand the body forms (which sit
            // at expression position, so the no-nested-define rule
            // applies inside them).
            if name_str == "define" && items.len() >= 3 {
                let mut out = vec![items[0].clone(), items[1].clone()];
                for i in &items[2..] {
                    out.push(self.expand_all_at(vm, i.clone(), depth + 1)?);
                }
                return Ok(Datum::list(out, d.span));
            }
            // Top-level macro call: expand once, then re-enter at top
            // level so a macro that expands to `(define …)` is allowed.
            let mac = self.macros.get(name_str).cloned();
            if let Some(m) = mac {
                let expansion = self.expand_macro_call(vm, &m, &items[1..], d.span)?;
                return self.expand_top_level_at(vm, expansion, depth + 1);
            }
        }
        self.expand_all_at(vm, d, depth)
    }

    /// Recursively expand macro calls inside `d`. Returns the expanded
    /// datum (or `d` unchanged if no macros apply).
    pub fn expand_all(&mut self, vm: &mut Vm, d: Datum) -> Result<Datum, LispErr> {
        self.expand_all_at(vm, d, 0)
    }

    /// Depth-bounded workhorse for [`expand_all`]. A self-referential macro
    /// (e.g. `(defmacro foo (x) ` + "`" + `(foo ,x))` then `(foo 1)`) re-expands
    /// forever as native recursion — the Vm step budget bounds evaluation, not
    /// expansion — and a macro that emits deeply nested output recurses
    /// structurally. Both funnel through here, so one depth cap converts either
    /// into a clean error instead of a stack-overflow abort (fatal in wasm).
    fn expand_all_at(&mut self, vm: &mut Vm, d: Datum, depth: usize) -> Result<Datum, LispErr> {
        if depth > MAX_EXPANSION_DEPTH {
            return Err(LispErr::maybe_at("macro expansion too deep", d.span));
        }
        if let Some(items) = d.as_list()
            && let Some(head) = items.first().and_then(Datum::as_sym)
        {
            let name = head.clone();
            let name_str = &*name;

            // Opaque: contents are data.
            if name_str == "quote" {
                return Ok(d);
            }
            // Quasiquote: descend, but unquoted parts get full macro expansion.
            if name_str == "quasiquote" && items.len() == 2 {
                let inside = self.expand_in_qq(vm, items[1].clone(), 1)?;
                return Ok(Datum::list(vec![items[0].clone(), inside], d.span));
            }
            // Lambda: don't macro-expand the params list (it's symbols).
            if (name_str == "lambda" || name_str == "λ") && items.len() >= 3 {
                let mut out = vec![items[0].clone(), items[1].clone()];
                for i in &items[2..] {
                    out.push(self.expand_all_at(vm, i.clone(), depth + 1)?);
                }
                return Ok(Datum::list(out, d.span));
            }
            // set!: don't macro-expand the name slot (it's a binding
            // reference, like the head of a `define` or a let pair).
            // The value position gets normal expression-level expansion.
            if name_str == "set!" && items.len() == 3 {
                return Ok(Datum::list(
                    vec![
                        items[0].clone(),
                        items[1].clone(),
                        self.expand_all_at(vm, items[2].clone(), depth + 1)?,
                    ],
                    d.span,
                ));
            }
            // guard: `(guard (var handler) body)` binds `var`, so the
            // binder position is a name and not an expression. Without
            // this branch the expander treats `(var handler)` as an
            // ordinary application and expands `var` when it happens to
            // match a macro — `(guard (when when) …)` failed inside
            // expansion rather than binding a variable called `when`.
            // Same shape as the let-family and `set!` branches below.
            if name_str == "guard" && items.len() == 3 {
                let clause = match items[1].as_list() {
                    Some(pair) if pair.len() == 2 => Datum::list(
                        vec![
                            pair[0].clone(),
                            self.expand_all_at(vm, pair[1].clone(), depth + 1)?,
                        ],
                        items[1].span,
                    ),
                    // Malformed: leave it for `compile` to reject with a
                    // position, rather than guessing here.
                    _ => items[1].clone(),
                };
                return Ok(Datum::list(
                    vec![
                        items[0].clone(),
                        clause,
                        self.expand_all_at(vm, items[2].clone(), depth + 1)?,
                    ],
                    d.span,
                ));
            }
            // Let-family: don't macro-expand binding-name positions.
            if matches!(name_str, "let" | "let*" | "letrec") && items.len() >= 3 {
                let bindings_out = match items[1].as_list() {
                    Some(pairs) => {
                        let mut new_pairs = Vec::with_capacity(pairs.len());
                        for p in pairs {
                            match p.as_list() {
                                Some(pair) if pair.len() == 2 => {
                                    new_pairs.push(Datum::list(
                                        vec![
                                            pair[0].clone(),
                                            self.expand_all_at(vm, pair[1].clone(), depth + 1)?,
                                        ],
                                        p.span,
                                    ));
                                }
                                _ => new_pairs.push(p.clone()),
                            }
                        }
                        Datum::list(new_pairs, items[1].span)
                    }
                    None => items[1].clone(),
                };
                let mut out = vec![items[0].clone(), bindings_out];
                for i in &items[2..] {
                    out.push(self.expand_all_at(vm, i.clone(), depth + 1)?);
                }
                return Ok(Datum::list(out, d.span));
            }
            // Defmacro / define inside other code: refuse so silent
            // misregistration doesn't bite us.
            if name_str == "defmacro" {
                return Err(LispErr::maybe_at(
                    "defmacro only valid at top level",
                    d.span,
                ));
            }
            if name_str == "define" {
                return Err(LispErr::maybe_at("define only valid at top level", d.span));
            }

            // Macro lookup
            let mac = self.macros.get(name_str).cloned();
            if let Some(m) = mac {
                let expansion = self.expand_macro_call(vm, &m, &items[1..], d.span)?;
                return self.expand_all_at(vm, expansion, depth + 1);
            }
        }

        let span = d.span;
        match d.kind {
            DatumKind::List(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for i in items {
                    new_items.push(self.expand_all_at(vm, i, depth + 1)?);
                }
                Ok(Datum::list(new_items, span))
            }
            other => Ok(Datum::new(other, span)),
        }
    }

    fn expand_in_qq(&mut self, vm: &mut Vm, d: Datum, depth: usize) -> Result<Datum, LispErr> {
        if depth == 0 {
            return self.expand_all(vm, d);
        }
        if let Some(items) = d.as_list()
            && !items.is_empty()
        {
            if let Some(s) = items[0].as_sym() {
                let name = &**s;
                if name == "quasiquote" && items.len() == 2 {
                    let inside = self.expand_in_qq(vm, items[1].clone(), depth + 1)?;
                    return Ok(Datum::list(vec![items[0].clone(), inside], d.span));
                }
                if (name == "unquote" || name == "unquote-splicing") && items.len() == 2 {
                    let inside = self.expand_in_qq(vm, items[1].clone(), depth - 1)?;
                    return Ok(Datum::list(vec![items[0].clone(), inside], d.span));
                }
            }
            let mut new_items = Vec::with_capacity(items.len());
            for i in items {
                new_items.push(self.expand_in_qq(vm, i.clone(), depth)?);
            }
            return Ok(Datum::list(new_items, d.span));
        }
        Ok(d)
    }

    /// Call a macro closure with the call site's raw argument datums and
    /// convert its returned value back into a datum.
    ///
    /// `call_span` is the position of the macro *call*, and every datum in
    /// the expansion is stamped with it. That is deliberately coarse: an
    /// error anywhere inside an expansion reports the line the user
    /// actually wrote, rather than reporting nothing at all (which is
    /// what ADR-022 left as a deferred item). It's coarse for a second,
    /// unavoidable reason too — macro arguments round-trip through `Val`
    /// on their way into the closure, and `Val` carries no spans, so even
    /// user code passed through a macro comes back position-less.
    fn expand_macro_call(
        &mut self,
        vm: &mut Vm,
        m: &Macro,
        raw_args: &[Datum],
        call_span: Option<Span>,
    ) -> Result<Datum, LispErr> {
        let arg_vals: Vec<Val> = raw_args.iter().map(parse::datum_to_val).collect();
        let args_to_pass = if m.variadic {
            vec![Val::list_from(&arg_vals)]
        } else {
            arg_vals
        };
        let result = vm
            .call_value(&m.closure, args_to_pass)
            .map_err(|e| e.with_span(call_span))?;
        val_to_datum(&result, call_span)
    }

    /// If `d` is `(defmacro name params body)`, register the macro and
    /// return `Ok(true)`. Otherwise return `Ok(false)` and leave `d` for
    /// the caller to handle.
    pub fn try_register_defmacro(&mut self, vm: &mut Vm, d: &Datum) -> Result<bool, LispErr> {
        let items = match d.as_list() {
            Some(items) => items,
            None => return Ok(false),
        };
        match items.first().and_then(Datum::as_sym) {
            Some(s) if &**s == "defmacro" => {}
            _ => return Ok(false),
        }
        if items.len() != 4 {
            return Err(LispErr::maybe_at(
                "defmacro: expected (defmacro name params body)",
                d.span,
            ));
        }
        let name: Sym = match items[1].as_sym() {
            Some(s) => s.clone(),
            None => {
                return Err(LispErr::maybe_at(
                    "defmacro: name must be a symbol",
                    items[1].span,
                ));
            }
        };
        let (params, variadic): (Vec<Sym>, bool) = match &items[2].kind {
            DatumKind::Sym(s) => (vec![s.clone()], true),
            DatumKind::List(ps) => {
                let names: Result<Vec<Sym>, LispErr> = ps
                    .iter()
                    .map(|p| {
                        p.as_sym().cloned().ok_or_else(|| {
                            LispErr::maybe_at("defmacro: param must be a symbol", p.span)
                        })
                    })
                    .collect();
                (names?, false)
            }
            _ => {
                return Err(LispErr::maybe_at(
                    "defmacro: params must be a symbol or a list",
                    items[2].span,
                ));
            }
        };
        // Expand macros inside the body so macros can use other macros.
        let body_datum = self.expand_all(vm, items[3].clone())?;
        let body_expr = parse::compile(&body_datum)?;
        let closure = Val::Clo {
            params: params.into(),
            body: Rc::new(body_expr),
            env: vm.env().clone(),
        };
        self.macros
            .insert(name.to_string(), Macro { closure, variadic });
        Ok(true)
    }

    /// Snapshot the macro table for transactional rollback.
    fn snapshot(&self) -> HashMap<String, Macro> {
        self.macros.clone()
    }

    fn restore(&mut self, snap: HashMap<String, Macro>) {
        self.macros = snap;
    }
}

/// Convenience wrapper that bundles a [`lisp::Vm`] with an [`Expander`]
/// and provides a macro-aware `eval_str`. Hosts that want macros use
/// this; hosts that don't can stay on a raw `lisp::Vm`.
///
/// `vm` is `pub` so hosts can install prims, preludes, set the step
/// budget, etc., on the inner engine directly.
pub struct MacroVm {
    pub vm: Vm,
    pub expander: Expander,
}

impl MacroVm {
    pub fn new() -> Self {
        MacroVm {
            vm: Vm::new(),
            expander: Expander::new(),
        }
    }

    pub fn from_vm(vm: Vm) -> Self {
        MacroVm {
            vm,
            expander: Expander::new(),
        }
    }

    /// Construct a MacroVm with [`STDLIB`] already installed.
    /// Equivalent to `Self::new()` + [`install_stdlib`].
    pub fn with_stdlib() -> Self {
        let mut vm = Self::new();
        install_stdlib(&mut vm).expect("STDLIB should always install cleanly");
        vm
    }

    /// Macro-aware `eval_str`. Forms are expanded, then handled in
    /// document order:
    ///
    /// - `(defmacro …)`  — registered in the expander
    /// - `(define …)`    — pre-pass + post-pass via `lisp::Vm`
    /// - anything else   — compiled and evaluated by `lisp::Vm`
    ///
    /// Atomic semantics: if any form fails, the macro table is restored
    /// in addition to whatever `lisp::Vm::eval_str` restores on its end.
    pub fn eval_str(&mut self, src: &str) -> Result<Val, LispErr> {
        let root = Rc::clone(self.vm.root());
        self.eval_str_in(&root, src)
    }

    /// Macro-aware [`lisp::Vm::eval_str_in`]: expand, then evaluate
    /// inside `ns` so a pack's prelude and casts resolve its own private
    /// vocabulary (ADR-042).
    ///
    /// Macros themselves stay global to the expander — the macro table is
    /// not namespaced, so two packs defining the same macro name still
    /// collide. Nothing in-tree does; noted in ADR-042 as the remaining
    /// half.
    pub fn eval_str_in(&mut self, ns: &Rc<Namespace>, src: &str) -> Result<Val, LispErr> {
        let saved = self.expander.snapshot();
        let result = self.eval_str_inner(ns, src);
        if result.is_err() {
            self.expander.restore(saved);
        }
        result
    }

    fn eval_str_inner(&mut self, ns: &Rc<Namespace>, src: &str) -> Result<Val, LispErr> {
        let forms = parse::read_many(src)?;

        // Split out defmacro forms (register them) and collect the rest.
        // Note: defmacro forms register in document order; subsequent
        // forms can use macros defined earlier in the same batch.
        // Non-defmacro forms are expanded *before* being re-stringified
        // back through lisp::Vm::eval_str, because Vm::eval_str doesn't
        // know about macros.
        let mut remaining: Vec<Datum> = Vec::with_capacity(forms.len());
        for datum in forms {
            if self.expander.try_register_defmacro(&mut self.vm, &datum)? {
                continue;
            }
            remaining.push(self.expander.expand_top_level(&mut self.vm, datum)?);
        }

        // Hand the expanded datums straight to the engine. This used to
        // re-serialize them to source text and make the reader parse
        // them a second time — parse → expand → print → parse → compile
        // — because `eval_str` was the only public way in. ADR-034 added
        // `eval_datums`, which is that entry point without the detour.
        if remaining.is_empty() {
            return Ok(Val::Bool(true));
        }
        self.vm.eval_datums_in(ns, &remaining)
    }

    /// Macro-aware [`lisp::Vm::start`]: register `defmacro`s, expand
    /// everything else, and hand the result to the engine as a resumable
    /// [`Session`]. Drive it with `mvm.vm.resume(&mut session, slice)`.
    ///
    /// All expansion happens here, up front. That's not just convenient —
    /// expansion *evaluates* macro bodies through `call_value`, which runs
    /// to completion and can't be sliced, so there'd be nothing to
    /// interleave even if it were deferred. What a session paces is the
    /// evaluation of already-expanded code.
    ///
    /// A batch with no evaluable forms (all `defmacro`) yields a session
    /// that finishes on its first `resume` with `#t`, matching `eval_str`.
    pub fn start(&mut self, src: &str) -> Result<Session, LispErr> {
        let root = Rc::clone(self.vm.root());
        self.start_in(&root, src)
    }

    /// [`MacroVm::start`] targeting `ns`.
    pub fn start_in(&mut self, ns: &Rc<Namespace>, src: &str) -> Result<Session, LispErr> {
        let saved = self.expander.snapshot();
        let result = self.start_inner(ns, src);
        if result.is_err() {
            // Same atomicity as `eval_str`: a batch that fails during
            // registration or expansion leaves no macros behind. Failures
            // *after* this point are engine-level and don't touch the
            // macro table, so `resume` has nothing to restore.
            self.expander.restore(saved);
        }
        result
    }

    fn start_inner(&mut self, ns: &Rc<Namespace>, src: &str) -> Result<Session, LispErr> {
        let forms = parse::read_many(src)?;
        let mut remaining: Vec<Datum> = Vec::with_capacity(forms.len());
        for datum in forms {
            if self.expander.try_register_defmacro(&mut self.vm, &datum)? {
                continue;
            }
            remaining.push(self.expander.expand_top_level(&mut self.vm, datum)?);
        }
        self.vm.start_datums_in(ns, &remaining)
    }
}

impl Default for MacroVm {
    fn default() -> Self {
        Self::new()
    }
}

/// A small library of useful macros:
///
/// - `(begin a b ... last)` — evaluate each form in order; return the
///   value of `last`. Replaces the engine-side `(let ((_ a)) b)`
///   workaround pattern (ADR-019 deferred item — closed via this
///   macro rather than an engine special form, per ADR-024's
///   minimal-engine stance).
/// - `(when c body...)` — `(if c (begin body...) #f)`.
/// - `(unless c body...)` — `(if c #f (begin body...))`.
/// - `(and a b ...)` — short-circuit conjunction. `(and)` → `#t`;
///   `(and x)` → `x`; otherwise returns the last truthy value or `#f`
///   on the first falsy.
/// - `(or a b ...)` — short-circuit disjunction. `(or)` → `#f`;
///   `(or x)` → `x`; otherwise returns the first truthy value or
///   `#f` if all are falsy. Uses a `__or-val__` temp binding to
///   avoid double-evaluating side-effecting args; the unhygienic
///   name could collide if user code binds `__or-val__` and
///   references it in a later `or` argument (vanishingly unlikely
///   but documented).
///
/// Opt-in: hosts that want it call [`install_stdlib`] explicitly, or
/// construct via [`MacroVm::with_stdlib`].
pub const STDLIB: &str = r#"
    (defmacro begin args
      (if (null? (cdr args))
          (car args)
          `(let ((_ ,(car args))) (begin ,@(cdr args)))))

    (defmacro when args
      `(if ,(car args) (begin ,@(cdr args)) #f))

    (defmacro unless args
      `(if ,(car args) #f (begin ,@(cdr args))))

    (defmacro and args
      (if (null? args)
          #t
          (if (null? (cdr args))
              (car args)
              `(if ,(car args) (and ,@(cdr args)) #f))))

    (defmacro or args
      (if (null? args)
          #f
          (if (null? (cdr args))
              (car args)
              `(let ((__or-val__ ,(car args)))
                 (if __or-val__ __or-val__ (or ,@(cdr args)))))))
"#;

/// Install [`STDLIB`] macros into `vm`'s expander. Mirrors the
/// `spells::install` / `world::world_prim::install` pattern.
pub fn install_stdlib(vm: &mut MacroVm) -> Result<(), LispErr> {
    vm.eval_str(STDLIB).map(|_| ())
}

/// Macro output (a `Val`) back into a datum the compiler can see, with
/// `span` stamped on every node — see [`Expander::expand_macro_call`].
fn val_to_datum(v: &Val, span: Option<Span>) -> Result<Datum, LispErr> {
    match v {
        Val::Num(n) => Ok(Datum::num(*n, span)),
        Val::Ratio(n, d) => Ok(Datum::new(DatumKind::Ratio(*n, *d), span)),
        Val::Bool(b) => Ok(Datum::bool(*b, span)),
        Val::Sym(s) => Ok(Datum::sym(s.clone(), span)),
        Val::Str(s) => Ok(Datum::str(s.clone(), span)),
        Val::Nil => Ok(Datum::list(vec![], span)),
        Val::Cons(_, _) => {
            let mut items = Vec::new();
            let mut cur = v;
            loop {
                match cur {
                    Val::Cons(h, t) => {
                        items.push(val_to_datum(h, span)?);
                        cur = t;
                    }
                    Val::Nil => break,
                    other => {
                        return Err(LispErr::maybe_at(
                            format!("non-proper list in macro expansion: {other}"),
                            span,
                        ));
                    }
                }
            }
            Ok(Datum::list(items, span))
        }
        other => Err(LispErr::maybe_at(
            format!("can't convert {other} back to a datum"),
            span,
        )),
    }
}

// `datum_to_source` (Datum → source text, so the engine's reader could
// parse the expansion a second time) was deleted in ADR-034. Its only
// caller now uses `Vm::eval_datums`. It was also lossy: it emitted just
// four string escapes, so a macro emitting a string with any other
// control character produced source the tokenizer would reject.
