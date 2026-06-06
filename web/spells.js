// let-rs · web · Spell Lab
//
// Rune palette → tape → (cast! ...). The world ticks on a
// setInterval, decaying tiles and regenerating mana. Casts that
// exceed the current budget refuse (logged as `mana-short`); casts
// that succeed paint tiles with finite lifetime that the tick loop
// reaps.
//
// History note: pre-ADR-026/027/028, this file simulated decay in
// JS (a Chebyshev-ring fade after a 2s hold) and reset the world
// before every cast so the painted pattern wouldn't accumulate.
// Both pieces went away once the engine grew real tile decay
// (ADR-027) and a mana model (ADR-028) — the world is the source
// of truth now; this script just drives the tick and reads the
// state back.

import { vm, $ } from './common.js';

const tapeEl    = $('#tape');
const xEl       = $('#x');
const yEl       = $('#y');
const gridEl    = $('#grid');
const logEl     = $('#log');
const meterEl   = $('#mana-meter');
const pipsEl    = $('#mana-pips');
const manaCurEl = $('#mana-cur');
const manaMaxEl = $('#mana-max');

// Map the ASCII glyphs the Rust World::Display emits onto the color
// classes the legend uses. Anything outside this set passes through.
const GLYPH_CLASS = {
  '.': 'g-floor', '*': 'g-fire', 'o': 'g-ice', '#': 'g-wall',
  '%': 'g-earth', '~': 'g-water', '&': 'g-mud', '^': 'g-lava',
};

// Read max-mana once at startup. The pip count is fixed (it's a UI
// concern, not a model one); if `(set! max-mana N)` ever fires from
// the REPL we'll surface the count via the trailing numeric without
// rebuilding the pip row.
const MAX_MANA = vm.max_mana();
const LOW_MANA_THRESHOLD = Math.max(1, Math.floor(MAX_MANA * 0.25));

// Build the pip row once.
const pips = [];
for (let i = 0; i < MAX_MANA; i++) {
  const pip = document.createElement('span');
  pip.className = 'pip';
  pipsEl.appendChild(pip);
  pips.push(pip);
}
manaMaxEl.textContent = String(MAX_MANA);

const refreshMana = () => {
  const cur = vm.mana();
  manaCurEl.textContent = String(cur);
  for (let i = 0; i < pips.length; i++) {
    pips[i].classList.toggle('lit', i < cur);
  }
  meterEl.classList.toggle('low', cur > 0 && cur <= LOW_MANA_THRESHOLD);
};

// Build the grid as persistent per-cell <span>s so opacity / color
// transitions in CSS can fire when tiles change. Newlines stay as
// raw text nodes inside the <pre> to preserve layout.
const refreshGrid = () => {
  gridEl.textContent = '';
  let x = 0, y = 0;
  for (const ch of vm.grid()) {
    if (ch === '\n') {
      gridEl.appendChild(document.createTextNode(ch));
      y += 1;
      x = 0;
      continue;
    }
    const span = document.createElement('span');
    span.textContent = ch;
    span.dataset.x = String(x);
    span.dataset.y = String(y);
    const cls = GLYPH_CLASS[ch];
    if (cls) span.className = cls;
    gridEl.appendChild(span);
    x += 1;
  }
  logEl.textContent = vm.log();
};

const refresh = () => {
  refreshGrid();
  refreshMana();
};

// Rune palette → append to tape input. Not calling tapeEl.focus() on
// purpose — programmatic focus pops mobile keyboards, the opposite of
// what we want when the user is composing via the palette buttons.
document.querySelectorAll('.rune').forEach((btn) => {
  if (btn.id === 'clear-tape') return;
  btn.addEventListener('click', () => { tapeEl.value += btn.dataset.rune; });
});

$('#clear-tape').addEventListener('click', () => { tapeEl.value = ''; });

