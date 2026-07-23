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

use lisp::{Datum, Sym, Val, Vm, parse};
use std::rc::Rc;

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
    pub fn expand_top_level(&mut self, vm: &mut Vm, d: Datum) -> Result<Datum, String> {
        if let Datum::List(items) = &d
            && !items.is_empty()
            && let Datum::Sym(head) = &items[0]
        {
            let name_str = &**head;
            // (define name body...) at top level: keep the define form
            // intact, but recursively expand the body forms (which sit
            // at expression position, so the no-nested-define rule
            // applies inside them).
            if name_str == "define" && items.len() >= 3 {
                let mut out = vec![items[0].clone(), items[1].clone()];
                for i in &items[2..] {
                    out.push(self.expand_all(vm, i.clone())?);
                }
                return Ok(Datum::List(out));
            }
            // Top-level macro call: expand once, then re-enter at top
            // level so a macro that expands to `(define …)` is allowed.
            let mac = self.macros.get(name_str).cloned();
            if let Some(m) = mac {
                let expansion = self.expand_macro_call(vm, &m, &items[1..])?;
                return self.expand_top_level(vm, expansion);
            }
        }
        self.expand_all(vm, d)
    }

    /// Recursively expand macro calls inside `d`. Returns the expanded
    /// datum (or `d` unchanged if no macros apply).
    pub fn expand_all(&mut self, vm: &mut Vm, d: Datum) -> Result<Datum, String> {
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
                let inside = self.expand_in_qq(vm, items[1].clone(), 1)?;
                return Ok(Datum::List(vec![items[0].clone(), inside]));
            }
            // Lambda: don't macro-expand the params list (it's symbols).
            if (name_str == "lambda" || name_str == "λ") && items.len() >= 3 {
                let mut out = vec![items[0].clone(), items[1].clone()];
                for i in &items[2..] {
                    out.push(self.expand_all(vm, i.clone())?);
                }
                return Ok(Datum::List(out));
            }
            // set!: don't macro-expand the name slot (it's a binding
            // reference, like the head of a `define` or a let pair).
            // The value position gets normal expression-level expansion.
            if name_str == "set!" && items.len() == 3 {
                return Ok(Datum::List(vec![
                    items[0].clone(),
                    items[1].clone(),
                    self.expand_all(vm, items[2].clone())?,
                ]));
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
                                    self.expand_all(vm, pair[1].clone())?,
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
                    out.push(self.expand_all(vm, i.clone())?);
                }
                return Ok(Datum::List(out));
            }
            // Defmacro / define inside other code: refuse so silent
            // misregistration doesn't bite us.
            if name_str == "defmacro" {
                return Err("defmacro only valid at top level".into());
            }
            if name_str == "define" {
                return Err("define only valid at top level".into());
            }

            // Macro lookup
            let mac = self.macros.get(name_str).cloned();
            if let Some(m) = mac {
                let expansion = self.expand_macro_call(vm, &m, &items[1..])?;
                return self.expand_all(vm, expansion);
            }
        }

        match d {
            Datum::List(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for i in items {
                    new_items.push(self.expand_all(vm, i)?);
                }
                Ok(Datum::List(new_items))
            }
            other => Ok(other),
        }
    }

    fn expand_in_qq(&mut self, vm: &mut Vm, d: Datum, depth: usize) -> Result<Datum, String> {
        if depth == 0 {
            return self.expand_all(vm, d);
        }
        if let Datum::List(items) = &d
            && !items.is_empty()
        {
            if let Datum::Sym(s) = &items[0] {
                let name = &**s;
                if name == "quasiquote" && items.len() == 2 {
                    let inside = self.expand_in_qq(vm, items[1].clone(), depth + 1)?;
                    return Ok(Datum::List(vec![items[0].clone(), inside]));
                }
                if (name == "unquote" || name == "unquote-splicing") && items.len() == 2 {
                    let inside = self.expand_in_qq(vm, items[1].clone(), depth - 1)?;
                    return Ok(Datum::List(vec![items[0].clone(), inside]));
                }
            }
            let mut new_items = Vec::with_capacity(items.len());
            for i in items {
                new_items.push(self.expand_in_qq(vm, i.clone(), depth)?);
            }
            return Ok(Datum::List(new_items));
        }
        Ok(d)
    }

    fn expand_macro_call(
        &mut self,
        vm: &mut Vm,
        m: &Macro,
        raw_args: &[Datum],
    ) -> Result<Datum, String> {
        let arg_vals: Vec<Val> = raw_args.iter().map(parse::datum_to_val).collect();
        let args_to_pass = if m.variadic {
            vec![Val::list_from(&arg_vals)]
        } else {
            arg_vals
        };
        let result = vm.call_value(&m.closure, args_to_pass)?;
        val_to_datum(&result)
    }

    /// If `d` is `(defmacro name params body)`, register the macro and
    /// return `Ok(true)`. Otherwise return `Ok(false)` and leave `d` for
    /// the caller to handle.
    pub fn try_register_defmacro(&mut self, vm: &mut Vm, d: &Datum) -> Result<bool, String> {
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
        let body_datum = self.expand_all(vm, items[3].clone())?;
        let body_expr = parse::compile(&body_datum)?;
        let closure = Val::Clo {
            params,
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
    pub fn eval_str(&mut self, src: &str) -> Result<Val, String> {
        let saved = self.expander.snapshot();
        let result = self.eval_str_inner(src);
        if result.is_err() {
            self.expander.restore(saved);
        }
        result
    }

    fn eval_str_inner(&mut self, src: &str) -> Result<Val, String> {
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

        // Hand the expanded datums to lisp::Vm by stringifying them.
        // Stringify is lossless for our Datum set (Num/Ratio/Bool/Sym/
        // List/Quote); the round-trip through the reader is the price
        // of going through the public Vm::eval_str entry point.
        if remaining.is_empty() {
            return Ok(Val::Bool(true));
        }
        let mut combined = String::new();
        for d in &remaining {
            datum_to_source(d, &mut combined);
            combined.push('\n');
        }
        self.vm.eval_str(&combined)
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
pub fn install_stdlib(vm: &mut MacroVm) -> Result<(), String> {
    vm.eval_str(STDLIB).map(|_| ())
}

fn val_to_datum(v: &Val) -> Result<Datum, String> {
    match v {
        Val::Num(n) => Ok(Datum::Num(*n)),
        Val::Ratio(n, d) => Ok(Datum::Ratio(*n, *d)),
        Val::Bool(b) => Ok(Datum::Bool(*b)),
        Val::Sym(s) => Ok(Datum::Sym(s.clone())),
        Val::Str(s) => Ok(Datum::Str(s.clone())),
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

/// Serialize a Datum back to source text so the engine's parser can
/// re-read it. Lossless for our Datum set; matches `Val::Display` for
/// the common cases.
fn datum_to_source(d: &Datum, out: &mut String) {
    use std::fmt::Write;
    match d {
        Datum::Num(n) => write!(out, "{n}").unwrap(),
        Datum::Ratio(n, dn) => write!(out, "{n}/{dn}").unwrap(),
        Datum::Bool(true) => out.push_str("#t"),
        Datum::Bool(false) => out.push_str("#f"),
        Datum::Sym(s) => out.push_str(s),
        Datum::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }
        Datum::List(items) => {
            out.push('(');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                datum_to_source(item, out);
            }
            out.push(')');
        }
    }
}
