//! Interactive REPL, plus a stepping debugger over ADR-040's pausable
//! machine (`:step`).
//!
//! The stepper is here because it costs almost nothing: a CEK machine is
//! already a state machine whose whole state is one value, so "show me
//! the next transition" is `Machine::step_once` plus a print. What makes
//! it *readable* is ADR-039's spans — the position of the expression in
//! flight and the chain of enclosing call sites are both real source
//! locations, so the view is the user's own text with a caret in it.

use std::io::{self, BufRead, Write};

use lisp::{Progress, Session, Span, render_span};
use macros::MacroVm;

const HELP: &str = "\
  :step <expr>   evaluate one CEK transition at a time
  :help          this
  <expr>         evaluate normally
in :step mode — <enter> one step · r run to the end · q abandon";

/// A stepping session: the paused machine plus the source it came from,
/// which the display needs in order to point into it.
struct Stepping {
    session: Session,
    src: String,
}

fn main() {
    // MacroVm wraps lisp::Vm + a macro expander so the REPL supports
    // `defmacro` and quasiquote-with-macros (ADR-024).
    let mut vm = MacroVm::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut stepping: Option<Stepping> = None;

    eprintln!("let-rs — :help for commands, Ctrl-D to exit");
    loop {
        eprint!("{} ", if stepping.is_some() { "step>" } else { ">" });
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
        let line = line.trim_end_matches('\n');

        // In step mode the blank line is the interesting input, so this
        // has to come before the skip-empty-lines check below.
        if let Some(mut st) = stepping.take() {
            stepping = advance(&mut vm, st_command(line), &mut st).then_some(st);
            continue;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == ":help" {
            eprintln!("{HELP}");
            continue;
        }
        if let Some(expr) = line.strip_prefix(":step") {
            let src = expr.trim().to_string();
            if src.is_empty() {
                eprintln!("usage: :step <expr>");
                continue;
            }
            match vm.start(&src) {
                Ok(session) => {
                    // `start` reads and expands but runs nothing, so the
                    // first <enter> takes step 1 of the first form.
                    let st = Stepping { session, src };
                    show(&st);
                    stepping = Some(st);
                }
                Err(e) => println!("error: {}", e.render(&src)),
            }
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

enum Cmd {
    Step,
    Run,
    Abandon,
}

fn st_command(line: &str) -> Cmd {
    match line.trim() {
        "q" => Cmd::Abandon,
        "r" => Cmd::Run,
        _ => Cmd::Step,
    }
}

/// Apply one stepper command. Returns whether the session is still live.
fn advance(vm: &mut MacroVm, cmd: Cmd, st: &mut Stepping) -> bool {
    let slice = match cmd {
        // Abandoning mid-session is safe and deliberately not rolled
        // back: whatever already ran, ran. That's the property that lets
        // a host cancel a runaway computation and keep using its Vm.
        Cmd::Abandon => {
            println!("abandoned after {} forms", st.session.forms_done());
            return false;
        }
        Cmd::Step => 1,
        Cmd::Run => u64::MAX,
    };
    match vm.vm.resume(&mut st.session, slice) {
        Ok(Progress::Done(v)) => {
            println!("=> {v}");
            false
        }
        Ok(Progress::Paused) => {
            show(st);
            true
        }
        Err(e) => {
            println!("error: {}", e.render(&st.src));
            false
        }
    }
}

/// Print where the machine is: step count, continuation depth, the
/// expression in flight (or the value being returned), and the enclosing
/// call sites.
fn show(st: &Stepping) {
    let Some(m) = st.session.machine() else {
        println!(
            "  [{}/{} forms] between forms",
            st.session.forms_done(),
            st.session.forms_total()
        );
        return;
    };

    let what = match m.value() {
        // Apply mode: the machine is handing this value to its
        // continuation. There's no expression to point at.
        Some(v) => format!("returning {v}"),
        None => "evaluating".to_string(),
    };
    println!("  step {} · depth {} · {what}", m.steps(), m.depth());

    // `position` is set on the three span-bearing expressions (`Var`,
    // `App`, `Raise`), so literals and value-passing steps show nothing
    // rather than a made-up location.
    if let Some(span) = m.position() {
        println!("{}", indent(&render_span(&st.src, Some(span), "here")));
    }
    // Read straight off the continuation chain, innermost first. Only
    // `K::App` frames have positions, so this is a call stack even though
    // `if` and `letrec` frames are what make `depth` above larger.
    let trace = m.backtrace();
    if !trace.is_empty() {
        let sites: Vec<String> = trace.iter().take(6).map(Span::to_string).collect();
        println!("    called from {}", sites.join(" < "));
    }
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
