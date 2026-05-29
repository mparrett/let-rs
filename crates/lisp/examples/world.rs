//! World + spell integration demo.
//!
//! A 7×5 grid. Each cast goes:  spell tape → ctx (in lisp) → `(world-apply! ctx)`
//! which reads element/tx/ty/area and paints tiles. The lisp side is the
//! prelude from `spells.rs` minus the rune translation (we hand-write tokens
//! here to focus on the world half).

use std::cell::RefCell;
use std::rc::Rc;

use lisp::{Vm, World};

const PRELUDE: &str = r#"
(letrec ((assoc-set (lambda (k v ctx) (cons (cons k v) ctx)))
         (thread    (lambda (ctx fs)
                      (if (null? fs) ctx
                          (thread ((car fs) ctx) (cdr fs)))))
         (start     (lambda (x y) (assoc-set 'ty y (assoc-set 'tx x '()))))
         (fire      (lambda (ctx) (assoc-set 'element 'fire ctx)))
         (ice       (lambda (ctx) (assoc-set 'element 'ice ctx)))
         (area      (lambda (n)   (lambda (ctx) (assoc-set 'area n ctx)))))
"#;

fn cast(vm: &mut Vm, x: i64, y: i64, tokens: &str) {
    let body = format!("(world-apply! (thread (start {x} {y}) (list {tokens})))");
    let src = format!("{PRELUDE}  {body})");
    match vm.eval_str(&src) {
        Ok(v) => println!("→ {v} tiles painted"),
        Err(e) => println!("→ err: {e}"),
    }
}

fn print_world(world: &Rc<RefCell<World>>) {
    let w = world.borrow();
    print!("{w}");
}

fn print_log(world: &Rc<RefCell<World>>) {
    let w = world.borrow();
    if w.log.is_empty() {
        return;
    }
    println!("log:");
    for entry in &w.log {
        println!("  · {entry}");
    }
}

fn main() {
    let world = Rc::new(RefCell::new(World::new(7, 5).expect("7×5 fits")));
    let mut vm = Vm::new();
    lisp::world_prim::install(&mut vm, world.clone());

    println!("== initial (7×5) ==");
    print_world(&world);

    println!("\n== cast fire at (1,1) ==");
    cast(&mut vm, 1, 1, "fire");
    print_world(&world);

    println!("\n== cast 'fire (area 1)' at (4,2) ==");
    cast(&mut vm, 4, 2, "fire (area 1)");
    print_world(&world);

    println!("\n== cast 'ice (area 2)' at (3,2) ==");
    cast(&mut vm, 3, 2, "ice (area 2)");
    print_world(&world);

    println!();
    print_log(&world);
}
