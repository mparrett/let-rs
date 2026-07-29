// let-rs · web · shared module
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

// CEK steps per animation frame. The engine is a state machine we can
// stop between any two transitions (ADR-040), so the REPL evaluates in
// slices instead of blocking: the page keeps painting, a step counter
// ticks, and `cancel` actually works. 50k is comfortably inside a frame
// on anything that can run this page; the Vm's own step budget still
// catches a runaway independently of how the host slices it.
const SLICE = 50_000;

const nextFrame = () => new Promise(requestAnimationFrame);

// REPL wiring (only attaches if the REPL panel is present).
const replEl = $('#repl-input');
const outEl  = $('#out');
if (replEl && outEl) {
  const evalBtn   = $('#eval-btn');
  const cancelBtn = $('#cancel-btn');
  const statusEl  = $('#eval-status');
  const idleHint  = statusEl?.textContent ?? '';
  let running = false;

  const setRunning = (on) => {
    running = on;
    if (evalBtn) evalBtn.disabled = on;
    if (cancelBtn) cancelBtn.hidden = !on;
    if (statusEl && !on) statusEl.textContent = idleHint;
  };

  const runRepl = async () => {
    // Guard rather than queue: a second submit while one is in flight
    // would need a policy (cancel? enqueue?) and the cancel button is
    // already the explicit way to say "stop".
    if (running) return;
    const src = replEl.value;
    if (!src.trim()) return;
    const stamp = `> ${src.replace(/\n/g, '\n  ')}\n`;
    setRunning(true);
    try {
      // Reading and macro expansion happen here, so a syntax error
      // throws before we start slicing.
      vm.eval_start(src);
      let done = false;
      let result = '';
      while (vm.eval_pending()) {
        const r = vm.eval_resume(SLICE);
        // `null`/`undefined` means "paused, more to do"; anything else is
        // the finished value — including the empty string, so this can't
        // be a falsiness check.
        if (r !== null && r !== undefined) {
          done = true;
          result = r;
          break;
        }
        if (statusEl) {
          const steps = vm.eval_steps().toLocaleString();
          statusEl.textContent = `running… ${steps} steps`;
        }
        await nextFrame();
      }
      // Falling out of the loop without a value means the session was
      // cancelled between frames.
      outEl.textContent += done
        ? `${stamp}= ${result}\n\n`
        : `${stamp}! cancelled\n\n`;
    } catch (e) {
      outEl.textContent += `${stamp}! ${e}\n\n`;
    } finally {
      setRunning(false);
      outEl.scrollTop = outEl.scrollHeight;
    }
  };

  evalBtn?.addEventListener('click', runRepl);
  // Whatever already ran stands — completed defines keep their values and
  // host effects aren't undone. The Vm stays usable, which is the point.
  cancelBtn?.addEventListener('click', () => vm.eval_cancel());
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
