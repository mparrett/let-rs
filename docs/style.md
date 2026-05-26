# Style & Conventions

How letrs is written, day to day. This is a working dev doc, not a
specification — strong recommendations unless enforced by tooling or
explicitly deviated from with a recorded reason.

For general engineering principles see `CLAUDE.md`. For the *why* behind
architecture choices see `docs/project_notes/decisions.md`.

## References

We cherry-pick from these — they are not authoritative for letrs:

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/about.html)
- [Apollo Rust Best Practices](https://github.com/apollographql/rust-best-practices)
- [Rust-Analyzer Style Guide](https://rust-analyzer.github.io/book/contributing/style.html)

Where our practice differs from one of these, the deviation is called out
below or in a numbered ADR.

## Lints & clippy

The clippy gate is the source of truth — anything the tooling enforces is
not a "recommendation," it's the rule.

- Workspace-wide config lives in the root `Cargo.toml` under
  `[workspace.lints.clippy]`. Currently `all = warn (priority -1)`.
- Local: `just lint` (`cargo clippy --workspace --all-targets --locked
  -- -D warnings`). CI runs the same command.
- `unsafe_code = "forbid"` at workspace level. There is no escape hatch —
  if you genuinely need unsafe, talk first.
- `--locked` is part of the gate. A stale `Cargo.lock` fails CI. Bump
  pinned versions deliberately, in their own commit.

### When you must silence a lint

Prefer `#[expect(clippy::xxx, reason = "…")]` over `#[allow(...)]`. When
the warning stops applying, `expect` re-warns and you'll notice. We
currently have zero overrides — keep it that way unless there's a
documented reason.

```rust
// ✅
#[expect(clippy::needless_pass_by_value, reason = "primitive-table signature")]
fn prim(args: Vec<Val>) -> R { ... }

// ❌ no reason, no expectation re-check
#[allow(clippy::needless_pass_by_value)]
fn prim(args: Vec<Val>) -> R { ... }
```

If clippy fights you repeatedly in one spot, the answer is usually to
refactor — not to silence.

## Comments

CLAUDE.md already says "comments explain *why*, not what." This section
just adds prefixes so they're greppable.

| Prefix         | When                                                     |
| -------------- | -------------------------------------------------------- |
| `// SAFETY:`   | Justifying an `unsafe` block. We forbid unsafe, so this should never appear in `crates/`. |
| `// PERF:`     | Non-obvious performance choice or workaround.            |
| `// CONTEXT:`  | Link to an ADR, RFC, or external decision.               |
| `// TODO(issue #N):` | Tracked work item. See "TODOs" below.              |

Doc comments (`///`, `//!`) are not currently enforced by lints. Use them
for public-facing API where the shape isn't obvious from the type. Don't
add doc comments that just restate the function name.

Living comments are an anti-pattern — comments rot, ADRs don't. When in
doubt: write the ADR, link to it from the code, keep the inline comment
to one line.

## TODOs

We do not use GitHub Issues for in-flight work. Local convention:

1. File the issue as `docs/project_incoming/issue_<N>.md` (or `feat_<N>.md`
   for feature work). The skills `6-fix-issue` and `7-build-feature` look
   here when GitHub is unavailable, and we treat that path as primary.
2. Reference in code: `// TODO(issue #N): one-line description`.
3. A naked `// TODO` with no issue link is a smell — either fix it now or
   file the doc.

## Errors

Deliberate deviation from Apollo's chapter 4.

The `lisp`, `runes`, and `codons` crates are zero-dep (ADR-002). That
rules out `thiserror` and `anyhow`. Runtime errors in the interpreter
are `Result<Val, String>` — they surface to the user as lisp
interpreter messages, not as structured library errors that need to
propagate typed across crate boundaries.

- `crates/lisp`: `Result<Val, String>` everywhere. Don't add `From` impls
  or wrapper enums.
- `crates/runes`, `crates/codons`: same.
- `crates/wasm`: same today; could opt into `thiserror` if error surface
  grows, but it's ~120 LOC and doesn't need it yet.
- Tests: use `unwrap()` / `expect("…")` freely. Apollo's "avoid unwrap in
  production" doesn't apply to test code.

We use `?` to bubble errors. We do not use `unwrap()` in non-test code
unless invariants make failure impossible and it's commented.

## Dependencies

Crate-by-crate policy (codified in ADR-002):

| Crate    | Runtime deps allowed?                                |
| -------- | ---------------------------------------------------- |
| `lisp`   | **No.** Zero deps. dev-deps OK for examples.         |
| `runes`  | **No.** Zero deps.                                   |
| `codons` | **No.** Zero deps.                                   |
| `wasm`   | Yes — `wasm-bindgen` and friends, justified by ADR-002's platform-independence caveat. |
| `bench`  | Dev-deps only (criterion).                           |

Adding a dep to a zero-dep crate requires an ADR explaining why the
caveat applies.

## Imports

`rustfmt` doesn't reorder imports for us yet (would need nightly `cargo
+nightly fmt`). Manual convention follows Apollo's order:

1. `std` / `core` / `alloc`
2. External crates
3. Workspace crates (`use lisp::…`)
4. `super::` then `crate::`

A blank line between groups. If you forget, the next person to touch the
file will fix it.

## Testing

- Tests live in `crates/<name>/tests/*.rs` (integration) and inline
  `#[cfg(test)] mod tests` (unit).
- Run `just test` before pushing. CI is the backstop, not the first
  check.
- Apollo recommends doc-tests as living examples. We can add these where
  a function has a non-obvious calling convention; we don't enforce
  coverage.
- `cargo insta` (snapshot testing) is *not* in use. If we want it, it's
  a dep on `lisp` and needs an ADR.

## Formatting

- `just fmt` runs `cargo fmt --all`. Run it before committing.
- Per CLAUDE.md: never mix `cargo fmt` / `cargo clippy --fix` changes
  with feature work in the same commit.
