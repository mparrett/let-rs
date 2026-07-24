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
const errEl     = $('#cast-error');

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

// Persistent error overlay. The log gets clobbered every tick (~600ms)
// by refresh(), so parse/eval errors dropped into it vanished before
// they could be read. The overlay stays up for ~5s with click-to-
// dismiss; new errors replace the message and reset the timer.
const ERROR_HOLD_MS = 5000;
let errorTimer = null;
const showError = (msg) => {
  errEl.textContent = `⚠ ${msg}`;
  errEl.classList.remove('fading');
  errEl.classList.add('visible');
  if (errorTimer) clearTimeout(errorTimer);
  errorTimer = setTimeout(() => {
    errEl.classList.add('fading');
    setTimeout(() => {
      errEl.classList.remove('visible', 'fading');
      errorTimer = null;
    }, 240);  // match the .cast-error transition duration
  }, ERROR_HOLD_MS);
};
const clearError = () => {
  if (errorTimer) { clearTimeout(errorTimer); errorTimer = null; }
  errEl.classList.remove('visible', 'fading');
};
errEl.addEventListener('click', clearError);

const doCast = () => {
  const tape = tapeEl.value;
  let x, y;
  try {
    x = BigInt(xEl.value || '0');
    y = BigInt(yEl.value || '0');
  } catch {
    showError('x and y must be integers');
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
    // Don't touch gridEl here — the grid state didn't change (the
    // cast aborted before painting), and rewriting its textContent
    // would tear out the per-cell <span>s that fizzle() needs to
    // position the ghost over.
    showError(String(e));
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
  } else {
    // Successful cast — clear any stale error so the overlay doesn't
    // linger past a recovery.
    clearError();
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
let interval = null;

const startTicking = () => {
  if (interval !== null) return;  // already running; don't stack intervals
  interval = setInterval(() => {
    try {
      vm.tick();
    } catch (e) {
      // A tick failure is recoverable (the next tick will retry); log
      // it once and don't stop the loop.
      console.warn('tick failed:', e);
    }
    refresh();
  }, TICK_MS);
};

const stopTicking = () => {
  if (interval === null) return;
  clearInterval(interval);
  interval = null;
};

// If the tab is hidden, pause ticking to avoid silent mana regen +
// log entries piling up while the user isn't looking; resume when it
// comes back. Clearing and re-arming (rather than letting the browser
// throttle us) also avoids the cumulative drift of a long hidden
// window suddenly catching up.
window.addEventListener('visibilitychange', () => {
  if (document.hidden) {
    stopTicking();
  } else {
    startTicking();
  }
});

startTicking();

// First render uses the freshly-constructed empty world. No seed
// cast — visitors land on a clean grid and start with full mana, so
// the first thing they do is their own.
refresh();
