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

check:
    cargo check --all-targets

fmt:
    cargo fmt --all

# ─── WASM bridge ─────────────────────────────────────────────────
#
# Uses the standalone `wasm-bindgen` CLI (ADR-009). Install with:
#   cargo install -f wasm-bindgen-cli --version 0.2.114
# Version must match the `wasm-bindgen` crate pin in crates/wasm/Cargo.toml.

wasm-build:
    cargo build -p wasm --target wasm32-unknown-unknown --release
    wasm-bindgen target/wasm32-unknown-unknown/release/wasm.wasm \
        --target web --out-dir web/pkg

wasm-serve: wasm-build
    @echo "→ open http://localhost:8000"
    python3 -m http.server -d web 8000
