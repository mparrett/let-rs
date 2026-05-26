// letrs · web · Gene Lab
//
// Codon palette → tape, express → creature card. Seeds with a balanced
// strand so the page lands on a rendered card.

import { vm, $ } from './common.js';

const tapeEl = $('#tape-genes');
const cardEl = $('#creature-card');
const seedEl = $('#seed-genes');

// Codon palette appends triplet + space so successive clicks compose a
// readable strand (`AUG CGA UAA`). Same focus-avoidance rule as the
// rune palette: don't call .focus() programmatically.
document.querySelectorAll('.codon').forEach((btn) => {
  btn.addEventListener('click', () => {
    const t = tapeEl.value;
    tapeEl.value = t && !t.endsWith(' ')
      ? `${t} ${btn.dataset.codon}`
      : `${t}${btn.dataset.codon}`;
  });
});

$('#clear-tape-genes').addEventListener('click', () => {
  tapeEl.value = '';
  cardEl.textContent = '';
});

const expressCard = () => {
  const tape = tapeEl.value.trim();
  if (!tape) {
    cardEl.textContent = '(no codons — paste a strand or click some codons above)';
    return;
  }
  let seed;
  try {
    seed = BigInt(seedEl.value || '0');
  } catch {
    cardEl.textContent = '⚠ seed must be an integer';
    return;
  }
  try {
    cardEl.textContent = vm.cast_genome(tape, seed);
  } catch (e) {
    cardEl.textContent = `⚠ ${e}`;
  }
};

$('#express-btn').addEventListener('click', expressCard);

// Evolve: increment seed and re-express. Same parent strand, fresh
// drift each click. Only meaningful when the tape contains MUT.
$('#evolve-btn').addEventListener('click', () => {
  const cur = Number(seedEl.value || '0');
  seedEl.value = String(cur + 1);
  expressCard();
});

// Strand cheatsheet click-to-load. Scoped by `.cheatsheet.genes` so it
// can't leak into the REPL (handled in common.js with `.cheatsheet.repl`).
document.querySelectorAll('.strand-example').forEach((el) => {
  el.addEventListener('click', () => {
    tapeEl.value = el.dataset.tape;
    tapeEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    expressCard();
  });
});

// Seed: the page lands on a rendered card.
expressCard();
