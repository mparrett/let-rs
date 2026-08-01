//! What a line typed in the web REPL can still do to a lab cast (issue_4).
//!
//! The ticket (codex review, 2026-05-26) is that `web/common.js` shares one
//! `Vm` between the REPL panel and the lab, so a curious typer can change
//! what the lab's buttons do. ADR-042 and ADR-043 were assumed to have
//! fixed it. They fixed **one mechanism**: a root `define` can no longer
//! shadow a pack's own vocabulary, because casts evaluate inside the pack
//! namespace and a pack resolves its names lexically.
//!
//! Almost everything else is still open, and the first version of this
//! file said otherwise — it tested `define` on three names, found them
//! contained, and generalised. What follows is the measured surface, with
//! the containment and the holes given equal weight. Most of the holes are
//! *silent*: the cast returns a plausible number and paints nothing, or
//! paints the wrong thing.
//!
//! | root REPL line | reaches the pack? |
//! |---|---|
//! | `(define thread …)` / `(define fire …)` / `(define cast! …)` | no — the fix |
//! | `(set! fire …)` / `(set! cast! …)` — any export | **yes**, exports share a cell |
//! | `(defmacro cast! …)` — any macro name | **yes**, root macros resolve into packs |
//! | `(define car …)` / `(define cons …)` — a builtin | **yes**, packs chain to root |
//! | `(define world-apply! …)` — a host prim | **yes**, prims install into root |
//! | `(define mana …)` — over an exported cell | breaks the alias; desyncs the host |
//!
//! Every test below that asserts a hole is asserting a **limitation, not a
//! feature**. If one starts failing, that route was closed: move it into
//! `a_root_define_cannot_hijack_pack_vocabulary` and update issue_4.

use macros::MacroVm;
use std::cell::RefCell;
use std::rc::Rc;
use world::World;

fn lab() -> (MacroVm, lisp::NsHandle, Rc<RefCell<World>>) {
    let world = Rc::new(RefCell::new(World::new(7, 5).expect("world")));
    let mut vm = MacroVm::new();
    let ns = spells::install_with_world(&mut vm, world.clone());
    (vm, ns, world)
}

/// The shape the WASM bridge generates for a one-rune tape, with the rune
/// list inlined (`runes` isn't a dependency here; ☲ lexes to `fire`). A
/// healthy cast returns `1` — one tile painted. The bridge additionally
/// reads the world log; these assert on the value and, where it matters,
/// the log.
fn cast(vm: &mut MacroVm, ns: &lisp::NsHandle) -> Result<String, String> {
    let src = "(cast! (assoc-set 'tx 3 (assoc-set 'ty 2 (thread (start) (list fire)))))";
    vm.eval_str_in(ns, src)
        .map(|v| format!("{v}"))
        .map_err(|e| e.msg)
}

/// Assert the baseline cast is healthy, so a later assertion of `"1"` is
/// known to mean "worked" rather than "happened to match".
fn healthy(vm: &mut MacroVm, ns: &lisp::NsHandle) {
    assert_eq!(cast(vm, ns).as_deref(), Ok("1"), "baseline cast");
}

#[test]
fn a_root_define_cannot_hijack_pack_vocabulary() {
    // This is what ADR-042 actually bought. `define` binds a *new* cell in
    // the root table; the pack keeps its own and never consults root for a
    // name it defines itself.
    for (hijack, probe) in [
        // A pack-private helper the generated cast source names directly.
        ("(define thread (lambda (a b) 'HIJACKED))", "thread"),
        // A name the pack exports, so the REPL can see it.
        ("(define fire (lambda (ctx) 'HIJACKED))", "fire"),
        // The mana-gated entry point the bridge calls.
        ("(define cast! (lambda (ctx) 'HIJACKED))", "cast!"),
    ] {
        let (mut vm, ns, _world) = lab();
        healthy(&mut vm, &ns);
        vm.eval_str(hijack).expect("the REPL line itself is legal");
        // The define really landed at root — otherwise this test would
        // pass by doing nothing at all.
        assert!(
            vm.vm.global(probe).is_some(),
            "{probe} should now be bound at root"
        );
        assert_eq!(
            cast(&mut vm, &ns).as_deref(),
            Ok("1"),
            "a root `define` reached into the pack: {hijack}"
        );
    }
}

