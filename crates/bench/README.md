# let-rs benchmarks

Criterion-driven benchmark suite for the lisp engine and the
spell/genes demos. Lives in its own crate so the core `lisp` crate
stays zero-dep.

## Running

```bash
just bench                      # all benches, default sample sizes (slow but accurate)
just bench -- --quick           # ~3-5× faster, less statistically tight
just bench --bench core         # core engine only
just bench --bench demos        # demos only
just bench cast_spell           # filter by name substring
```

Criterion writes HTML reports under `target/criterion/<bench>/report/`
(gitignored). Open in a browser for the full breakdown.

## Workflow: comparing before/after a refactor

```bash
# Before the change:
just bench -- --save-baseline pre

# After the change:
just bench -- --baseline pre
```

Criterion prints a delta vs `pre` for every bench, with statistical
significance flags. This is the intended use — let-rs benchmarks
aren't about hitting an absolute number, they're about catching
regressions when the engine changes.

## What's covered

**`core`** — pure engine, no DSL vocabulary. A regression here
points at the CEK loop, env, parser, or pure prims.

- `tail_call_loop_10k` — tail-call optimization under a 10k-iter
  countdown loop.
- `letrec_mutual_500` — `even?/odd?` mutual recursion; placeholder-
  cell pattern.
- `env_deep_lookup_30` — 30 nested lets, look up the outermost var;
  env-chain traversal cost.
- `list_map_1000` — square 1000 numbers via user-defined `map`;
  closure application + cons allocation.
- `assoc_get_at_50` — 50-entry alist lookup; new `assoc-get` prim.
- `arith_int_fold_100` — `(+ 1 … 100)`; integer fast path.
- `arith_ratio_fold_99` — `(+ 1/2 1/3 … 1/100)`; rational
  promotion + gcd normalize on every step.
- `parser_define_chain_40` — 40 top-level defines + a trivial body;
  separates parser regressions from eval regressions.
- `macro_thread_chain_8` — `->` thread-first macro over an 8-stage
  pipeline; macro expansion hot path.

**`demos`** — DSL end-to-end. The Vm and prelude install happen
once per setup via `iter_batched`, so iteration measures only the
per-cast cost (not installation).

- `cast_spell_canonical` — `ᚦ ᛞ 3 ᛇ` over a fresh 8×5 world.
- `cast_genome_balanced` — one allele per trait, no mutation.
- `cast_genome_with_mut` — same strand with a `MUT` codon;
  rational-rate `mutate!` path.
- `breed_diploid` — full diploid × diploid cross.
