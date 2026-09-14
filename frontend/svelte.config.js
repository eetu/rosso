import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  compilerOptions: {
    // Force runes mode (Svelte 5). Can be removed in Svelte 6.
    runes: ({ filename }) => (filename.split(/[/\\]/).includes("node_modules") ? undefined : true),
  },
  kit: {
    // Pure SPA, no server logic. Output to dist/ so the Rust backend embeds it,
    // and name the fallback index.html so the backend's SPA handler serves the
    // same file for / and every unmatched path with no extra wiring.
    adapter: adapter({
      pages: "dist",
      assets: "dist",
      fallback: "index.html",
      precompress: false,
      strict: true,
    }),
    // SvelteKit's built-in version poll: `version.name` defaults to a fresh
    // build timestamp, so it flips on every :main rebuild — which is the signal
    // that matters here, since the SPA and backend ship in one image and a
    // deploy restarts both. The root layout reloads on it.
    version: {
      pollInterval: 60_000,
    },
  },
};

export default config;
