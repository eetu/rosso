<script lang="ts">
  import X from "@lucide/svelte/icons/x";

  import { api, type Inspection } from "$lib/api";
  import { reader } from "$lib/stores/reader.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let data = $state<Inspection | null>(null);
  let error = $state("");

  async function load() {
    try {
      data = await api.inspectFeeds();
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  $effect(() => {
    void load();
  });

  /** Seconds as something readable. Exactness past the hour is not the point. */
  function every(seconds: number): string {
    if (seconds < 3600) return `${Math.round(seconds / 60)} min`;
    const hours = seconds / 3600;
    if (hours < 48) return `${hours % 1 === 0 ? hours : hours.toFixed(1)} h`;
    return `${Math.round(hours / 24)} d`;
  }

  function when(iso: string | null): string {
    if (!iso) return "never";
    const delta = (Date.parse(iso) - Date.now()) / 1000;
    const ahead = delta > 0;
    const mag = every(Math.abs(delta));
    return ahead ? `in ${mag}` : `${mag} ago`;
  }

  /**
   * The line that answers the actual question: is rosso doing what this
   * publisher asked? Each clause only appears when there is something to say, so
   * a feed that stated no preferences shows the one fact that is always true.
   */
  function honoured(feed: Inspection["feeds"][number]): string[] {
    const out: string[] = [];
    if (feed.ttl_minutes !== null) {
      const asked = feed.ttl_minutes * 60;
      out.push(
        feed.interval_s >= asked
          ? `asks for ${every(asked)}, polled no sooner`
          : `asks for ${every(asked)} — not honoured`,
      );
    }
    if (feed.retry_after_s !== null) {
      out.push(`asked us to wait ${every(feed.retry_after_s)}, honoured`);
    }
    out.push(
      feed.conditional
        ? "conditional GET: a quiet poll costs a 304"
        : "no validator offered, so each poll is a full body",
    );
    return out;
  }

  async function reenable(id: number) {
    await api.updateFeed(id, { disabled: false });
    await Promise.all([load(), reader.reloadFeeds()]);
  }
</script>

<div
  class="backdrop"
  role="button"
  tabindex="-1"
  onclick={onclose}
  onkeydown={(e) => e.key === "Escape" && onclose()}
></div>

<section class="panel halo-card" aria-label="feed inspector">
  <header>
    <h2>feeds</h2>
    <button onclick={onclose} aria-label="close"><X size={16} /></button>
  </header>

  {#if error}
    <p class="error">{error}</p>
  {:else if !data}
    <p class="hint">loading…</p>
  {:else}
    <p class="hint">
      polled between {every(data.min_interval_s)} and {every(
        data.max_interval_s,
      )}, sooner when a feed is busy — a publisher asking for longer gets it.
    </p>
    <ul>
      {#each data.feeds as feed (feed.id)}
        <li class:disabled={feed.disabled}>
          <div class="row">
            <span class="name" title={feed.url}>{feed.title}</span>
            <span class="every">every {every(feed.interval_s)}</span>
          </div>
          <div class="row sub">
            <span>last {when(feed.last_fetch_at)}</span>
            <span
              >{feed.disabled
                ? "not scheduled"
                : `next ${when(feed.next_fetch_at)}`}</span
            >
            {#if !feed.llm_enabled}<span>no model</span>{/if}
          </div>
          <ul class="notes">
            {#each honoured(feed) as note (note)}
              <li>{note}</li>
            {/each}
          </ul>
          {#if feed.last_error}
            <p class="err" title={feed.last_error}>{feed.last_error}</p>
          {/if}
          {#if feed.disabled}
            <!-- Retired rather than deleted, so the reason stays readable and
                 the decision stays yours. -->
            <p class="retired">
              retired after {feed.refusals} refusals.
              <button onclick={() => reenable(feed.id)}>try again</button>
            </p>
          {:else if feed.refusals > 0}
            <p class="err">
              {feed.refusals} of {data.max_refusals} refusals — retires at {data.max_refusals}.
            </p>
          {/if}
        </li>
      {:else}
        <li class="hint">no feeds yet.</li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    border: none;
    background: rgb(0 0 0 / 35%);
  }

  .panel {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(40rem, calc(100vw - 2rem));
    max-height: calc(100dvh - 2rem);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.25rem;
  }

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
    border: none;
    background: none;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  ul > li {
    padding: 0.6rem 0;
    border-bottom: 1px solid var(--halo-border);
  }

  ul > li:last-child {
    border-bottom: none;
  }

  li.disabled .name {
    text-decoration: line-through;
  }

  .row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.9rem;
    font-weight: 500;
  }

  .every {
    flex: none;
    font-family: var(--halo-font-heading);
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .sub {
    justify-content: flex-start;
    gap: 0.75rem;
    margin-top: 0.1rem;
    font-family: var(--halo-font-heading);
    font-size: 0.68rem;
    color: var(--halo-text-muted);
  }

  .notes {
    margin: 0.3rem 0 0;
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .notes li {
    padding: 0;
    border: none;
  }

  .notes li::before {
    content: "· ";
  }

  .hint {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .err,
  .error {
    margin: 0.3rem 0 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.72rem;
    color: var(--halo-error);
  }

  .retired {
    margin: 0.3rem 0 0;
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .retired button {
    padding: 0.1rem 0.4rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-main);
    font: inherit;
    font-size: 0.7rem;
    cursor: pointer;
  }
</style>
