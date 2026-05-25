# Key Facts

## Toolchain

- Rust 1.93+, edition 2024
- Workspace at repo root, members = `["crates/*"]`, resolver = `"3"`
- `just` is the task runner (Homebrew install). No `Makefile`.
- No external dependencies in `crates/lisp/Cargo.toml` — keep it that way.

## Commands

```bash
just              # default → cargo test -p lisp
just test         # run the 26 tests in crates/lisp/tests/eval.rs
just repl         # interactive REPL (examples/repl.rs)
just check        # cargo check --all-targets
just fmt          # cargo fmt --all
cargo run -q -p lisp --example spells   # rune tape → ctx demo
cargo run -q -p lisp --example world    # spell → grid paint demo
```

## Layout

```
letrs/
├── Cargo.toml           workspace root
├── CLAUDE.md            session-orientation
├── justfile
├── crates/
│   └── lisp/            the engine — zero deps
│       ├── src/
│       │   ├── expr.rs           AST
│       │   ├── val.rs            runtime values + Arity
│       │   ├── env.rs            Rc<RefCell<Val>> frames
│       │   ├── k.rs              continuation variants
│       │   ├── step.rs           step + run loop
│       │   ├── prim.rs           pure built-ins
│       │   ├── world.rs          grid + log
│       │   ├── world_prim.rs     WorldPrim primitives
│       │   ├── parse.rs          tokenize + read + compile + qq
│       │   └── lib.rs            Vm + macro expansion
│       ├── tests/eval.rs         26 tests
│       └── examples/
│           ├── repl.rs
│           ├── spells.rs         rune tape → ctx
│           └── world.rs          ctx → world tiles
└── docs/
    ├── letrs.html                single-page narrative tour
    └── project_notes/            this directory
```

## Stats as of 2026-05-25 (day one)

- Source LOC: ~1,560 across 10 files in `crates/lisp/src/`
- Test LOC: 258 in `crates/lisp/tests/eval.rs` (26 tests)
- Example LOC: 247 across 3 files
- Total Rust: ~2,060 LOC
- Dependencies: 0
- Commits: 3 (initial CEK + lisp + spell demo; world state; macros)

## Test highlights

- `tail_calls_dont_grow_the_stack` — counts 100,000 deep, no growth
- `letrec_mutual_recursion` — even?/odd? in terms of each other
- `recursion_via_y_combinator` — factorial without letrec, proves first-class
  lambdas + closures work without sugar
- `macro_thread_first` — `(-> 5 (+ 3) (* 2))` → `16`, defined in lisp
- `macro_calls_macro` — a macro body uses another macro
- `quasiquote_splice` — `\`(1 ,@xs 4)` with `xs = '(2 3)` → `(1 2 3 4)`

## Related repos

- `../xsofy` — the original roguelike whose spell DSL inspired this. Same
  authoring conventions; do not push to either upstream from local sessions.
- `../let-go` — the Go-based Clojure dialect that xsofy runs on. Not used by
  letrs; mentioned only for context.

## Narrative

`docs/letrs.html` is the human-readable tour — open in a browser. Same
typography/aesthetic as xsofy's quest notes, but covers letrs's three slices
(CEK / lisp / spell DSL) and the macro system.
