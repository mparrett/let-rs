//! Can a line typed in the web REPL change what a lab button does?
//!
//! The original complaint (issue_4, from the 2026-05-26 codex review) was
//! that `web/common.js` shares one `Vm` between the REPL panel and the
//! lab, so a curious typer could redefine `thread` or `fire` and silently
//! change what casting did. ADR-042 and ADR-043 closed most of that
//! without setting out to: the bridge runs casts with `eval_str_in(ns,…)`
//! while the REPL evaluates at the root, and a pack resolves its own
//! vocabulary lexically.
//!
//! These mirror the bridge's two paths exactly — pack source in the pack
//! namespace, REPL source at the root — and pin both what is now
//! contained and what still isn't.

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

/// The same shape the WASM bridge generates for a one-rune tape, with the
/// rune list inlined (`runes` isn't a dependency here; ☲ lexes to `fire`).
fn cast(vm: &mut MacroVm, ns: &lisp::NsHandle) -> Result<String, String> {
    let src = "(cast! (assoc-set 'tx 3 (assoc-set 'ty 2 (thread (start) (list fire)))))";
    vm.eval_str_in(ns, src)
        .map(|v| format!("{v}"))
        .map_err(|e| e.msg)
}

#[test]
fn a_repl_define_cannot_hijack_a_lab_cast() {
    // The ticket's own examples. Each of these used to land in the one
    // shared table and change what the lab did on its next cast.
    for hijack in [
        // A pack-private helper the cast source names directly.
        "(define thread (lambda (a b) 'HIJACKED))",
        // A name the pack exports, so the REPL can see it.
        "(define fire (lambda (ctx) 'HIJACKED))",
        // The mana-gated entry point the bridge calls.
        "(define cast! (lambda (ctx) 'HIJACKED))",
    ] {
        let (mut vm, ns, _world) = lab();
        assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("1"), "baseline");
        vm.eval_str(hijack).expect("the REPL line itself is legal");
        assert_eq!(
            cast(&mut vm, &ns).as_deref(),
            Ok("1"),
            "a root `define` reached into the pack: {hijack}"
        );
    }
}

#[test]
fn a_repl_define_of_a_builtin_still_reaches_into_packs() {
    // **This test asserts a limitation, not a feature.** A pack chains to
    // the root for builtins (ADR-020 put them in the globals table), so a
    // root `define` that shadows one is visible inside every pack — and
    // the lab breaks. It is the part of issue_4 that ADR-042 did not
    // close, because closing it means deciding what else a pack should
    // stop seeing through the root, which is an ADR and not a fix.
    //
    // If this starts failing, the hole was closed: delete it, fold the
    // case into the test above, and close issue_4.
    let (mut vm, ns, _world) = lab();
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("1"), "baseline");
    vm.eval_str("(define car (lambda (x) 'HIJACKED))").unwrap();
    let err = cast(&mut vm, &ns).expect_err("the cast should break — that's the limitation");
    assert!(err.contains("not callable"), "unexpected error: {err}");
}

#[test]
fn set_bang_through_an_export_reaches_the_pack_deliberately() {
    // Not a hole — the mechanism ADR-042 built on purpose. An export
    // shares the *cell*, so `set!` through either name writes the same
    // slot, which is what lets the bridge read the mana counter from the
    // root while it lives in the pack (ADR-037's grandfathered
    // exception). The same mechanism means a REPL `set!` on an exported
    // name is felt inside the pack. Pinned so nobody "fixes" it and
    // silently breaks mana.
    let (mut vm, ns, _world) = lab();
    assert_eq!(cast(&mut vm, &ns).as_deref(), Ok("1"), "baseline");
    vm.eval_str("(set! mana 0)").unwrap();
    assert_eq!(
        cast(&mut vm, &ns).as_deref(),
        Ok("0"),
        "a root `set!` on an exported name should reach the pack's cell"
    );
}
