<script lang="ts">
  import X from "@lucide/svelte/icons/x";

  import { reader } from "$lib/stores/reader.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const settings = $derived(reader.settings);

  let profile = $state("");
  let threshold = $state(65);
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
      model = settings.llm_model;
      seeded = true;
    }
  });

  async function save() {
    saving = true;
    saved = false;
    try {
      await reader.saveSettings({
        interest_profile: profile,
        score_threshold: threshold,
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

  .row {
    display: flex;
    gap: 0.75rem;
    align-items: flex-end;
  }

  .row span {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    width: 6rem;
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
