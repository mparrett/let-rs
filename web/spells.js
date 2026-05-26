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

const colorizeGrid = (text) => {
  let html = '';
  for (const ch of text) {
    const cls = GLYPH_CLASS[ch];
    html += cls ? `<span class="${cls}">${ch}</span>` : ch;
  }
  return html;
};

const refresh = () => {
  // innerHTML is safe here: source is our own wasm grid, glyph set is
  // closed and free of HTML-meaningful characters.
  gridEl.innerHTML  = colorizeGrid(vm.grid());
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
});

$('#reset-btn').addEventListener('click', () => {
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

// Seed cast so visitors land on a rendered grid.
try {
  vm.cast('ᚠ ᛊ 3 ᛁ', 3n, 2n);
} catch (e) {
  console.warn('seed cast failed:', e);
}
refresh();
