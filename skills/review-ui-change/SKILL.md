---
name: review-ui-change
description: Review the review console and the pg-probe.dev site after a change to what pg-probe shows, and update them when they no longer match. Use when a change touches pg-probe top (src/top.rs, src/top/), the capture progress (src/progress.rs), the review console (web/), the printed report, the MCP server (src/mcp.rs, schema/catalog.md, schema/error-codes.md), or the version, before committing it.
---

# Review the console and the site after a UI change

The site shows real pg-probe screens, and the console and `top` name the same
things. A change to one surface leaves the others stale unless someone checks.

## 1. Find what the change reaches

| Change in | Shows on the site in | How to refresh it |
|---|---|---|
| `src/top.rs`, `src/top/` | `site/src/assets/top/*.html`, drawn by `TopDemo.astro` | `cargo build --release --locked`, then `scripts/top-screens.sh` |
| `src/progress.rs` | `site/src/components/TermCapture.astro` | Copy the last frame of a real `pg-probe capture` by hand |
| `web/` (the console) | `site/public/demo/console/`, `ConsoleCards.astro` | A static copy of the page `pg-probe analyze` serves; see `site/README.md` |
| The printed report | `site/public/demo/report/*.pdf`, `site/src/assets/shots/report-page-*.png` | Print from the console, then `pdftoppm`; see `site/README.md` |
| `src/mcp.rs`, `schema/catalog.md`, `schema/error-codes.md`, the `mcp` flags in `src/main.rs` | The MCP section in `site/src/i18n/*.ts` (`agent`), `AgentChat.astro`, `site/public/llms.txt`, and "Ask an AI agent" in `README.md` | Edit the text by hand. `AgentChat.astro` is a real Claude Code session on the demo capture: when a tool name, a limit or the server instructions change, run the question again on that capture and copy the new calls and answer |
| The version in `Cargo.toml` | The badge in every capture above | Refresh all of them |

A change can also reach the text around the screens. Search
`site/src/i18n/` for the view, key, column or term that changed. The notes
under the `top` demo and the console cards describe features in words.

## 2. Review the console against `top`

`top` and the console share Kiso and name the same metrics. When the change
renames a term, a state, a unit or a key, or changes a color role, check the
same concept in `web/src/`, `web/src/i18n.ts` and `web/src/locales/`. Make
them match, or say why they differ.

## 3. Update what no longer matches

- Regenerate captures with the scripts. Do not edit captured HTML by hand.
- Change site text in `en.ts` first, then `pt-BR.ts` and `es.ts`. The type of
  `en.ts` makes the other files carry every key.
- Follow `docs/kiso.md` for screen changes, and update it when a rule changes.

## 4. Check the result

```sh
cd site && pnpm check && pnpm build && pnpm preview
```

Open the `top` section and each changed view. The screens are 120 columns
wide. Check that no row is cut at the right edge and that the boxes, rules
and selection marks line up.

## 5. Report

List each surface you checked, what you changed, and what you left stale on
purpose. A change users can see in the binary still needs a changeset; see
`docs/releases.md`. The site alone does not.
