<script lang="ts">
  import X from "@lucide/svelte/icons/x";

  import { reader } from "$lib/stores/reader.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const settings = $derived(reader.settings);

  let profile = $state("");
  let threshold = $state(65);
  let dedupe = $state(0.9);
  let model = $state("");
  let saving = $state(false);
  let saved = $state(false);
  let seeded = false;

  $effect(() => {
    // Seed the fields once, from whatever the server had. Re-seeding on every
    // settings reload would overwrite whatever the user is halfway through
    // typing.
    if (settings && !seeded) {
      profile = settings.interest_profile;
      threshold = settings.score_threshold;
      dedupe = settings.dedupe_threshold;
      model = settings.llm_model;
      seeded = true;
    }
  });

  let file: HTMLInputElement | null = $state(null);
  let importing = $state(false);
  let importReport = $state("");

  async function runImport(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const chosen = input.files?.[0];
    if (!chosen) return;
    importing = true;
    importReport = "";
    try {
      const { added, skipped } = await reader.importOpml(await chosen.text());
      importReport = `${added} added, ${skipped} already subscribed. they fetch on the next poll.`;
    } catch (e) {
      importReport = e instanceof Error ? e.message : String(e);
    } finally {
      importing = false;
      // Cleared so choosing the same file twice fires `change` the second time.
      input.value = "";
    }
  }

  async function save() {
    saving = true;
    saved = false;
    try {
      await reader.saveSettings({
        interest_profile: profile,
        score_threshold: threshold,
        dedupe_threshold: dedupe,
        llm_model: model,
      });
      saved = true;
    } finally {
      saving = false;
    }
  }
</script>

<div
  class="backdrop"
  role="button"
  tabindex="-1"
  onclick={onclose}
  onkeydown={(e) => e.key === "Escape" && onclose()}
></div>

<section class="panel halo-card" aria-label="settings">
  <header>
    <h2>settings</h2>
    <button onclick={onclose} aria-label="close"><X size={16} /></button>
  </header>

  <label for="profile">what you want to read</label>
  <textarea
    id="profile"
    bind:value={profile}
    rows="7"
    placeholder="rust, sqlite, self-hosting, embedded linux. not crypto, not startup drama."
  ></textarea>
  <p class="hint">
    {#if profile.trim() === ""}
      empty: items get a summary but no score.
    {:else}
      editing this re-scores everything already summarized.
    {/if}
  </p>

  <div class="row">
    <span>
      <label for="threshold">interesting at</label>
      <input
        id="threshold"
        type="number"
        bind:value={threshold}
        min="0"
        max="100"
      />
    </span>
    <span>
      <label for="dedupe">same story at</label>
      <input
        id="dedupe"
        type="number"
        bind:value={dedupe}
        min="0.5"
        max="1"
        step="0.01"
      />
    </span>
    <span class="grow">
      <label for="model">model</label>
      {#if settings && settings.models.length > 0}
        <select id="model" bind:value={model}>
          {#each settings.models as name (name)}
            <option value={name}>{name}</option>
          {/each}
        </select>
      {:else}
        <!-- The host is asleep, so there is nothing to enumerate. Free text
             keeps the field usable rather than showing an empty dropdown. -->
        <input id="model" type="text" bind:value={model} spellcheck="false" />
      {/if}
    </span>
  </div>

  <p class="hint">
    {#if dedupe >= 1}
      duplicates off: every item keeps its own row.
    {:else}
      items this alike collapse to one row. the rest are listed on it.
    {/if}
  </p>

  <div class="subs">
    <span class="label">subscriptions</span>
    <div class="actions">
      <!-- A link, not a fetch: the browser's own download path sends the
           forward-auth cookie and names the file from the header. `resolve()` is
           for SvelteKit routes, and this is the backend's. -->
      <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
      <a href="/api/opml/export" download="rosso.opml">export opml</a>
      <button onclick={() => file?.click()} disabled={importing}>
        {importing ? "importing…" : "import opml"}
      </button>
      <input
        bind:this={file}
        type="file"
        accept=".opml,.xml,text/xml,text/x-opml"
        onchange={runImport}
        hidden
      />
    </div>
  </div>
  {#if importReport}
    <p class="hint">{importReport}</p>
  {/if}

  <footer>
    <span class="hint">
      {#if settings && !settings.llm_available}
        model host unreachable — saved settings apply when it is back.
      {:else if saved}
        saved.
      {/if}
    </span>
    <button class="save" onclick={save} disabled={saving}>
      {saving ? "saving…" : "save"}
    </button>
  </footer>
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
    width: min(34rem, calc(100vw - 2rem));
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

  label {
    font-size: 0.75rem;
    color: var(--halo-text-muted);
  }

  textarea,
  input,
  select {
    width: 100%;
    padding: 0.4rem 0.55rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
    font: inherit;
    font-size: 0.85rem;
  }

  textarea {
    resize: vertical;
    line-height: 1.45;
  }

  input:focus-visible,
  textarea:focus-visible,
  select:focus-visible {
    outline: 2px solid var(--halo-accent);
    outline-offset: -1px;
  }

  /* `flex-end` lined the two columns up by their bottoms, and a number input and
     a select are not the same height — so the labels above them sat at
     different heights. Stretching instead puts both labels on one line and lets
     the fields settle underneath. */
  .row {
    display: flex;
    gap: 0.75rem;
    align-items: stretch;
  }

  .row span {
    display: flex;
    flex-direction: column;
    /* The field sits at the bottom of the column whatever the label does, which
       is what keeps the two inputs aligned with each other. */
    justify-content: flex-end;
    gap: 0.2rem;
    width: 6rem;
  }

  /* Both controls end up the same height, so neither column dictates the other. */
  .row input,
  .row select {
    height: 2.1rem;
  }

  .row .grow {
    flex: 1;
    width: auto;
    min-width: 0;
  }

  .hint {
    margin: 0;
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  .subs {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
    margin-top: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px solid var(--halo-border);
  }

  .subs .label {
    font-size: 0.75rem;
    color: var(--halo-text-muted);
  }

  .actions {
    display: flex;
    gap: 0.5rem;
  }

  .actions a,
  .actions button {
    padding: 0.25rem 0.55rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    color: var(--halo-text-muted);
    font-size: 0.75rem;
    text-decoration: none;
  }

  .actions a:hover,
  .actions button:hover {
    color: var(--halo-text-main);
  }

  .actions button:disabled {
    cursor: default;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-top: 0.75rem;
  }

  button {
    border: none;
    background: none;
    color: var(--halo-text-muted);
    font: inherit;
    cursor: pointer;
    border-radius: var(--halo-radius);
  }

  .save {
    padding: 0.35rem 0.9rem;
    background: var(--halo-accent);
    color: var(--halo-on-accent);
    font-size: 0.8rem;
  }

  .save:disabled {
    background: var(--halo-off-bg);
    color: var(--halo-text-muted);
    cursor: default;
  }
</style>
