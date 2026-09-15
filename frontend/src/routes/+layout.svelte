<script lang="ts">
  import "$lib/styles/halo.css";

  import { updated } from "$app/state";

  let { children } = $props();

  $effect(() => {
    // SvelteKit's version poll flips this when a new build is deployed. The SPA
    // and backend ship in one image, so a deploy restarts both and a tab left
    // open would otherwise run a stale SPA against a newer backend. Nothing here
    // holds unsaved state, so reloading outright is safe.
    if (updated.current) {
      location.reload();
    }
  });

  $effect(() => {
    // Installed iOS PWAs report a stale dynamic viewport at cold start, so
    // `100dvh` resolves short and leaves a dead band at the bottom until the
    // device is rotated. `100vh` is computed against the static viewport and is
    // correct from launch — and in standalone there is no browser chrome for it
    // to overshoot. A browser tab is the opposite case and keeps `100dvh`.
    const standalone =
      (window.navigator as { standalone?: boolean }).standalone === true ||
      window.matchMedia("(display-mode: standalone)").matches;
    document.documentElement.classList.toggle("standalone", standalone);
  });
</script>

{@render children()}

<style>
  /* halo.css is tokens and primitives, not a reset, so this has to be stated.
     Without it a declared width means the *content* box and padding is added on
     top: the mobile sidebar is `width: 100%` with 0.75rem of padding, so it ran
     1.5rem off the right edge. Every full-width input had the same latent bug. */
  :global(*),
  :global(*::before),
  :global(*::after) {
    box-sizing: border-box;
  }

  /* The body owns the viewport and never scrolls; an inner element does. Keeps a
     phantom page scrollbar from appearing behind full-screen overlays. */
  :global(html),
  :global(body) {
    height: 100svh;
    height: 100dvh;
  }

  :global(html.standalone),
  :global(html.standalone body) {
    height: 100vh;
  }

  :global(body) {
    margin: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    background: var(--halo-body);
    color: var(--halo-text-main);
    font-family: var(--halo-font-body);
  }
</style>
