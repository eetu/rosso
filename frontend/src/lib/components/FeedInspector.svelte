<script lang="ts">
  import { api, type Inspection } from "$lib/api";
  import Modal from "$lib/components/Modal.svelte";
  import { reader } from "$lib/stores/reader.svelte";

  type Props = {
    onclose: () => void;
    /** Opened from one feed's menu: bring that row into view and mark it. */
    focus?: number | null;
  };

  let { onclose, focus = null }: Props = $props();

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

  /**
   * Bring the feed this was opened from into view.
   *
   * Without it, choosing "Feed details" on the twentieth feed opens a list
   * scrolled to the first — the answer is on screen, just not where you are
   * looking, which reads as the menu item having done nothing.
   *
   * `instant` rather than smooth: the panel has only just appeared, so there is
   * no position the reader was tracking for an animation to explain.
   */
  function reveal(node: HTMLElement) {
    if (node.dataset.feed !== String(focus)) return;
    node.scrollIntoView({ block: "center", behavior: "instant" });
  }

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

  /**
   * What to send as User-Agent, offered as a list rather than a text box.
   *
   * The blocks these exist for are name allowlists, not bot detection — a
   * publisher whose WAF refuses `rosso/0.1.0` will wave `curl` through while
   * refusing `Feedly` too. So the useful answers are a short known set, and
   * inviting a typed string mostly invites typos.
   */
  const AGENTS = [
    { label: "rosso (default)", value: "" },
    { label: "curl", value: "curl/8.7.1" },
    { label: "wget", value: "Wget/1.21.4" },
    { label: "python-requests", value: "python-requests/2.31.0" },
    {
      label: "a browser",
      value:
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 " +
        "(KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
    },
  ];

  const CUSTOM = "custom";

  /** Which feed has its agent picker open. */
  let editing = $state<number | null>(null);
  let choice = $state("");
  let draft = $state("");

  function edit(feed: Inspection["feeds"][number]) {
    editing = feed.id;
    const current = feed.user_agent ?? "";
    const known = AGENTS.some((a) => a.value === current);
    choice = known ? current : CUSTOM;
    draft = current;
  }

  /** The label for whatever a feed is currently set to. */
  function agentLabel(agent: string | null): string {
    return AGENTS.find((a) => a.value === (agent ?? ""))?.label ?? agent ?? "";
  }

  /**
   * Setting this also clears the feed's extraction attempts, server-side —
   * every item it would help has already been given up on, so without that the
   * setting would look like it did nothing.
   */
  async function saveAgent(id: number) {
    await api.updateFeed(id, {
      user_agent: choice === CUSTOM ? draft : choice,
    });
    editing = null;
    await load();
  }
</script>

<Modal title="feeds" {onclose} width="40rem">
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
        <li
          class:disabled={feed.disabled}
          class:revealed={feed.id === focus}
          data-feed={feed.id}
          use:reveal
        >
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

          <!-- Some publishers allowlist client names, so an unfamiliar one is
               refused while curl and wget are waved through. Naming a different
               agent for one feed is the lever for that. -->
          {#if editing === feed.id}
            <p class="agent">
              <select bind:value={choice} aria-label="fetch {feed.title} as">
                {#each AGENTS as agent (agent.value)}
                  <option value={agent.value}>{agent.label}</option>
                {/each}
                <option value={CUSTOM}>something else…</option>
              </select>
              {#if choice === CUSTOM}
                <input
                  bind:value={draft}
                  placeholder="a User-Agent string"
                  spellcheck="false"
                  aria-label="user-agent for {feed.title}"
                />
              {/if}
              <button onclick={() => saveAgent(feed.id)}>save</button>
              <button onclick={() => (editing = null)}>cancel</button>
            </p>
          {:else}
            <p class="agent">
              <span>fetched as {agentLabel(feed.user_agent)}</span>
              <button onclick={() => edit(feed)}>change</button>
            </p>
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
</Modal>

<style>
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

  /* The feed this was opened from. Scrolling it into view puts it somewhere in
     the middle of a list of near-identical rows, so it also has to say which
     one it is. Same recipe as a selected row elsewhere. */
  li.revealed {
    margin: 0 calc(-1 * var(--halo-card-padding));
    padding-left: var(--halo-card-padding);
    padding-right: var(--halo-card-padding);
    background: var(--halo-accent-soft);
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

  .agent {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.3rem 0 0;
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .agent span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .agent select,
  .agent input {
    flex: 1;
    min-width: 0;
    padding: 0.15rem 0.35rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
    font: inherit;
    font-size: 0.72rem;
  }

  .agent button {
    flex: none;
    padding: 0.1rem 0.4rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    font: inherit;
    font-size: 0.7rem;
    cursor: pointer;
  }

  .agent button:hover {
    color: var(--halo-text-main);
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
