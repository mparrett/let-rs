// letrs · web · shared module
//
// One Vm per tab, plus the bits that don't care which DSL is on the page:
// the COI status chip and the REPL (which is present on both lab pages
// because it's a debugging surface for the underlying lisp, not a third
// concept that competes with the labs).
//
// Page-specific modules (spells.js, genes.js) import { vm, $ } from here.

import init, { Vm } from './pkg/wasm.js';

export const $ = (sel) => document.querySelector(sel);

await init();
export const vm = new Vm(7, 5);

// COI status chip — the whole point is *not* needing crossOriginIsolation.
// Only wires if the chip is on this page (the landing page omits it).
const coiChip = $('#coi-chip');
if (coiChip) {
  if (crossOriginIsolated) {
    coiChip.textContent = 'crossOriginIsolated: yes';
    coiChip.classList.add('coi-yes');
  } else {
    coiChip.textContent = 'crossOriginIsolated: no · still works';
    coiChip.classList.add('coi-no');
  }
}

// REPL wiring (only attaches if the REPL panel is present).
const replEl = $('#repl-input');
const outEl  = $('#out');
if (replEl && outEl) {
  const runRepl = () => {
    const src = replEl.value;
    if (!src.trim()) return;
    const stamp = `> ${src.replace(/\n/g, '\n  ')}\n`;
    try {
      const result = vm.eval(src);
      outEl.textContent += `${stamp}= ${result}\n\n`;
    } catch (e) {
      outEl.textContent += `${stamp}! ${e}\n\n`;
    }
    outEl.scrollTop = outEl.scrollHeight;
  };

  $('#eval-btn')?.addEventListener('click', runRepl);
  $('#clear-out')?.addEventListener('click', () => { outEl.textContent = ''; });

  replEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      runRepl();
    }
  });

  // REPL cheatsheet click-to-load. We scope by `.cheatsheet.repl` (added
  // per-page) so the rune / strand cheatsheets don't accidentally leak
  // their text into the REPL textarea.
  document.querySelectorAll('.cheatsheet.repl code').forEach((el) => {
    el.addEventListener('click', () => {
      replEl.value = el.textContent;
      replEl.focus();
      replEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    });
  });
}
