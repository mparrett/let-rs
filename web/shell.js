// letrs · web shell
//
// Loads the wasm bridge, instantiates a single Vm bound to a 7×5 world,
// wires DOM events. No bundler, no framework — plain ESM.

import init, { Vm } from './pkg/wasm.js';

const $ = (sel) => document.querySelector(sel);
const tapeEl    = $('#tape');
const xEl       = $('#x');
const yEl       = $('#y');
const gridEl    = $('#grid');
const logEl     = $('#log');
const outEl     = $('#out');
const replEl    = $('#repl-input');
const coiChip   = $('#coi-chip');
const geneTape  = $('#tape-genes');
const cardEl    = $('#creature-card');

await init();
const vm = new Vm(7, 5);

// Map the ASCII glyphs the Rust World::Display emits onto the color classes
// the legend uses. Anything outside this set (newlines, future glyphs) passes
// through as plain text.
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
  // innerHTML is safe here: the source is our own wasm grid, which only
  // emits glyphs from GLYPH_CLASS + newlines.
  gridEl.innerHTML  = colorizeGrid(vm.grid());
  logEl.textContent = vm.log();
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

// ─── Gene Lab ──────────────────────────────────────────────
//
// Codon palette appends triplet + space, so successive clicks compose a
// readable strand (`AUG CGA UAA`). Same focus-avoidance reasoning as the
// rune palette — programmatic focus pops mobile keyboards.
document.querySelectorAll('.codon').forEach((btn) => {
  btn.addEventListener('click', () => {
    const t = geneTape.value;
    geneTape.value = t && !t.endsWith(' ') ? `${t} ${btn.dataset.codon}` : `${t}${btn.dataset.codon}`;
  });
});

$('#clear-tape-genes').addEventListener('click', () => {
  geneTape.value = '';
  cardEl.textContent = '';
});

const expressCard = () => {
  const tape = geneTape.value.trim();
  if (!tape) {
    cardEl.textContent = '(no codons — paste a strand or click some codons above)';
    return;
  }
  try {
    cardEl.textContent = vm.cast_genome(tape);
  } catch (e) {
    cardEl.textContent = `⚠ ${e}`;
  }
};

$('#express-btn').addEventListener('click', expressCard);

// Strand cheatsheet: click loads into the gene tape (not focused — mobile).
document.querySelectorAll('.strand-example').forEach((el) => {
  el.addEventListener('click', () => {
    geneTape.value = el.dataset.tape;
    geneTape.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    expressCard();
  });
});

// Seed casts so first-time visitors see both pipelines rendered immediately.
try {
  vm.cast('ᚠ ᛊ 3 ᛁ', 3n, 2n);
  refresh();
} catch (e) {
  console.warn('seed cast failed:', e);
}
expressCard();
