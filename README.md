# let-rs

[![CI](https://github.com/mparrett/let-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/mparrett/let-rs/actions/workflows/ci.yml)

A small functional Lisp built on a [CEK abstract machine](https://en.wikipedia.org/wiki/CEK_Machine),
written in zero-dependency Rust (edition 2024).

The whole engine is five transition rules. The point is that the smallest
interesting thing you can honestly call a programming language fits in a few
hundred lines of Rust — and once you have it, everything else is just
vocabulary. To prove the point, that vocabulary becomes three rune-tape DSLs:
spells, genes, and curves.

## Live

The browser playground (REPL + all three labs, WASM) runs at
**[mparrett.github.io/let-rs](https://mparrett.github.io/let-rs/)** —
[Spell Lab](https://mparrett.github.io/let-rs/spells.html) ·
[Gene Lab](https://mparrett.github.io/let-rs/genes.html) ·
[Curve Lab](https://mparrett.github.io/let-rs/curves.html) ·
[dev log](https://mparrett.github.io/let-rs/let-rs.html).

## Try it

```bash
just repl       # interactive REPL
just spells     # rune tape → context pipeline
just world      # spell paints a 7×5 tile grid
just genes      # codon tape → diploid genome → ASCII creature card
just curves     # stroke tape → L-system → turtle → ASCII canvas
just            # cargo test --workspace
```

For the browser playground (REPL + Spell/Gene/Curve labs, all WASM):

```bash
just wasm-serve  # builds and serves web/ at http://localhost:7670
```

The running dev log lives in `web/let-rs.html` — open it for the narrative of
what's here and why.

## How it works

The five CEK rules live in `crates/lisp/src/step.rs` — read that first; the
rest of the engine is decoration.

- **expr.rs** — AST: `Num | Bool | Var | Quote | Lam | App | If | Letrec | SetBang`
- **val.rs** — runtime values: `Num | Ratio | Bool | Sym | Str | Nil | Cons | Clo | Prim`
- **env.rs** — Rc-linked immutable frames
- **k.rs** — continuations: `Halt | App | If | Letrec | SetBang`
- **step.rs** — `step(State) -> Step` plus the `run` loop
- **prim.rs / parse.rs / lib.rs** — built-ins, reader/compiler, and the `Vm`

On top of the core: closures, `letrec`, `cons`/`quote`, variadic prims,
`let`/`let*`/`cond`, predicates, comparison chains, and procedural macros
(`defmacro` + quasiquote) in a sibling crate.

## Layout

The core `lisp` crate stays zero-dependency. Everything domain-specific is a
sibling crate that depends only on `lisp`:

| crate | what |
|-------|------|
| `lisp` | the CEK engine + reader + Vm (zero deps) |
| `macros` | `defmacro`, procedural expansion, quasiquote |
| `runes` / `codons` / `strokes` | Unicode/ASCII tapes → sexprs (zero deps) |
| `spells` | rune vocabulary + mana model + world prims |
| `genes` | diploid genome vocabulary + phenotype resolver |
| `curves` | L-system rewrite engine + turtle canvas |
| `world` | tile grid host state shared by the spell demos |
| `wasm` | `wasm-bindgen` bridge for the web playground |

A spell/gene/curve "primitive" is just a user-level closure — adding behavior
means adding a closure, not a new engine rule.

## Design notes

Architecture decisions are recorded as ADRs in `docs/project_notes/decisions.md`;
known bugs and config live alongside in `docs/project_notes/`.

## License

MIT
