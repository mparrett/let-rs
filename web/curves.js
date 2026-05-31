// let-rs · web · Curve Lab
//
// Stroke palette → axiom, rules textarea → lisp alist, draw → canvas.
// The bridge expects rules as a pre-built lisp form, so the page module
// owns the per-line `lhs = rhs` → `((F F + F))` translation — keeps the
// Rust side domain-neutral (see ADR-019 / web/curves.html).

import { vm, $ } from './common.js';

const axiomEl  = $('#axiom');
const rulesEl  = $('#rules');
const itersEl  = $('#iters');
const canvasEl = $('#canvas');

// Stroke palette: append the glyph (no space — strokes are single chars
// and the lexer is whitespace-tolerant either way).
document.querySelectorAll('.stroke[data-stroke]').forEach((btn) => {
  btn.addEventListener('click', () => {
    axiomEl.value += btn.dataset.stroke;
  });
});

$('#clear-axiom').addEventListener('click', () => {
  axiomEl.value = '';
});

$('#clear-canvas').addEventListener('click', () => {
  canvasEl.textContent = '';
});

// Parse the rules textarea into a lisp form. Each line is `lhs = rhs`
// where lhs is a single char and rhs is a stroke fragment (no spaces
// expected, but tolerated). Empty input → empty string (the bridge
// converts to `()`, the no-op rules list).
function rulesToSexpr(text) {
  // Tolerate either newlines or `;` as separators. The textarea uses
  // newlines (one rule per line), but cheatsheet `data-rules` payloads
  // use `;` because HTML attribute whitespace can normalize.
  const lines = text.split(/[\n;]/).map((s) => s.trim()).filter(Boolean);
  if (lines.length === 0) return '';
  const parts = lines.map((line) => {
    const eq = line.indexOf('=');
    if (eq < 0) throw new Error(`rule missing '=': "${line}"`);
    const lhs = line.slice(0, eq).trim();
    const rhs = line.slice(eq + 1).trim();
    if (lhs.length !== 1) {
      throw new Error(`rule lhs must be one char, got "${lhs}"`);
    }
    // Strip whitespace from rhs and space-separate each char so it
    // reads as a list of symbols in lisp (`F+F` → `F + F`).
    const rhsSyms = [...rhs].filter((c) => !/\s/.test(c)).join(' ');
    return `(${lhs} ${rhsSyms})`;
  });
  return `(${parts.join(' ')})`;
}

const draw = () => {
  const axiom = axiomEl.value.trim();
  if (!axiom) {
    canvasEl.textContent = '(no axiom — click some stroke glyphs or paste a tape)';
    return;
  }
  let rules;
  try {
    rules = rulesToSexpr(rulesEl.value);
  } catch (e) {
    canvasEl.textContent = `⚠ ${e.message}`;
    return;
  }
  const iters = Math.max(0, Math.floor(Number(itersEl.value) || 0));
  try {
    const canvas = vm.cast_curve(axiom, rules, iters);
    // Empty turtle (e.g., axiom of just `+`) renders to empty string.
    // Show a placeholder so the panel doesn't look broken.
    canvasEl.textContent = canvas.length > 0
      ? canvas
      : '(nothing drawn — axiom contained no F/G strokes)';
  } catch (e) {
    canvasEl.textContent = `⚠ ${e}`;
  }
};

$('#draw-btn').addEventListener('click', draw);

// Stepper: bump iters by ±1 (clamped to the input's min/max) and
// re-draw immediately so the canvas reflects each click without a
// second action. Reads the bounds from the input attributes so the
// JS doesn't duplicate the HTML's clamp range.
const bumpIters = (delta) => {
  const min = Number(itersEl.min || 0);
  const max = Number(itersEl.max || 99);
  const cur = Math.max(0, Math.floor(Number(itersEl.value) || 0));
  const next = Math.min(max, Math.max(min, cur + delta));
  if (next === cur) return;
  itersEl.value = String(next);
  draw();
};
$('#iters-up').addEventListener('click', () => bumpIters(+1));
$('#iters-down').addEventListener('click', () => bumpIters(-1));

// ⌘/Ctrl + ↵ in any of the three input fields fires draw. textarea
// already swallows plain Enter for newlines; the modifier is what
// distinguishes "evaluate" from "type a newline."
[axiomEl, rulesEl, itersEl].forEach((el) => {
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      draw();
    }
  });
});

// Cheatsheet click-to-load: populate axiom + rules + iters and re-draw.
// Scoped by `.cheatsheet.curves` so the REPL examples don't fire this
// handler (the REPL cheatsheet listener in common.js loads into the
// REPL textarea instead).
document.querySelectorAll('.curve-example').forEach((el) => {
  el.addEventListener('click', () => {
    axiomEl.value = el.dataset.axiom || '';
    // Normalize `;`-separated rule payloads back to one-per-line in
    // the textarea so what the user sees matches what they'd type.
    rulesEl.value = (el.dataset.rules || '').split(/\s*;\s*/).filter(Boolean).join('\n');
    itersEl.value = el.dataset.iters || '0';
    axiomEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    draw();
  });
});

// Seed: page lands on a rendered canvas.
draw();
