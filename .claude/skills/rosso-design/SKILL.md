---
name: rosso-design
description: Visual identity for rosso — a sibling in eetu's homebrew web app family. Layers rosso's glyph, wordmark, layout, and voice on top of the shared halo-design tokens. Use when building or styling rosso's UI.
user-invocable: true
---

# rosso-design

Shared tokens + conventions come from `halo-design` — copy `colors_and_type.css`
verbatim (already at `frontend/src/lib/styles/halo.css`) and use the `--halo-*`
vars in Svelte `<style>` blocks. Below is rosso's delta.

## The four deltas

**Glyph** — the **broadcast signal**: two concentric arcs radiating from a dot in
the lower-left corner. The dot is the family's warm centre (`#f78f08`), the arcs
are `currentColor` at stroke 3 (outer) / 2.5 (inner), round caps. It reads as the
universal feed mark without being a copy of it — the arcs are open, not filled.
Source: `frontend/static/favicon.svg` (square opaque ground, deliberately not
pre-rounded) and, in `currentColor` form, inline in `Wordmark.svelte`. PNGs via
`frontend/scripts/gen-icons.sh`.

**Wordmark** — `rosso` + accent period. Full riff: *"il buono, il brutto, il
rosso."* — Leone's title with the third adjective swapped, which is the whole
pitch: the app's job is sorting the good from the bad. Collapses to bare `rosso.`
under 640px. Lowercase, Inter 600, `-0.04em`.

**Layout / density** — a **three-pane reader**, dense, keyboard-first. Sources
sidebar (saved views, then folders and feeds with unread counts) → item list →
reader pane. The item list is the hero: one row per story, and each row carries
the LLM's contribution — a score dot, a one-line summary under the title, and a
"4 sources" badge when the row heads a cluster. On mobile the three panes become
a drill-down stack.

**Voice** — terse, lowercase, Italian only in the wordmark. Counts and timestamps
do the talking. Empty states get one quiet line (`no feeds yet`, `nothing
unread`, `no summary — llm is off`). Never explain the LLM's absence twice.

## Differences from the family baseline

| | rosso |
|---|---|
| Hero element | the item list, ranked by score rather than time |
| Accent use | the score dot, the active view in the sidebar, unread counts |
| Density | highest in the family — hundreds of rows, virtualized |
| Degraded mode | every LLM affordance disappears silently when Ollama is off; no error chrome |

## Source-of-truth files

- `frontend/src/lib/styles/halo.css` — canonical tokens (verbatim copy).
- `frontend/src/lib/components/Wordmark.svelte` — brand + glyph.
- `frontend/static/favicon.svg` — the glyph; `scripts/gen-icons.sh` regenerates
  the PNGs.
