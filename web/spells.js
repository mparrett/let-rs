// letrs · web · Spell Lab
//
// Rune palette → tape, cast → world. Seeds with a small fire-then-ice
// cast so visitors land on a rendered grid rather than a blank one.

import { vm, $ } from './common.js';

const tapeEl = $('#tape');
const xEl    = $('#x');
const yEl    = $('#y');
const gridEl = $('#grid');
const logEl  = $('#log');

// Map the ASCII glyphs the Rust World::Display emits onto the color
// classes the legend uses. Anything outside this set passes through.
const GLYPH_CLASS = { '.': 'g-floor', '*': 'g-fire', 'o': 'g-ice', '#': 'g-wall' };

// Build the grid as persistent per-cell <span>s so the dissipation
// animation can mutate them in place — using innerHTML on every
// frame would replace the elements wholesale and CSS transitions
// would never fire. Each span carries its (x, y) so the dissipation
// can group cells by Chebyshev ring around the cast center.
// Newlines stay as raw text nodes inside the <pre> to preserve
// layout.
const refresh = () => {
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

// Rune palette → append to tape input. Not calling tapeEl.focus() on
// purpose — programmatic focus pops mobile keyboards, the opposite of
// what we want when the user is composing via the palette buttons.
document.querySelectorAll('.rune').forEach((btn) => {
  if (btn.id === 'clear-tape') return;
  btn.addEventListener('click', () => { tapeEl.value += btn.dataset.rune; });
});

$('#clear-tape').addEventListener('click', () => { tapeEl.value = ''; });

// Dissipation animation: after a successful cast, hold the painted
// grid briefly so the eye can land on the effect, then shrink the
// spell back to its center — outermost Chebyshev ring around the
// cast (cx, cy) flips to floor first, then the next ring in, and
// so on down to the center cell. Each ring fades together (with a
// small per-cell jitter so it doesn't snap in lockstep). Once the
// last ring has flipped, sync the underlying World so a follow-up
// cast starts from a clean slate.
//
// `castToken` is the abort handle: every cast bumps it, every
// scheduled timer checks it. A new cast mid-dissipation cancels the
// in-flight animation cleanly. The per-cell DOM persistence (set up
// by refresh()) is what makes the CSS opacity transition actually
// fire — replacing innerHTML each tick would destroy the elements
// before the browser could animate them.
let castToken = 0;
const HOLD_MS     = 400;  // pause after cast before dissipation begins
const RING_STEP   = 180;  // ms between consecutive rings
const RING_JITTER =  60;  // ms randomization within a ring
const FADE_MS     = 220;  // per-cell opacity transition each direction

const dissipate = (cx, cy) => {
  const myToken = ++castToken;
  setTimeout(() => {
    if (myToken !== castToken) return;
    const cells = Array.from(gridEl.querySelectorAll('span'))
      .filter((s) => s.textContent !== '.')
      .map((span) => {
        const x = Number(span.dataset.x);
        const y = Number(span.dataset.y);
        return { span, ring: Math.max(Math.abs(x - cx), Math.abs(y - cy)) };
      });
    if (cells.length === 0) return;
    const maxRing = cells.reduce((m, c) => Math.max(m, c.ring), 0);
    cells.forEach(({ span, ring }) => {
      // Outermost ring fires at delay 0, innermost at maxRing * step.
      const delay = (maxRing - ring) * RING_STEP + Math.random() * RING_JITTER;
      setTimeout(() => {
        if (myToken !== castToken) return;
        span.style.opacity = '0.12';
        setTimeout(() => {
          if (myToken !== castToken) return;
          span.textContent = '.';
          span.className = 'g-floor';
          span.style.opacity = '1';
        }, FADE_MS);
      }, delay);
    });
    setTimeout(() => {
      if (myToken !== castToken) return;
      vm.reset_world();
      // No refresh() — the DOM already shows all floor; refreshing
      // would rebuild every span and snap a momentary unstyled blink.
    }, maxRing * RING_STEP + RING_JITTER + FADE_MS * 2 + 80);
  }, HOLD_MS);
};

$('#cast-btn').addEventListener('click', () => {
  const tape = tapeEl.value;
  let x, y;
  try {
    x = BigInt(xEl.value || '0');
    y = BigInt(yEl.value || '0');
  } catch {
    logEl.textContent = '⚠ x and y must be integers\n' + vm.log();
    return;
  }
  try {
    vm.cast(tape, x, y);
  } catch (e) {
    logEl.textContent = `⚠ ${e}\n${vm.log()}`;
    gridEl.textContent = vm.grid();
    return;
  }
  refresh();
  // Convert BigInt → Number for ring math. The grid is at most a few
  // hundred cells on each axis, well within Number's safe range.
  dissipate(Number(x), Number(y));
});

$('#reset-btn').addEventListener('click', () => {
  // Cancel any in-flight dissipation so it can't fire vm.reset_world()
  // a second time mid-render and clobber a fresh state the user just
  // wanted reset directly.
  castToken++;
  vm.reset_world();
  refresh();
});

// Rune cheatsheet click-to-load. Scoped by `.cheatsheet.spells` so it
// can't leak into the REPL (handled in common.js with `.cheatsheet.repl`).
document.querySelectorAll('.tape-example').forEach((el) => {
  el.addEventListener('click', () => {
    tapeEl.value = el.dataset.tape;
    tapeEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  });
});

// Seed cast so visitors land on a rendered grid. Deliberately NOT
// followed by dissipate() — the page-load state should hold visible
// until the user does something, rather than evaporating before they
// can read what's on screen.
try {
  vm.cast('ᚠ ᛊ 3 ᛁ', 3n, 2n);
} catch (e) {
  console.warn('seed cast failed:', e);
}
refresh();
