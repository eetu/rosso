// Pure SPA: no SSR, no prerendering. The Rust backend serves index.html for
// every unmatched path and the client router takes it from there.
export const ssr = false;
export const prerender = false;
