<script lang="ts">
  import RotateCw from "@lucide/svelte/icons/rotate-cw";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import TriangleAlert from "@lucide/svelte/icons/triangle-alert";

  import type { ItemView } from "$lib/api";
  import AddFeed from "$lib/components/AddFeed.svelte";
  import { reader } from "$lib/stores/reader.svelte";

  const VIEWS: { id: ItemView; label: string }[] = [
    { id: "unread", label: "unread" },
    { id: "interesting", label: "interesting" },
    { id: "starred", label: "starred" },
    { id: "all", label: "all" },
  ];

  // Nothing has been scored, so the view would only ever be empty. Hiding it is
  // kinder than offering a door onto a blank room.
  const views = $derived(
    VIEWS.filter(
      (v) => v.id !== "interesting" || reader.settings?.interest_profile.trim(),
    ),
  );

  let busyFeed = $state<number | null>(null);

  async function refresh(id: number) {
    busyFeed = id;
    try {
      await reader.refreshFeed(id);
    } finally {
      busyFeed = null;
    }
  }

  async function remove(id: number, title: string) {
    if (!confirm(`unsubscribe from ${title}? its items go too.`)) return;
    busyFeed = id;
    try {
      await reader.removeFeed(id);
    } finally {
      busyFeed = null;
    }
  }
</script>

<aside>
  <nav>
    {#each views as v (v.id)}
      <button
        class="view"
        class:active={reader.view === v.id && reader.feedId === null}
        onclick={() => reader.select({ view: v.id, feedId: null })}
      >
        <span>{v.label}</span>
        {#if v.id === "unread" && reader.totalUnread > 0}
          <span class="count">{reader.totalUnread}</span>
        {/if}
      </button>
    {/each}
  </nav>

  <h2>feeds</h2>

  <ul>
    {#each reader.feeds as feed (feed.id)}
      <li class:busy={busyFeed === feed.id}>
        <button
          class="feed"
          class:active={reader.feedId === feed.id}
          onclick={() => reader.select({ feedId: feed.id })}
          title={feed.url}
        >
          {#if feed.last_error}
            <span class="warn" title={feed.last_error}
              ><TriangleAlert size={13} /></span
            >
          {/if}
          <span class="name">{feed.title}</span>
          {#if feed.unread > 0}
            <span class="count">{feed.unread}</span>
          {/if}
        </button>
        <span class="actions">
          <button
            onclick={() => refresh(feed.id)}
            aria-label="refresh {feed.title}"
          >
            <RotateCw size={13} />
          </button>
          <button
            onclick={() => remove(feed.id, feed.title)}
            aria-label="remove {feed.title}"
          >
            <Trash2 size={13} />
          </button>
        </span>
      </li>
    {:else}
      <li class="empty">no feeds yet.</li>
    {/each}
  </ul>

  <div class="add"><AddFeed /></div>
</aside>

<style>
  aside {
    display: flex;
    flex-direction: column;
    min-height: 0;
    width: 15rem;
    flex: none;
    padding: 0.75rem;
    gap: 0.5rem;
    background: var(--halo-bg-light);
    border-right: 1px solid var(--halo-border);
  }

  nav {
    display: flex;
    flex-direction: column;
  }

  h2 {
    margin: 0.5rem 0 0;
    font-family: var(--halo-font-heading);
    font-size: 0.7rem;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--halo-text-muted);
  }

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
    align-items: center;
  }

  li.busy {
    opacity: 0.5;
  }

  li .actions {
    display: none;
    gap: 0.125rem;
  }

  /* Row verbs stay out of the way until the row is under the pointer or holds
     focus — keyboard users get them at the same moment. */
  li:hover .actions,
  li:focus-within .actions {
    display: flex;
  }

  button {
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    border-radius: var(--halo-radius);
  }

  .view,
  .feed {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.4rem;
    text-align: left;
    font-size: 0.85rem;
  }

  .view {
    font-family: var(--halo-font-heading);
  }

  .view:hover,
  .feed:hover {
    background: var(--halo-bg-main);
  }

  .active {
    color: var(--halo-accent);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count {
    font-family: var(--halo-font-heading);
    font-size: 0.7rem;
    color: var(--halo-text-muted);
  }

  .warn {
    display: inline-flex;
    color: var(--halo-error);
  }

  .actions button {
    display: grid;
    place-items: center;
    padding: 0.2rem;
    color: var(--halo-text-muted);
  }

  .actions button:hover {
    color: var(--halo-text-main);
  }

  .empty {
    padding: 0.3rem 0.4rem;
    font-size: 0.8rem;
    color: var(--halo-text-muted);
  }

  .add {
    padding-top: 0.5rem;
    border-top: 1px solid var(--halo-border);
  }
</style>
