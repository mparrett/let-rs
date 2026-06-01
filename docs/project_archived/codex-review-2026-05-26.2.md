I checked the repo for freshness/accuracy. Code health is good: `cargo test --workspace` passes with 97 tests, and `just check` passes.

Main stale spots are documentation, not implementation:

- [CLAUDE.md](/Users/matt/projects-new/3p/letrs/CLAUDE.md:142) still says `just` runs 90 tests; current verified count is 97.
- [docs/project_notes/key_facts.md](/Users/matt/projects-new/3p/letrs/docs/project_notes/key_facts.md:25) is older: “34 currently”, old crate layout, old `shell.js`, old test counts, and `lisp/src/world.rs` / `world_prim.rs` still listed under the core crate.
- [crates/lisp/src/lib.rs](/Users/matt/projects-new/3p/letrs/crates/lisp/src/lib.rs:9) crate docs still list `world` and `world_prim` modules even though ADR-018 moved them to `crates/world`.
- [docs/letrs.html](/Users/matt/projects-new/3p/letrs/docs/letrs.html:963) still says spell/genes vocabulary got hoisted into `lisp`; current code has sibling crates `spells` and `genes`.
- [docs/letrs.html](/Users/matt/projects-new/3p/letrs/docs/letrs.html:976) and [docs/project_notes/host-state.md](/Users/matt/projects-new/3p/letrs/docs/project_notes/host-state.md:74) still describe genes carrying dummy `World::empty()` state; current `Vm::new()` is host-agnostic.
- [docs/letrs.html](/Users/matt/projects-new/3p/letrs/docs/letrs.html:1063) says cross-call mutual recursion still does not work; current code and tests say it does.
- [docs/letrs.html](/Users/matt/projects-new/3p/letrs/docs/letrs.html:1102) lists host-agnostic `Vm` and `crates/world/` as future work, but ADR-017/018 already landed.
- [docs/letrs.html](/Users/matt/projects-new/3p/letrs/docs/letrs.html:1191) footer says 78 tests.

I did not edit anything. My read: the implementation is fresh; the next useful cleanup is a documentation refresh pass across `key_facts.md`, `docs/letrs.html`, and the `lisp` crate-level docs.