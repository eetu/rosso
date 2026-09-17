<script lang="ts">
  import Info from "@lucide/svelte/icons/info";
  import RotateCw from "@lucide/svelte/icons/rotate-cw";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import TriangleAlert from "@lucide/svelte/icons/triangle-alert";

  import type { ItemView } from "$lib/api";
  import AddFeed from "$lib/components/AddFeed.svelte";
  import FeedInspector from "$lib/components/FeedInspector.svelte";
  import { reader } from "$lib/stores/reader.svelte";

  // On a phone the sidebar *is* the screen, so choosing something has to hand
  // the screen back to the list. On desktop it stays put and this does nothing.
  let { onselect = () => {} }: { onselect?: () => void } = $props();

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
  let inspecting = $state(false);

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
        class:active={reader.view === v.id &&
          reader.feedId === null &&
          reader.tag === null &&
          !reader.digestMode}
        onclick={() => {
          reader.select({ view: v.id, feedId: null, tag: null });
          onselect();
        }}
      >
        <span>{v.label}</span>
        {#if v.id === "unread" && reader.totalUnread > 0}
          <span class="count">{reader.totalUnread}</span>
        {/if}
      </button>
    {/each}
    <!-- A document rather than a filtered list, which is why it sits apart from
         the views instead of alongside them. Only offered once there is a model
         to write one — without it the section is a permanently empty room. -->
    {#if reader.settings?.llm_available || reader.digestDays.length > 0}
      <button
        class="view"
        class:active={reader.digestMode}
        onclick={() => {
          reader.showDigests();
          onselect();
        }}
      >
        <span>digest</span>
      </button>
    {/if}
  </nav>

  <!-- Topics are the tags the model already assigns, kept only where enough
       items share one. Absent until the model has been round, which is the
       honest state rather than an empty heading. -->
  {#if reader.topics.length > 0}
    <h2>topics</h2>
    <ul class="topics">
      {#each reader.topics as topic (topic.tag)}
        <li>
          <button
            class="feed"
            class:active={reader.tag === topic.tag}
            onclick={() => {
              reader.select({
                tag: reader.tag === topic.tag ? null : topic.tag,
              });
              onselect();
            }}
          >
            <span class="name">{topic.tag}</span>
            <span class="count">{topic.unread}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <h2 class="heading">
    feeds
    <!-- How often each is polled, and whether what the publisher asked for is
         being honoured. -->
    <button onclick={() => (inspecting = true)} aria-label="inspect feeds">
      <Info size={13} />
    </button>
  </h2>

  <ul>
    {#each reader.feeds as feed (feed.id)}
      <li class:busy={busyFeed === feed.id}>
        <button
          class="feed"
          class:active={reader.feedId === feed.id}
          onclick={() => {
            reader.select({ feedId: feed.id });
            onselect();
          }}
          title={feed.url}
        >
          {#if feed.last_error}
            <span class="warn" title={feed.last_error}
              ><TriangleAlert size={13} /></span
            >
          {:else if feed.icon}
            <!-- A `data:` URI the backend fetched once, not a link to the
                 publisher: an <img> pointing at their host would announce the
                 reader to every site in this list on every page load. -->
            <img class="icon" src={feed.icon} alt="" width="14" height="14" />
          {/if}
          <span class="name">{feed.title}</span>
          {#if feed.unread > 0}
            <span class="count">{feed.unread}</span>
          {/if}
        </button>
        <span class="actions">
          <!-- Some feeds say everything in the title — release notes, a commit
               log — and a summary of one costs a generation to restate the
               headline. Off leaves what was already written alone. -->
          {#if reader.settings?.llm_available || !feed.llm_enabled}
            <button
              class:off={!feed.llm_enabled}
              onclick={() => reader.setFeedLlm(feed.id, !feed.llm_enabled)}
              aria-label={feed.llm_enabled
                ? "stop summarizing {feed.title}"
                : "summarize {feed.title}"}
              title={feed.llm_enabled
                ? "summarized and scored"
                : "not read by the model"}
            >
              <Sparkles size={13} />
            </button>
          {/if}
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

{#if inspecting}
  <FeedInspector onclose={() => (inspecting = false)} />
{/if}

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

  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .heading button {
    display: grid;
    place-items: center;
    padding: 0;
    border: none;
    background: none;
    color: var(--halo-text-light);
    cursor: pointer;
  }

  .heading button:hover {
    color: var(--halo-text-main);
  }

  ul {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Topics are a short, capped list — the feed list is the one that grows and
     earns the remaining height. */
  .topics {
    flex: none;
    overflow: visible;
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

  /* Struck through rather than merely dimmed: every icon in this row is quiet,
     so dimming one more would not read as a state. */
  .actions button.off {
    color: var(--halo-text-light);
    text-decoration: line-through;
  }

  /* Sits where the warning triangle would, so a feed does not shift sideways
     when it starts failing. */
  .icon {
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: 2px;
    object-fit: contain;
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

  /* Last in the file on purpose: these override rules above at equal
     specificity, and a media query earlier in the sheet would simply lose.
     On a phone this is not a sidebar, it is a screen — 15rem of it beside the
     list left about 150px for the content. */
  @media (max-width: 800px) {
    aside {
      width: 100%;
      border-right: none;
      /* The pane scrolls as one. On desktop only the feed list scrolls, which
         works because everything above it is short — but here the views, the
         topics and the add form together can already fill the screen, leaving
         the feed list (the one scrolling region) squeezed to nothing and the
         feeds unreachable. */
      overflow-y: auto;
      padding-bottom: calc(0.75rem + env(safe-area-inset-bottom));
    }

    /* So the inner lists stop being scroll containers of their own. */
    ul {
      flex: none;
      overflow: visible;
    }

    /* Touch has no hover to reveal them, so the row verbs stay out. */
    li .actions {
      display: flex;
    }

    /* Finger-sized rows. */
    .view,
    .feed {
      padding: 0.5rem 0.4rem;
      font-size: 0.95rem;
    }
  }
</style>