#[test]
fn a_root_set_bang_on_an_export_still_hijacks_the_lab() {
    // LIMITATION. `export` shares the binding *cell* (ADR-042), which is
    // what lets the bridge read the mana counter from root while it lives
    // in the pack (ADR-037's grandfathered exception). The same mechanism
    // means every exported name is *writable* from root — including the
    // ones bound to closures. So the ticket's own sentence, "redefine
    // `fire` and silently change what casting did", is still true; it just
    // needs `set!` where it used to need `define`.
    let (mut vm, ns, world) = lab();
    healthy(&mut vm, &ns);
    vm.eval_str("(set! fire (lambda (ctx) 'HIJACKED))").unwrap();
    // Silent: `cast!` guards, refunds, and reports 0 painted tiles.
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("0"));
    assert!(
        world.borrow().log.last().is_some_and(|l| l.contains("cast-failed")),
        "expected a cast-failed log entry, got {:?}",
        world.borrow().log.last()
    );

    // And the entry point itself, which doesn't even fail loudly.
    let (mut vm, ns, _world) = lab();
    healthy(&mut vm, &ns);
    vm.eval_str("(set! cast! (lambda (ctx) 'HIJACKED))").unwrap();
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("HIJACKED"));
}

#[test]
fn a_root_defmacro_still_hijacks_the_lab() {
    // LIMITATION, and the one that most needs stating: ADR-043 namespaced
    // the macro tables, but resolution walks *outward* so the root stdlib
    // stays visible inside every pack. A macro defined at root is
    // therefore visible inside every pack too — and a pack cannot shadow
    // one it doesn't itself define. Namespacing macros closed
    // pack-vs-pack collisions; it did not close root-vs-pack.
    for (hijack, expected) in [
        ("(defmacro cast! (ctx) ''HIJACKED)", "HIJACKED"),
        ("(defmacro thread (a b) ''HIJACKED)", "0"),
    ] {
        let (mut vm, ns, _world) = lab();
        healthy(&mut vm, &ns);
        vm.eval_str(hijack).expect("the REPL line itself is legal");
        assert_eq!(
            cast(&mut vm, &ns).as_deref(),
            Ok(expected),
            "expected the root macro to reach the pack: {hijack}"
        );
    }
}

#[test]
fn a_root_define_of_a_builtin_or_host_prim_still_reaches_packs() {
    // LIMITATION. A pack chains to the root for everything it doesn't
    // define, which is builtins (ADR-020 moved them into the globals
    // table) *and* host prims — `install_with_world` deliberately puts the
    // world prims in the root rather than the pack, because they're a host
    // capability rather than spell vocabulary (ADR-042).
    //
    // Note how few of these are loud. `car` happens to produce an error;
    // the rest return a plausible number and quietly do the wrong thing,
    // which is worse.
    let (mut vm, ns, _world) = lab();
    healthy(&mut vm, &ns);
    vm.eval_str("(define car (lambda (x) 'HIJACKED))").unwrap();
    let err = cast(&mut vm, &ns).expect_err("shadowing `car` breaks the cast");
    assert_eq!(
        err, "not callable: HIJACKED",
        "the error should name the injected value, or this test passes for any breakage"
    );

    // Silent: a wrong answer, no error.
    let (mut vm, ns, _world) = lab();
    healthy(&mut vm, &ns);
    vm.eval_str("(define cons (lambda (a b) 'HIJACKED))").unwrap();
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("0"));

    // Silent, and it's a *host* capability, not language vocabulary: the
    // REPL overrides what painting a tile means, and mana is still spent.
    let (mut vm, ns, _world) = lab();
    healthy(&mut vm, &ns);
    vm.eval_str("(define world-apply! (lambda (ctx) 42))").unwrap();
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("42"));
}

#[test]
fn a_root_define_over_an_exported_cell_desyncs_the_host() {
    // LIMITATION, and the inverse of the `set!` case: `define` does *not*
    // write through the shared cell, it replaces root's entry with a fresh
    // one. So the alias export set up is broken from then on. The pack
    // still has its own `mana` and spends it normally, while the host —
    // which reads the meter with `Vm::global`, see `WasmVm::mana` — reads
    // the detached root cell and shows a frozen number forever.
    let (mut vm, ns, _world) = lab();
    healthy(&mut vm, &ns);
    let before = vm.vm.global("mana").map(|v| format!("{v}"));
    assert_eq!(before.as_deref(), Some("9"), "host sees mana spent by the cast");

    vm.eval_str("(define mana 999)").unwrap();
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("1"), "the pack casts on unaffected");
    assert_eq!(
        vm.vm.global("mana").map(|v| format!("{v}")).as_deref(),
        Some("999"),
        "the host's view is now detached from the pack's real mana"
    );

    // And it defeats the `set!` route above, since there is no longer a
    // shared cell to write through.
    vm.eval_str("(set! mana 0)").unwrap();
    assert_eq!(
        cast(&mut vm, &ns).as_deref(),
        Ok("1"),
        "after a root `define`, `set!` no longer reaches the pack"
    );
}
