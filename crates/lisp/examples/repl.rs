use std::io::{self, BufRead, Write};

use lisp::Vm;

fn main() {
    let mut vm = Vm::new();
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
            Err(e) => println!("error: {e}"),
        }
    }
}
