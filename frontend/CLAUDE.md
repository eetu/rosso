# frontend

SvelteKit SPA (runes), `adapter-static` in pure-SPA mode, built to `dist/` and
embedded by the Rust backend.

## Conventions

- Design tokens come from `src/lib/styles/halo.css`, copied verbatim from the
  `halo-design` skill. Consume the `--halo-*` vars in scoped `<style>` blocks —
  do not re-derive values by hand.
- `src/lib/api.ts` is the only place that talks to the backend. Types are
  hand-written to mirror the Rust structs in `backend/src/store.rs`; the routes
  they belong to are in `backend/src/api.rs`.
- `src/lib/stores/reader.svelte.ts` owns everything two components both read
  (feeds, the item page, the open item, the active view). Components read it
  directly rather than having it threaded through `+page.svelte` as props.
- Shared reactive state lives in `.svelte.ts` rune modules, not threaded through
  parents as props. Leaf components take data props plus callback props.
- `yarn validate` = typecheck + lint + format. Yarn is vendored at
  `.yarn/releases`; invoke it as `node .yarn/releases/yarn-*.cjs` where no shell
  shim exists (hooks, CI, the justfile).
- Icons: `@lucide/svelte`, imported per-icon. No emoji.

- Per-row verbs live in the shared context menu (`menu.svelte.ts` +
  `ContextMenu.svelte`, copied from nib), never as a row of icons. Right-click
  and the row's `⋯` open the same items. The menu is rendered once in the root
  layout — inside the sidebar it would be clipped by that panel's scroll box.

## Notes that bite

- **`fallback: "index.html"`, not `200.html`.** In pure-SPA mode adapter-static
  emits only the fallback file, and naming it `index.html` is what lets the
  backend serve the same file for `/` and every unmatched path with no extra
  wiring.
- **Installed iOS PWAs need `100vh`, not `100dvh`.** The dynamic viewport is
  stale at cold start and resolves short, leaving a dead band until the device is
  rotated. The root layout detects standalone mode and switches.
- **Hover-only affordances are unreachable on a phone.** Touch has neither hover
  nor right-click, so anything revealed by `li:hover` needs a
  `@media (hover: none)` twin that simply shows it. The row's `⋯` is the only
  route to a feed's verbs there.
- **Icon PNGs are committed.** Regenerate with `./scripts/gen-icons.sh` after
  editing `static/favicon.svg`; the build ships no rasterizer.
