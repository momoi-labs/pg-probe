# pg-probe brand

The emblem is a solid elephant that sends a probe, three waves. The ear, the
eye, and the tusk are cut out, so they show the background. The wordmark is
"pg-probe" in JetBrains Mono Bold, converted to paths.

## Colors

| Part | On light | On dark |
| --- | --- | --- |
| Elephant and wordmark | `#1b1a1e` | `#eeece9` (Kiso foreground) |
| Probe waves | `#9d7832` | `#cba65e` |

On a light background, the elephant has a stroke 3 units wide in its own
color. A light cut-out in a dark shape looks wider than a dark cut-out in a
light shape, so the stroke narrows the cut-outs to look like the dark files.
The light and `-mono` files have it; the `-dark` files do not.

## Files

| File | Use |
| --- | --- |
| `mark.svg`, `mark-dark.svg` | Emblem in color, on a light or a dark background. |
| `mark-mono.svg` | Emblem in `currentColor`. |
| `lockup.svg`, `lockup-dark.svg`, `lockup-mono.svg` | Emblem and wordmark side by side. |
| `lockup-stacked.svg`, `lockup-stacked-dark.svg`, `lockup-stacked-mono.svg` | Emblem above the wordmark. |
| `wordmark.svg` | The wordmark alone, in `currentColor`. |
| `elephant-mono.svg` | The elephant alone, in `currentColor`. |
| `favicon.svg` | The favicon, a copy of `web/public/favicon.svg`. Keep the two the same. |
| `usage.html` | A page that shows each file in use. Open it in a browser. |

## In the console

- The shell's brand draws the emblem with `PgProbeMark`, 32 px high. The
  elephant takes the text color. The waves keep the colors above, in both
  themes. `docs/kiso.md` records why it replaces Kiso's BrandMark.
- `web/public/favicon.svg` is the emblem in a square viewBox, so it reads at
  16 px. It switches to the dark colors when the system uses a dark theme.

## Use

- Every file is SVG with a viewBox, a transparent background, and no images,
  scripts, IDs, or fonts.
- `currentColor` files take the CSS `color` when inlined. Through `<img>`, they
  are black.
- The emblem is about 5:3, wider than it is high. Below 32 px high, use the
  favicon instead.
- JetBrains Mono is under the SIL Open Font License. Kiso uses it as its code
  font.
