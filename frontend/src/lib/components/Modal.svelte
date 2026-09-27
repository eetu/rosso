<script lang="ts">
  import X from "@lucide/svelte/icons/x";
  import type { Snippet } from "svelte";

  type Props = {
    title: string;
    onclose: () => void;
    /** Overrides `--halo-dialog-width` for a panel that wants to be wider. */
    width?: string;
    /** The body — the only region that scrolls. */
    children: Snippet;
    /** Buttons. Omit it for a dialog that only reports. */
    footer?: Snippet;
  };

  let { title, onclose, width, children, footer }: Props = $props();

  /**
   * Escape closes, bound at the window rather than on the veil.
   *
   * A `keydown` on the backdrop element only fires while that element has focus,
   * which it never has once a click has landed inside the panel — so a dialog
   * wired that way looks like it handles Escape and does not.
   *
   * Capture-phase and swallowed, so the page's own Escape (which closes the open
   * item) does not also fire underneath. The context menu registers its capture
   * handler first, at the root, so a menu opened over a dialog still closes
   * first: one rung of the ladder per press.
   */
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      onclose();
      e.stopPropagation();
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  /** First field if there is one, else the first button — never nothing. */
  function focusFirst(node: HTMLElement) {
    const target = node.querySelector<HTMLElement>(
      "input:not([type='file']), textarea, select, button",
    );
    target?.focus();
  }
</script>

<div
  class="halo-veil"
  role="presentation"
  onclick={onclose}
  oncontextmenu={(e) => e.stopPropagation()}
></div>

<!-- A plain `div`, not a `section`: a sectioning element carrying role="dialog"
     is a landmark claiming to be a widget, which is what the a11y rule objects
     to. The role is the part that matters here. -->
<div
  class="halo-dialog halo-card"
  role="dialog"
  aria-modal="true"
  aria-label={title}
  style:--halo-dialog-width={width}
  use:focusFirst
>
  <header>
    <h2>{title}</h2>
    <button onclick={onclose} aria-label="close"><X size={16} /></button>
  </header>

  <!-- The panel holds its shape; this is what scrolls. Move `overflow-y` up to
       the panel and the header leaves with the content, close button and all. -->
  <div data-body>{@render children()}</div>

  {#if footer}
    <footer>{@render footer()}</footer>
  {/if}
</div>

<style>
  h2 {
    margin: 0;
    font-family: var(--halo-font-heading);
    font-size: 0.8rem;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--halo-text-muted);
  }

  header button {
    display: grid;
    place-items: center;
    padding: 0.25rem;
    border: none;
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  header button:hover {
    color: var(--halo-text-main);
  }
</style>
