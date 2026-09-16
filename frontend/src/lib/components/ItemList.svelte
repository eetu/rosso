<script lang="ts">
  import Star from "@lucide/svelte/icons/star";

  import ScoreDot from "$lib/components/ScoreDot.svelte";
  import { reader } from "$lib/stores/reader.svelte";
  import { relativeTime } from "$lib/time";

  let { selectedId }: { selectedId: number | null } = $props();
</script>

<!-- New items arrive at the top, so the store needs to know whether that is
     where you are looking before it reloads the list under you. -->
<ul onscroll={(e) => reader.setListAtTop(e.currentTarget.scrollTop < 4)}>
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
      {:else if reader.q}
        nothing matches “{reader.q}”.
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

  /* The row's tint is painted here rather than on the button inside it, so it
     covers the star's column too. Painted on `.row` alone it stopped at the
     button's edge and left a pale gutter with the star floating in it. */
  li {
    display: flex;
    align-items: flex-start;
    border-bottom: 1px solid var(--halo-border);
  }

  li:hover {
    background: var(--halo-bg-light);
  }

  li:has(.selected) {
    background: var(--halo-accent-soft);
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

  /* Shrinks and ellipsises so a long publication name never pushes the age out
     of sight. */
  .feed {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Sits next to the feed name rather than being pushed to the far edge by a
     margin-left:auto. Right-aligning it means the gap grows with the window,
     which on a wide list is a band of nothing across every row — and capping the
     row's width to hide that only moved the gap somewhere else. */
  .age {
    flex: none;
  }

  .age::before {
    content: "·";
    margin-right: 0.4rem;
  }

  .title {
    font-size: 0.9rem;
    font-weight: 500;
    line-height: 1.3;
  }

  /* Quiet until it has something to say. An empty star on every row competes
     with the titles for attention, when the thing worth noticing is the handful
     that are actually starred. */
  .star {
    padding: 0.5rem;
    color: var(--halo-text-light);
    opacity: 0;
  }

  .star.on,
  li:hover .star,
  .star:focus-visible {
    opacity: 1;
  }

  .star.on {
    color: var(--halo-accent);
  }

  /* Touch has no hover to reveal it, so the empty outline stays visible — just
     faint. `:not(.on)` because a starred one is never dimmed: it is the whole
     signal the column exists to carry. */
  @media (hover: none) {
    .star:not(.on) {
      opacity: 0.35;
    }
  }

  .empty {
    padding: 1rem 0.75rem;
    border-bottom: none;
    font-size: 0.85rem;
    color: var(--halo-text-muted);
  }
</style>
