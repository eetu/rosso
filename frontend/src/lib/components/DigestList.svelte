<script lang="ts">
  import { reader } from "$lib/stores/reader.svelte";

  let making = $state(false);
  let failure = $state("");

  async function make() {
    making = true;
    failure = "";
    try {
      await reader.makeDigest();
    } catch (e) {
      // The two real cases are a day that held too little and a model host that
      // is not answering. The backend says which; repeating it beats "failed".
      failure = e instanceof Error ? e.message : String(e);
    } finally {
      making = false;
    }
  }
</script>

<ul>
  {#each reader.digestDays as day (day)}
    <li>
      <button
        class="row"
        class:selected={reader.digest?.day === day}
        onclick={() => reader.openDigest(day)}
      >
        {day}
      </button>
    </li>
  {:else}
    <li class="empty">
      no digests yet. one is written each morning for the day before.
    </li>
  {/each}
</ul>

<!-- Waiting a day to see whether the prompt is any good is how a prompt goes
     untuned. -->
<div class="foot">
  {#if failure}
    <p class="error">{failure}</p>
  {/if}
  <button class="make" onclick={make} disabled={making}>
    {making ? "writing…" : "write yesterday's now"}
  </button>
</div>

<style>
  ul {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    border-bottom: 1px solid var(--halo-border);
  }

  li:hover {
    background: var(--halo-bg-light);
  }

  li:has(.selected) {
    background: var(--halo-accent-soft);
  }

  .row {
    width: 100%;
    padding: 0.5rem 0.75rem;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    font-family: var(--halo-font-heading);
    font-size: 0.85rem;
    text-align: left;
    cursor: pointer;
  }

  .empty {
    padding: 1rem 0.75rem;
    border-bottom: none;
    font-size: 0.85rem;
    color: var(--halo-text-muted);
  }

  .foot {
    flex: none;
    padding: 0.6rem 0.75rem;
    padding-bottom: calc(0.6rem + env(safe-area-inset-bottom));
    border-top: 1px solid var(--halo-border);
  }

  .error {
    margin: 0 0 0.4rem;
    font-size: 0.72rem;
    color: var(--halo-error);
  }

  .make {
    width: 100%;
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
  }

  .make:hover:not(:disabled) {
    color: var(--halo-text-main);
  }

  .make:disabled {
    cursor: default;
  }
</style>
