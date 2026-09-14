<script lang="ts">
  import Plus from "@lucide/svelte/icons/plus";

  import { reader } from "$lib/stores/reader.svelte";

  let url = $state("");
  let pending = $state(false);
  let error = $state<string | null>(null);
  let added = $state<string | null>(null);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const value = url.trim();
    if (!value || pending) return;

    pending = true;
    error = null;
    added = null;
    try {
      const feed = await reader.addFeed(value);
      url = "";
      added = feed.title;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      pending = false;
    }
  }
</script>

<form onsubmit={submit}>
  <div class="row">
    <input
      bind:value={url}
      type="text"
      inputmode="url"
      autocomplete="url"
      spellcheck="false"
      placeholder="example.com or a feed url"
      aria-label="feed url"
      disabled={pending}
    />
    <button
      type="submit"
      disabled={pending || url.trim() === ""}
      aria-label="add feed"
    >
      <Plus size={16} />
    </button>
  </div>

  {#if pending}
    <p class="note">looking for a feed…</p>
  {:else if error}
    <p class="note error">{error}</p>
  {:else if added}
    <p class="note">added {added}</p>
  {/if}
</form>

<style>
  .row {
    display: flex;
    gap: 0.375rem;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 0.4rem 0.55rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font: inherit;
    font-size: 0.85rem;
  }

  input:focus-visible {
    outline: 2px solid var(--halo-accent);
    outline-offset: -1px;
  }

  button {
    display: grid;
    place-items: center;
    padding: 0 0.55rem;
    border: none;
    border-radius: var(--halo-radius);
    background: var(--halo-accent);
    color: var(--halo-on-accent);
    cursor: pointer;
  }

  button:disabled {
    background: var(--halo-off-bg);
    color: var(--halo-text-muted);
    cursor: default;
  }

  .note {
    margin: 0.4rem 0 0;
    font-size: 0.75rem;
    color: var(--halo-text-muted);
  }

  .error {
    color: var(--halo-error);
  }
</style>
