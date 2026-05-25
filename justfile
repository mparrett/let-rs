default: test

build:
    cargo build

test:
    cargo test -p lisp

repl:
    cargo run -q -p lisp --example repl

check:
    cargo check --all-targets

fmt:
    cargo fmt --all
