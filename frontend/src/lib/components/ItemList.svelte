<script lang="ts">
  import Star from "@lucide/svelte/icons/star";

  import ScoreDot from "$lib/components/ScoreDot.svelte";
  import { reader } from "$lib/stores/reader.svelte";
  import { relativeTime } from "$lib/time";

  let { selectedId }: { selectedId: number | null } = $props();
</script>

<ul>
  {#each reader.items as item (item.id)}
    <li>
      <!-- Two lines per row. The summary belongs to the reader pane; on a
           hundred-row list it turns scanning into reading, which is the pile
           this app exists to shrink. It stays reachable on hover. -->
      <button
        class="row"
        class:selected={item.id === selectedId}
        class:read={item.read}
        title={item.summary ?? undefined}
        onclick={() => reader.openItem(item.id)}
      >
        <span class="head">
          {#if item.score !== null}
            <ScoreDot score={item.score} reason={item.score_reason} />
          {/if}
          <span class="feed">{item.feed_title}</span>
          <span class="age">{relativeTime(item.published_at)}</span>
        </span>
        <span class="title">{item.title || "(untitled)"}</span>
      </button>
      <button
        class="star"
        class:on={item.starred}
        onclick={() => reader.setStarred(item.id, !item.starred)}
        aria-label={item.starred ? "unstar" : "star"}
      >
        <Star size={14} fill={item.starred ? "currentColor" : "none"} />
      </button>
    </li>
  {:else}
    <li class="empty">
      {#if reader.loadingItems}
        loading…
      {:else if reader.view === "unread"}
        nothing unread.
      {:else}
        nothing here.
      {/if}
    </li>
  {/each}
</ul>

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
    display: flex;
    align-items: flex-start;
    border-bottom: 1px solid var(--halo-border);
  }

  button {
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .row {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.4rem 0.75rem;
    text-align: left;
  }

  /* The button stays full width so the whole row is clickable, but its contents
     stop at a reading measure — the same one the reader pane uses. Left to
     stretch, `.age`'s margin-left:auto flings the timestamp to the far edge of a
     wide window and leaves a dead band between it and the title. */
  .row > * {
    width: 100%;
    max-width: 42rem;
  }

  .row:hover {
    background: var(--halo-bg-light);
  }

  .selected {
    background: var(--halo-accent-soft);
  }

  .read .title {
    color: var(--halo-text-muted);
    font-weight: 400;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-family: var(--halo-font-heading);
    font-size: 0.68rem;
    color: var(--halo-text-muted);
  }

  .feed {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .age {
    flex: none;
    margin-left: auto;
  }

  .title {
    font-size: 0.9rem;
    font-weight: 500;
    line-height: 1.3;
  }

  .star {
    padding: 0.55rem 0.5rem;
    color: var(--halo-text-light);
  }

  .star.on {
    color: var(--halo-accent);
  }

  .empty {
    padding: 1rem 0.75rem;
    border-bottom: none;
    font-size: 0.85rem;
    color: var(--halo-text-muted);
  }
</style>
