<script lang="ts">
  import type { Digest, Item } from "$lib/api";
  import ScoreDot from "$lib/components/ScoreDot.svelte";
  import { reader } from "$lib/stores/reader.svelte";

  let { digest }: { digest: Digest } = $props();

  const byId = $derived(new Map(digest.items.map((i) => [i.id, i])));

  /** Ids the backend resolved. A thread never renders a link to nothing. */
  function itemsOf(ids: number[]): Item[] {
    return ids.flatMap((id) => {
      const item = byId.get(id);
      return item ? [item] : [];
    });
  }
</script>

<article>
  <header>
    <span class="meta">{digest.day} · from {digest.item_count} items</span>
  </header>

  <p class="intro">{digest.intro}</p>

  {#each digest.threads as thread, i (i)}
    <section>
      <h2>{thread.title}</h2>
      <p>{thread.note}</p>
      <ul>
        {#each itemsOf(thread.item_ids) as item (item.id)}
          <li>
            <button onclick={() => reader.openItem(item.id)}>
              {#if item.score !== null}
                <ScoreDot score={item.score} reason={item.score_reason} />
              {/if}
              <span class="title">{item.title || "(untitled)"}</span>
              <span class="feed">{item.feed_title}</span>
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="empty">nothing in this one.</p>
  {/each}
</article>

<style>
  article {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 1rem 1.25rem 3rem;
  }

  header {
    margin-bottom: 0.75rem;
  }

  .meta {
    font-family: var(--halo-font-heading);
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .intro {
    margin: 0 0 1.25rem;
    padding: 0.6rem 0.75rem;
    border-left: 2px solid var(--halo-accent);
    background: var(--halo-accent-soft);
    font-size: 0.9rem;
    line-height: 1.5;
  }

  section {
    margin-bottom: 1.5rem;
  }

  h2 {
    margin: 0 0 0.35rem;
    font-size: 1rem;
    font-weight: 600;
    line-height: 1.3;
  }

  section p {
    margin: 0 0 0.5rem;
    font-size: 0.88rem;
    line-height: 1.5;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li button {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.25rem 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    font-size: 0.82rem;
    text-align: left;
    cursor: pointer;
  }

  li button:hover .title {
    color: var(--halo-accent);
  }

  /* Shrinks and ellipsises so a long headline never pushes the feed name off. */
  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .feed {
    flex: none;
    font-family: var(--halo-font-heading);
    font-size: 0.68rem;
    color: var(--halo-text-muted);
  }

  .empty {
    color: var(--halo-text-muted);
    font-size: 0.85rem;
  }
</style>
