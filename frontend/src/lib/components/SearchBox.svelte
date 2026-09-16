<script lang="ts">
  import SearchIcon from "@lucide/svelte/icons/search";
  import X from "@lucide/svelte/icons/x";

  import { reader } from "$lib/stores/reader.svelte";

  let value = $state(reader.q);
  let field: HTMLInputElement | null = $state(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  /**
   * A keystroke is not a query. Every letter typed would otherwise be a full-text
   * scan of the archive and a list that reflows under the cursor, so the box
   * holds what you typed and the store only hears about it once you pause.
   */
  function schedule(next: string) {
    value = next;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void reader.search(next.trim()), 250);
  }

  function clear() {
    if (timer) clearTimeout(timer);
    value = "";
    void reader.search("");
  }

  /** `/` is the reader convention for search; the page's own keys ignore it. */
  function onKeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    if (event.key !== "/" || event.metaKey || event.ctrlKey) return;
    if (target?.matches("input, textarea, select")) return;
    event.preventDefault();
    field?.focus();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="box">
  <SearchIcon size={14} />
  <input
    bind:this={field}
    {value}
    type="search"
    placeholder="search everything"
    aria-label="search"
    oninput={(e) => schedule(e.currentTarget.value)}
    onkeydown={(e) => {
      if (e.key === "Escape") {
        clear();
        e.currentTarget.blur();
      }
    }}
  />
  {#if value}
    <!-- Words vs meaning. Semantic needs the model host; when it is asleep the
         backend answers from the text index and says so, and the label follows
         what ran rather than what was asked for. -->
    <button
      class="mode"
      class:on={reader.mode === "semantic"}
      onclick={() =>
        reader.search(
          value.trim(),
          reader.mode === "semantic" ? "text" : "semantic",
        )}
      title={reader.mode === "semantic" && reader.ranAs === "text"
        ? "model host asleep — searched the words instead"
        : "search by meaning"}
    >
      {reader.ranAs === "semantic" ? "meaning" : "words"}
    </button>
    <button onclick={clear} aria-label="clear search"><X size={14} /></button>
  {/if}
</div>

<style>
  .box {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex: none;
    padding: 0.35rem 0.75rem;
    border-bottom: 1px solid var(--halo-border);
    color: var(--halo-text-muted);
  }

  input {
    flex: 1;
    min-width: 0;
    border: none;
    background: none;
    color: var(--halo-text-main);
    font: inherit;
    font-size: 0.85rem;
  }

  input:focus {
    outline: none;
  }

  /* Safari draws its own round clear button inside a type=search field, which
     would sit beside ours. */
  input::-webkit-search-cancel-button {
    display: none;
  }

  button {
    display: grid;
    place-items: center;
    padding: 0.15rem;
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
  }

  button:hover {
    color: var(--halo-text-main);
  }

  .mode {
    flex: none;
    padding: 0.05rem 0.4rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    font-family: var(--halo-font-heading);
    font-size: 0.68rem;
  }

  .mode.on {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }
</style>
