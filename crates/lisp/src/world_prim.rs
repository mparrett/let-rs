use crate::val::{Arity, Val};
use crate::world::{Tile, World};

type R = Result<Val, String>;
type WorldPrimFn = fn(&[Val], &mut World) -> R;

fn coord(v: &Val, name: &str) -> Result<u32, String> {
    match v {
        Val::Num(n) if *n >= 0 => Ok(*n as u32),
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
        other => return Err(format!("world-set-tile!: 3rd arg must be a symbol, got {other}")),
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
    Ok(Val::cons(Val::Num(w.width as i64), Val::Num(w.height as i64)))
}

/// `(world-apply! ctx)` — resolver: reads `element`, `tx`, `ty`, optional `area`
/// from a ctx alist, paints a square neighborhood around (tx,ty) with the
/// corresponding tile, and logs the cast. Returns the number of tiles painted.
fn world_apply(args: &[Val], w: &mut World) -> R {
    let ctx = &args[0];
    let element = assoc_get(ctx, "element");
    let tx = assoc_get(ctx, "tx").and_then(as_num).unwrap_or(0);
    let ty = assoc_get(ctx, "ty").and_then(as_num).unwrap_or(0);
    let area = assoc_get(ctx, "area").and_then(as_num).unwrap_or(0).max(0);

    let tile = match element.as_ref() {
        Some(Val::Sym(s)) => Tile::from_sym(s)
            .ok_or_else(|| format!("world-apply!: unknown element '{s}'"))?,
        Some(other) => return Err(format!("world-apply!: element must be a symbol, got {other}")),
        None => return Err("world-apply!: ctx has no 'element".into()),
    };

    let mut painted = 0i64;
    for dy in -area..=area {
        for dx in -area..=area {
            let x = tx + dx;
            let y = ty + dy;
            if x >= 0 && y >= 0 && w.set_tile(x as u32, y as u32, tile) {
                painted += 1;
            }
        }
    }

    w.log_event(format!(
        "cast {} at ({tx},{ty}) area={area} → {painted} tiles",
        tile.as_sym()
    ));
    Ok(Val::Num(painted))
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
    ("world-tile",      Arity::Exact(2),   world_tile),
    ("world-set-tile!", Arity::Exact(3),   world_set_tile),
    ("world-log!",      Arity::AtLeast(1), world_log),
    ("world-size",      Arity::Exact(0),   world_size),
    ("world-apply!",    Arity::Exact(1),   world_apply),
];
