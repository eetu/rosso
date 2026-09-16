<script lang="ts">
  import ExternalLink from "@lucide/svelte/icons/external-link";
  import MessagesSquare from "@lucide/svelte/icons/messages-square";
  import Star from "@lucide/svelte/icons/star";
  import ThumbsDown from "@lucide/svelte/icons/thumbs-down";
  import ThumbsUp from "@lucide/svelte/icons/thumbs-up";
  import X from "@lucide/svelte/icons/x";

  import type { ItemDetail } from "$lib/api";
  import ScoreDot from "$lib/components/ScoreDot.svelte";
  import { reader } from "$lib/stores/reader.svelte";
  import { relativeTime } from "$lib/time";

  let { item }: { item: ItemDetail } = $props();
</script>

<article>
  <header>
    <span class="meta">
      {item.feed_title}
      {#if item.author}· {item.author}{/if}
      · {relativeTime(item.published_at)}
    </span>
    <span class="verbs">
      <button
        class:on={item.starred}
        onclick={() => reader.setStarred(item.id, !item.starred)}
        aria-label={item.starred ? "unstar" : "star"}
      >
        <Star size={15} fill={item.starred ? "currentColor" : "none"} />
      </button>
      {#if item.url}
        <!-- A publisher's own URL, not an app route: resolve() would be wrong
             here, and the link deliberately leaves the SPA. A block disable
             rather than -next-line, which prettier's attribute wrapping moves
             off the reported line. -->
        <!-- eslint-disable svelte/no-navigation-without-resolve -->
        <a
          href={item.url}
          target="_blank"
          rel="noreferrer"
          aria-label="open original"
        >
          <ExternalLink size={15} />
        </a>
        <!-- eslint-enable svelte/no-navigation-without-resolve -->
      {/if}
      {#if item.comments_url}
        <!-- eslint-disable svelte/no-navigation-without-resolve -->
        <a
          href={item.comments_url}
          target="_blank"
          rel="noreferrer"
          aria-label="open discussion"
        >
          <MessagesSquare size={15} />
        </a>
        <!-- eslint-enable svelte/no-navigation-without-resolve -->
      {/if}
      <button onclick={() => reader.closeItem()} aria-label="close"
        ><X size={15} /></button
      >
    </span>
  </header>

  <h1>{item.title || "(untitled)"}</h1>

  {#if item.tags.length > 0}
    <p class="tags">
      {#each item.tags as tag (tag)}
        <button onclick={() => reader.select({ tag })}>{tag}</button>
      {/each}
    </p>
  {/if}

  {#if item.summary}
    <div class="summary">
      <p>{item.summary}</p>
      <div class="verdict">
        {#if item.score !== null}
          <span class="score" title={item.score_reason ?? ""}>
            <ScoreDot score={item.score} reason={item.score_reason} />
            {item.score}
          </span>
        {/if}
        <!-- The thumbs are the only training signal rosso has: they become
             labelled examples in the scoring prompt for everything after. -->
        <button
          class:on={item.feedback > 0}
          onclick={() => reader.setFeedback(item.id, 1)}
          aria-label="more like this"
        >
          <ThumbsUp size={14} />
        </button>
        <button
          class:on={item.feedback < 0}
          onclick={() => reader.setFeedback(item.id, -1)}
          aria-label="less like this"
        >
          <ThumbsDown size={14} />
        </button>
      </div>
    </div>
  {/if}

  {#if item.siblings.length > 0}
    <!-- The rows the list collapsed into this one. Without them a dedupe that
         guessed wrong would be a silent deletion. -->
    <details class="siblings">
      <summary
        >also covered by {item.siblings.length} other{item.siblings.length === 1
          ? ""
          : "s"}</summary
      >
      <ul>
        <!-- eslint-disable svelte/no-navigation-without-resolve -->
        {#each item.siblings as sibling (sibling.id)}
          <li>
            <span class="feed">{sibling.feed_title}</span>
            {#if sibling.url}
              <a href={sibling.url} target="_blank" rel="noreferrer"
                >{sibling.title}</a
              >
            {:else}
              <span>{sibling.title}</span>
            {/if}
          </li>
        {/each}
        <!-- eslint-enable svelte/no-navigation-without-resolve -->
      </ul>
    </details>
  {/if}

  {#if item.content_html}
    <!-- Feed HTML is sanitized server-side by ammonia in feed/parse.rs, once,
         before it is ever stored — and the CSP carries no script-src
         'unsafe-inline', so even a sanitizer miss cannot execute. Rendering the
         publisher's markup is the whole job of a reader. -->
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    <div class="body">{@html item.content_html}</div>
  {:else}
    <!-- Aggregator feeds ship no body at all — the item *is* the pair of links.
         Say so and put them where the article would have been, rather than
         leaving the pane looking broken. -->
    <p class="empty">no content in the feed.</p>
    <!-- eslint-disable svelte/no-navigation-without-resolve -->
    <p class="elsewhere">
      {#if item.url}
        <a href={item.url} target="_blank" rel="noreferrer">open article</a>
      {/if}
      {#if item.comments_url}
        <a href={item.comments_url} target="_blank" rel="noreferrer"
          >open discussion</a
        >
      {/if}
    </p>
    <!-- eslint-enable svelte/no-navigation-without-resolve -->
  {/if}
</article>

<style>
  article {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 1rem 1.25rem 3rem;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }

  .meta {
    flex: 1;
    min-width: 0;
    font-family: var(--halo-font-heading);
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .verbs {
    display: flex;
    gap: 0.125rem;
  }

  .verbs button,
  .verbs a {
    display: grid;
    place-items: center;
    padding: 0.25rem;
    border: none;
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  .verbs button:hover,
  .verbs a:hover {
    color: var(--halo-text-main);
  }

  .verbs .on {
    color: var(--halo-accent);
  }

  h1 {
    margin: 0 0 0.5rem;
    font-size: 1.35rem;
    font-weight: 600;
    line-height: 1.25;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin: 0 0 0.75rem;
  }

  .tags button {
    padding: 0.1rem 0.4rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    font: inherit;
    font-family: var(--halo-font-heading);
    font-size: 0.7rem;
    cursor: pointer;
  }

  .tags button:hover {
    color: var(--halo-accent);
    border-color: var(--halo-accent);
  }

  .summary {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    margin: 0 0 1rem;
    padding: 0.6rem 0.75rem;
    border-left: 2px solid var(--halo-accent);
    background: var(--halo-accent-soft);
    font-size: 0.85rem;
    line-height: 1.45;
  }

  .summary p {
    flex: 1;
    min-width: 0;
    margin: 0;
  }

  .verdict {
    display: flex;
    align-items: center;
    gap: 0.2rem;
    flex: none;
  }

  /* Folded away by default: the point of collapsing the list was not to read
     the same story five times. */
  .siblings {
    margin: 0 0 1rem;
    font-size: 0.8rem;
    color: var(--halo-text-muted);
  }

  .siblings summary {
    cursor: pointer;
  }

  .siblings ul {
    margin: 0.4rem 0 0;
    padding: 0;
    list-style: none;
  }

  .siblings li {
    display: flex;
    gap: 0.5rem;
    padding: 0.15rem 0;
  }

  .siblings .feed {
    flex: none;
    min-width: 7rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--halo-font-heading);
    font-size: 0.7rem;
  }

  .score {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin-right: 0.3rem;
    font-family: var(--halo-font-heading);
    font-size: 0.75rem;
    color: var(--halo-text-muted);
  }

  .verdict button {
    display: grid;
    place-items: center;
    padding: 0.2rem;
    border: none;
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  .verdict button:hover {
    color: var(--halo-text-main);
  }

  .verdict button.on {
    color: var(--halo-accent);
  }

  .body {
    max-width: 42rem;
    font-size: 0.95rem;
    line-height: 1.6;
  }

  /* Publisher markup is outside this component's scope, so it needs :global.
     Sanitizing strips the publisher's own classes and stylesheet, so every rule
     the article relied on has to be supplied here — untouched, browser defaults
     make extracted pages read as a pile of loosely stacked blocks. */

  .body :global(img),
  .body :global(video),
  .body :global(iframe) {
    max-width: 100%;
    height: auto;
    border-radius: var(--halo-radius);
  }

  .body :global(p) {
    margin: 0 0 0.9em;
  }

  .body :global(h1),
  .body :global(h2),
  .body :global(h3),
  .body :global(h4) {
    margin: 1.4em 0 0.4em;
    font-size: 1.05em;
    line-height: 1.3;
  }

  .body :global(ul),
  .body :global(ol) {
    margin: 0 0 0.9em;
    padding-left: 1.25rem;
  }

  .body :global(li) {
    margin-bottom: 0.3em;
  }

  /* Link-wrapped list rows — a liveblog's "key events", a related-articles
     strip. The publisher laid these out with their own CSS; on bare defaults the
     inner paragraphs bring full margins and the block children split the inline
     anchor, stranding the list marker on a line of its own above the entry.
     Scoped with :has so it only catches anchors that actually wrap blocks — a
     plain inline link inside list prose must stay on its line. */
  .body :global(li > a:has(p, div, h1, h2, h3, h4)) {
    display: block;
  }

  .body :global(li p) {
    margin: 0;
  }

  .body :global(time) {
    color: var(--halo-text-muted);
    font-size: 0.85em;
  }

  /* Accent as a signal, not a wash: a nav list of twenty links rendered in the
     brand colour drowns out the one thing on the page that is actually live. */
  .body :global(a) {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--halo-accent);
    text-underline-offset: 2px;
  }

  .body :global(a:hover) {
    color: var(--halo-accent);
  }

  .body :global(pre) {
    overflow-x: auto;
    padding: 0.6rem;
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
  }

  .body :global(blockquote) {
    margin: 0 0 0.9em;
    padding-left: 0.75rem;
    border-left: 2px solid var(--halo-border);
    color: var(--halo-text-muted);
  }

  .body :global(figure) {
    margin: 1em 0;
  }

  .body :global(figcaption) {
    margin-top: 0.4em;
    font-size: 0.8em;
    color: var(--halo-text-muted);
  }

  /* A wide table scrolls itself rather than widening the whole pane. */
  .body :global(table) {
    display: block;
    overflow-x: auto;
    border-collapse: collapse;
  }

  .body :global(th),
  .body :global(td) {
    padding: 0.3em 0.6em;
    border: 1px solid var(--halo-border);
    text-align: left;
  }

  .body :global(hr) {
    border: none;
    border-top: 1px solid var(--halo-border);
    margin: 1.4em 0;
  }

  .empty {
    color: var(--halo-text-muted);
    font-size: 0.85rem;
  }

  .elsewhere {
    display: flex;
    gap: 1rem;
    font-size: 0.85rem;
  }

  .elsewhere a {
    color: var(--halo-accent);
  }
</style>