// Fizzle: spawn a transient `?` ghost rising from the target cell.
// Used to signal "tried but didn't paint" — either mana-short or an
// eval error. CSS handles the animation; we just plant the element
// and let it self-remove.
const fizzle = (x, y) => {
  const cell = gridEl.querySelector(`span[data-x="${x}"][data-y="${y}"]`);
  if (!cell) return;
  const cRect = cell.getBoundingClientRect();
  const gRect = gridEl.getBoundingClientRect();
  const ghost = document.createElement('span');
  ghost.textContent = '?';
  ghost.className = 'fizzle-ghost';
  ghost.style.left = `${cRect.left - gRect.left + cRect.width / 2}px`;
  ghost.style.top  = `${cRect.top  - gRect.top}px`;
  gridEl.appendChild(ghost);
  ghost.addEventListener('animationend', () => ghost.remove(), { once: true });
};

const flashMana = () => {
  // Restart the animation cleanly by toggling the class off-then-on.
  meterEl.classList.remove('flash');
  void meterEl.offsetWidth;  // force reflow so the class re-add restarts the animation
  meterEl.classList.add('flash');
};

const doCast = () => {
  const tape = tapeEl.value;
  let x, y;
  try {
    x = BigInt(xEl.value || '0');
    y = BigInt(yEl.value || '0');
  } catch {
    logEl.textContent = '⚠ x and y must be integers\n' + vm.log();
    fizzle(Number(xEl.value) || 0, Number(yEl.value) || 0);
    return;
  }
  // No pre-cast reset — the world accumulates. Tiles painted on
  // previous casts that are still within their lifetime stay until
  // they decay. A `(cast! …)` that's refused for mana logs a
  // `mana-short` event and paints nothing; both outcomes are
  // visible via the refresh.
  const manaBefore = vm.mana();
  try {
    vm.cast(tape, x, y);
  } catch (e) {
    logEl.textContent = `⚠ ${e}\n${vm.log()}`;
    gridEl.textContent = vm.grid();
    fizzle(Number(x), Number(y));
    return;
  }
  // Mana unchanged ⇒ cast! refused for mana-short (the only path that
  // returns without decrementing). Distinguishes from a successful
  // cast that painted zero tiles for legitimate reasons.
  const refused = vm.mana() === manaBefore;
  refresh();
  if (refused) {
    fizzle(Number(x), Number(y));
    flashMana();
  }
};

$('#cast-btn').addEventListener('click', doCast);

$('#reset-btn').addEventListener('click', () => {
  // Resets world tiles + mana (the bridge calls reset-mana! after
  // rebuilding the world). The tick loop keeps running — the next
  // tick fires on schedule.
  vm.reset_world();
  refresh();
});

// Rune cheatsheet click-to-load-and-cast. Scoped by `.cheatsheet.spells`
// so it can't leak into the REPL (handled in common.js with
// `.cheatsheet.repl`). The cast fires immediately at the current x/y —
// turning the cheatsheet into a live "try it" panel rather than a
// passive reference. If the cast fizzles for mana, the fizzle UI
// shows it; the user can hit reset or wait for tick regen.
document.querySelectorAll('.tape-example').forEach((el) => {
  el.addEventListener('click', () => {
    tapeEl.value = el.dataset.tape;
    doCast();
  });
});

// Tick loop. Drives both halves of the temporal model: tile decay
// (ADR-027) reverts expired tiles to floor; mana regen (ADR-028)
// adds one point per tick up to the cap. 600ms feels alive without
// being twitchy — at default lifetime 5, a painted tile takes
// ~3 seconds to fade.
const TICK_MS = 600;
const interval = setInterval(() => {
  try {
    vm.tick();
  } catch (e) {
    // A tick failure is recoverable (the next tick will retry); log
    // it once and don't stop the loop.
    console.warn('tick failed:', e);
  }
  refresh();
}, TICK_MS);

// If the tab is hidden, pause ticking to avoid silent mana regen +
// log entries piling up while the user isn't looking. (`visibilitychange`
// is one of the rare events fast enough that we can react without a
// debounce.)
window.addEventListener('visibilitychange', () => {
  // Easy path: clear + restart. Avoids cumulative drift from a long
  // hidden window suddenly catching up.
  if (document.hidden) {
    clearInterval(interval);
  }
  // We don't restart on visible — the user reloading or interacting
  // is the natural recovery. Keeping things simple matters more than
  // perfect symmetry here.
});

// First render uses the freshly-constructed empty world. No seed
// cast — visitors land on a clean grid and start with full mana, so
// the first thing they do is their own.
refresh();
