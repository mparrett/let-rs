use std::io::{self, BufRead, Write};

use macros::MacroVm;

fn main() {
    // MacroVm wraps lisp::Vm + a macro expander so the REPL supports
    // `defmacro` and quasiquote-with-macros (ADR-024).
    let mut vm = MacroVm::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = stdin.lock();

    eprintln!("let-rs — Ctrl-D to exit");
    loop {
        eprint!("> ");
        stdout.flush().ok();
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                eprintln!();
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match vm.eval_str(line) {
            Ok(v) => println!("{v}"),
            // The user typed this line, so a caret under the offending
            // token is meaningful here (ADR-039). `render` falls back to
            // the plain message when the error has no span.
            Err(e) => println!("error: {}", e.render(line)),
        }
    }
}
