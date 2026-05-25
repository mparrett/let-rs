// letrs · web shell
//
// Loads the wasm bridge, instantiates a single Vm bound to a 7×5 world,
// wires DOM events. No bundler, no framework — plain ESM.

import init, { Vm } from './pkg/wasm.js';

const $ = (sel) => document.querySelector(sel);
const tapeEl  = $('#tape');
const xEl     = $('#x');
const yEl     = $('#y');
const gridEl  = $('#grid');
const logEl   = $('#log');
const outEl   = $('#out');
const replEl  = $('#repl-input');
const coiChip = $('#coi-chip');

await init();
const vm = new Vm(7, 5);

const refresh = () => {
  gridEl.textContent = vm.grid();
  logEl.textContent  = vm.log();
};
refresh();

// COI status chip — the whole point of this exercise is *not* needing it.
if (crossOriginIsolated) {
  coiChip.textContent = 'crossOriginIsolated: yes';
  coiChip.classList.add('coi-yes');
} else {
  coiChip.textContent = 'crossOriginIsolated: no · still works';
  coiChip.classList.add('coi-no');
}

// Rune palette → append to tape input.
//
// Deliberately *not* calling tapeEl.focus() here — on mobile, programmatic
// focus pops the soft keyboard, which is the opposite of what we want when
// the user is composing a spell via the rune buttons. Tapping the field
// directly still focuses it normally (default browser behavior).
document.querySelectorAll('.rune').forEach((btn) => {
  if (btn.id === 'clear-tape') return;
  btn.addEventListener('click', () => {
    tapeEl.value += btn.dataset.rune;
  });
});

$('#clear-tape').addEventListener('click', () => {
  tapeEl.value = '';
});

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
    // Surface the error in the log pane, then re-render so the existing world
    // (if any) is still visible.
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

// REPL.
$('#eval-btn').addEventListener('click', () => {
  const src = replEl.value;
  if (!src.trim()) return;
  // Indent multiline source for readability in the history pane.
  const stamp = `> ${src.replace(/\n/g, '\n  ')}\n`;
  try {
    const result = vm.eval(src);
    outEl.textContent += `${stamp}= ${result}\n\n`;
  } catch (e) {
    outEl.textContent += `${stamp}! ${e}\n\n`;
  }
  outEl.scrollTop = outEl.scrollHeight;
});

$('#clear-out').addEventListener('click', () => {
  outEl.textContent = '';
});

// Click-to-load: rune-tape examples populate the tape; lisp examples populate
// the REPL. The two cheatsheets carry different selectors so a tape that
// happens to contain valid lisp punctuation can't leak into the wrong field.
document.querySelectorAll('.tape-example').forEach((el) => {
  el.addEventListener('click', () => {
    tapeEl.value = el.dataset.tape;
    // Scroll into view so the user can see what got loaded, but don't focus —
    // same mobile-keyboard concern as the rune palette handler above.
    tapeEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  });
});
document.querySelectorAll('.cheatsheet:not(.spells) code').forEach((el) => {
  el.addEventListener('click', () => {
    replEl.value = el.textContent;
    replEl.focus();
    replEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  });
});

// Cmd/Ctrl-Enter from the REPL textarea evaluates.
replEl.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    $('#eval-btn').click();
  }
});

// Render a seed cast so first-time visitors see the spell pipeline in action.
try {
  vm.cast('ᚠ ᛊ 3 ᛁ', 3n, 2n);
  refresh();
} catch (e) {
  console.warn('seed cast failed:', e);
}
