use std::cell::RefCell;
use std::rc::Rc;

use lisp::Vm;
use lisp::val::{Arity, Val};

use crate::{Tile, World};

type R = Result<Val, String>;
type WorldPrimFn = fn(&[Val], &mut World) -> R;

fn coord(v: &Val, name: &str) -> Result<u32, String> {
    match v {
        Val::Num(n) => {
            u32::try_from(*n).map_err(|_| format!("{name}: coord out of u32 range, got {n}"))
        }
        _ => Err(format!("{name}: expected non-negative integer, got {v}")),
    }
}

fn world_tile(args: &[Val], w: &mut World) -> R {
    let x = coord(&args[0], "world-tile")?;
    let y = coord(&args[1], "world-tile")?;
    match w.tile_at(x, y) {
        Some(t) => Ok(Val::Sym(t.as_sym().into())),
        None => Ok(Val::Nil),
    }
}

fn world_set_tile(args: &[Val], w: &mut World) -> R {
    let x = coord(&args[0], "world-set-tile!")?;
    let y = coord(&args[1], "world-set-tile!")?;
    let kind = match &args[2] {
        Val::Sym(s) => {
            Tile::from_sym(s).ok_or_else(|| format!("world-set-tile!: unknown tile '{s}'"))?
        }
        other => {
            return Err(format!(
                "world-set-tile!: 3rd arg must be a symbol, got {other}"
            ));
        }
    };
    Ok(Val::Bool(w.set_tile(x, y, kind)))
}

fn world_log(args: &[Val], w: &mut World) -> R {
    let msg = args
        .iter()
        .map(|v| format!("{v}"))
        .collect::<Vec<_>>()
        .join(" ");
    w.log_event(msg);
    Ok(Val::Bool(true))
}

fn world_size(_args: &[Val], w: &mut World) -> R {
    Ok(Val::cons(
        Val::Num(w.width as i64),
        Val::Num(w.height as i64),
    ))
}

/// Default lifetime for painted tiles when ctx doesn't carry an explicit
/// `power`. Five ticks gives the lab UI a visible-but-brief decay window
/// at the default 500ms interval (~2.5s for fire to fade). See ADR-027.
const DEFAULT_LIFETIME: u8 = 5;

/// `(world-apply! ctx)` — resolver: reads `element`, `tx`, `ty`, optional
/// `area`, `duration`, and `power` from a ctx alist, paints a square
/// neighborhood around (tx,ty) with the corresponding tile (each painted
/// tile carries a decay countdown), and logs the cast. Returns the number
/// of tiles painted.
///
/// Lifetime selection (ADR-027, refined by ᛃ duration rune): the explicit
/// `duration` key wins if present; otherwise `power` falls back (the
/// pre-duration behavior); otherwise `DEFAULT_LIFETIME`. The fallback
/// chain keeps every previous cast site working unchanged while letting
/// new casts separate cost-knobs from lifetime-knobs.
///
/// Each value is clamped to `u8` (0..=255). Negative / zero values mean
/// "permanent" — useful for an opt-out and preserves the "lifetime 0 =
/// permanent" convention.
fn world_apply(args: &[Val], w: &mut World) -> R {
    let ctx = &args[0];
    let element = assoc_get(ctx, "element");
    let tx = assoc_get(ctx, "tx").and_then(as_num).unwrap_or(0);
    let ty = assoc_get(ctx, "ty").and_then(as_num).unwrap_or(0);
    let area = assoc_get(ctx, "area").and_then(as_num).unwrap_or(0).max(0);
    let lifetime_from = |key: &str| -> Option<u8> {
        assoc_get(ctx, key).and_then(as_num).map(|n| {
            if n > 0 {
                n.min(u8::MAX as i64) as u8
            } else {
                0 // negative or zero = permanent
            }
        })
    };
    let lifetime = lifetime_from("duration")
        .or_else(|| lifetime_from("power"))
        .unwrap_or(DEFAULT_LIFETIME);

    let tile = match element.as_ref() {
        Some(Val::Sym(s)) => {
            Tile::from_sym(s).ok_or_else(|| format!("world-apply!: unknown element '{s}'"))?
        }
        Some(other) => {
            return Err(format!(
                "world-apply!: element must be a symbol, got {other}"
            ));
        }
        None => return Err("world-apply!: ctx has no 'element".into()),
    };

    // Clamp the paint region to the grid intersection. Pre-fix a tape
    // like `ᚦ ᛞ 1000000000` drove ~4e18 iterations on a 7×5 grid; now
    // we only walk the actual rectangle that lands inside the world.
    // Saturating arithmetic on i64 lets `area = i64::MAX` survive without
    // overflowing the bounds computation.
    let mut painted = 0i64;
    if w.width > 0 && w.height > 0 {
        let w_max = (w.width - 1) as i64;
        let h_max = (w.height - 1) as i64;
        let x_lo = tx.saturating_sub(area).max(0);
        let x_hi = tx.saturating_add(area).min(w_max);
        let y_lo = ty.saturating_sub(area).max(0);
        let y_hi = ty.saturating_add(area).min(h_max);
        if x_lo <= x_hi && y_lo <= y_hi {
            for y in y_lo..=y_hi {
                for x in x_lo..=x_hi {
                    if w.set_tile_with_lifetime(x as u32, y as u32, tile, lifetime) {
                        painted += 1;
                    }
                }
            }
        }
    }

    w.log_event(format!(
        "cast {} at ({tx},{ty}) area={area} life={lifetime} → {painted} tiles",
        tile.as_sym()
    ));
    Ok(Val::Num(painted))
}

/// `(world-tick!)` — advance the world by one tick. Every tile with a
/// positive lifetime decrements; tiles that hit zero revert to Floor.
/// Returns the number of tiles that reverted this tick. Permanent
/// tiles (lifetime 0) are untouched. No args.
fn world_tick(_args: &[Val], w: &mut World) -> R {
    let reverted = w.tick();
    if reverted > 0 {
        w.log_event(format!("tick → {reverted} reverted"));
    }
    Ok(Val::Num(reverted as i64))
}

fn assoc_get(ctx: &Val, key: &str) -> Option<Val> {
    let mut cur = ctx;
    loop {
        match cur {
            Val::Cons(head, tail) => {
                if let Val::Cons(k, v) = head.as_ref()
                    && let Val::Sym(s) = k.as_ref()
                    && &**s == key
                {
                    return Some((**v).clone());
                }
                cur = tail.as_ref();
            }
            _ => return None,
        }
    }
}

fn as_num(v: Val) -> Option<i64> {
    match v {
        Val::Num(n) => Some(n),
        _ => None,
    }
}

pub const WORLD_PRIMS: &[(&str, Arity, WorldPrimFn)] = &[
    ("world-tile", Arity::Exact(2), world_tile),
    ("world-set-tile!", Arity::Exact(3), world_set_tile),
    ("world-log!", Arity::AtLeast(1), world_log),
    ("world-size", Arity::Exact(0), world_size),
    ("world-apply!", Arity::Exact(1), world_apply),
    ("world-tick!", Arity::Exact(0), world_tick),
];

/// Register every entry in [`WORLD_PRIMS`] as a state-capturing closure
/// over `world`. Hosts that want tile-grid prims call this once at Vm
/// construction; the lisp crate no longer auto-installs anything host-
/// specific (ADR-017).
pub fn install(vm: &mut Vm, world: Rc<RefCell<World>>) {
    for &(name, arity, f) in WORLD_PRIMS {
        let world = world.clone();
        vm.register_prim(name, arity, move |args| {
            let mut w = world.borrow_mut();
            f(args, &mut w)
        });
    }
}
