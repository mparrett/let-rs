default: test

build:
    cargo build

test:
    cargo test --workspace

repl:
    cargo run -q -p lisp --example repl

spells:
    cargo run -q -p lisp --example spells

world:
    cargo run -q -p lisp --example world

genes:
    cargo run -q -p lisp --example genes

curves:
    cargo run -q -p lisp --example curves

check:
    cargo check --all-targets

# Clippy gate. `-D warnings` denies on any lint at warn level (the workspace-
# wide config in Cargo.toml); `--locked` mirrors CI so a stale Cargo.lock
# fails here too. Kept separate from `check` to preserve fast cargo-check
# feedback.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

fmt:
    cargo fmt --all

# Run all benches. Use `just bench-save NAME` (TODO: as needed) to lock
# a baseline before a refactor, then re-run after to see deltas.
# Criterion writes HTML reports under target/criterion/.
bench *args:
    cargo bench -p bench {{args}}

# ─── WASM bridge ─────────────────────────────────────────────────
#
# Three-stage pipeline:
#   1. cargo → wasm32 release binary
#   2. wasm-bindgen → JS glue + processed .wasm
#   3. wasm-opt -Oz → size-optimized .wasm (in place)
#
# Tooling (install once):
#   rustup target add wasm32-unknown-unknown
#   cargo install -f wasm-bindgen-cli --version 0.2.114   # match the crate pin in crates/wasm/Cargo.toml
#   brew install binaryen                                  # provides `wasm-opt`

wasm-build:
    cargo build -p wasm --target wasm32-unknown-unknown --release
    wasm-bindgen target/wasm32-unknown-unknown/release/wasm.wasm \
        --target web --out-dir web/pkg
    wasm-opt -Oz --strip-debug --strip-producers \
        web/pkg/wasm_bg.wasm -o web/pkg/wasm_bg.wasm
    @ls -lh web/pkg/wasm_bg.wasm | awk '{print "  → web/pkg/wasm_bg.wasm:", $5}'

wasm-serve: wasm-build
    @echo "→ open http://localhost:8000"
    python3 -m http.server -d web 8000
