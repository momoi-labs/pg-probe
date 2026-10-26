#!/usr/bin/env node
// Converts one screen captured with `tmux capture-pane -p -e -N` from ANSI
// to the HTML that site/src/components/TopScreen.astro draws. Reads stdin,
// writes stdout.
//
// Each run of cells with the same style becomes a span with an inline
// style. Box drawing and blocks become `<i class="x ...">` elements that hold
// one no-break space per cell, which the site paints, so lines join across
// rows. A run of the same horizontal character is one element: the browser
// rounds the width of each element, and a rule of separate cells drifts
// against the text. Dots, braille and the pause bars go in plain `<i>`
// cells, since they come from a fallback font. Each row loses its trailing spaces.

import { readFileSync } from 'node:fs';

// The box drawing and block characters top draws, by the class that
// TopScreen.astro paints for each.
const BOXES = {
  '─': 'h', '━': 'H', '│': 'v', '┃': 'V',
  '╭': 'tl', '╮': 'tr', '╰': 'bl', '╯': 'br',
  '├': 'lt', '└': 'lb',
  '▄': 'lo', '▀': 'up', '▌': 'le',
};
// Painted across the whole cell, so a run of them is one element.
const HORIZONTAL = new Set(['h', 'H', 'lo', 'up']);
const DOTS = /[●○‖⠀-⣿]/u;

const hex = (r, g, b) => '#' + [r, g, b].map((n) => Number(n).toString(16).padStart(2, '0')).join('');

function sgr(state, params) {
  const codes = params === '' ? [0] : params.split(';').map(Number);
  for (let i = 0; i < codes.length; i++) {
    const code = codes[i];
    if (code === 0) Object.assign(state, { fg: null, bg: null, bold: false });
    else if (code === 1) state.bold = true;
    else if (code === 22) state.bold = false;
    else if (code === 39) state.fg = null;
    else if (code === 49) state.bg = null;
    else if ((code === 38 || code === 48) && codes[i + 1] === 2) {
      state[code === 38 ? 'fg' : 'bg'] = hex(...codes.slice(i + 2, i + 5));
      i += 4;
    } else {
      throw new Error(`unsupported SGR ${code} in "${params}": top should send 24-bit color only`);
    }
  }
}

const escape = (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c] ?? c;

function cells(text) {
  const chars = [...text];
  let html = '';
  for (let k = 0; k < chars.length; k++) {
    const c = chars[k];
    if (BOXES[c]) {
      let n = 1;
      while (HORIZONTAL.has(BOXES[c]) && chars[k + n] === c) n++;
      html += `<i class="x ${BOXES[c]}">${'\u00a0'.repeat(n)}</i>`;
      k += n - 1;
    } else if (DOTS.test(c)) html += `<i>${c}</i>`;
    else html += escape(c);
  }
  return html;
}

function style({ fg, bg, bold }) {
  return [fg && `color:${fg}`, bg && `background:${bg}`, bold && 'font-weight:700'].filter(Boolean).join(';');
}

// tmux carries the style from the end of one row to the next.
function row(state, line) {
  // Runs of [style, text].
  const runs = [];
  const re = /\x1b\[([0-9;]*)m|([^\x1b])/gu;
  for (const [, params, c] of line.matchAll(re)) {
    if (params !== undefined) {
      sgr(state, params);
      continue;
    }
    const s = style(state);
    const last = runs.at(-1);
    if (last && last[0] === s) last[1] += c;
    else runs.push([s, c]);
  }
  // The row ends at its last cell with text or a background.
  while (runs.length) {
    const last = runs.at(-1);
    if (last[0].includes('background')) break;
    last[1] = last[1].trimEnd();
    if (last[1]) break;
    runs.pop();
  }
  return runs
    .map(([s, text]) => {
      const html = cells(text);
      return s ? `<span style="${s}">${html}</span>` : html;
    })
    .join('');
}

const lines = readFileSync(0, 'utf8').replace(/\n$/, '').split('\n');
const state = { fg: null, bg: null, bold: false };
process.stdout.write(lines.map((line) => row(state, line)).join('\n'));
