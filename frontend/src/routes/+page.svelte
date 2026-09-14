<script lang="ts">
  import CheckCheck from "@lucide/svelte/icons/check-check";
  import SlidersHorizontal from "@lucide/svelte/icons/sliders-horizontal";

  import { api } from "$lib/api";
  import ItemList from "$lib/components/ItemList.svelte";
  import Reader from "$lib/components/Reader.svelte";
  import SettingsDialog from "$lib/components/SettingsDialog.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Wordmark from "$lib/components/Wordmark.svelte";
  import { createResource } from "$lib/createResource.svelte";
  import { reader } from "$lib/stores/reader.svelte";

  const status = createResource(() => api.status(), { intervalMs: 60_000 });
  $effect(() => status.start());
  $effect(() => {
    void reader.init();
  });

  let settingsOpen = $state(false);

  const pane = $derived.by(() => {
    if (reader.opening !== null && reader.open?.id !== reader.opening)
      return "loading";
    return reader.open ? "item" : "list";
  });

  const selectedIndex = $derived(
    reader.items.findIndex((i) => i.id === reader.open?.id),
  );

  function step(delta: number) {
    if (reader.items.length === 0) return;
    const next = selectedIndex < 0 ? 0 : selectedIndex + delta;
    const item =
      reader.items[Math.max(0, Math.min(next, reader.items.length - 1))];
    if (item) void reader.openItem(item.id);
  }

  function onKeydown(event: KeyboardEvent) {
    // Never steal a key from a field the user is typing in.
    const target = event.target as HTMLElement | null;
    if (
      target?.matches("input, textarea, select") ||
      event.metaKey ||
      event.ctrlKey
    )
      return;

    const open = reader.open;
    switch (event.key) {
      case "j":
        step(1);
        break;
      case "k":
        step(-1);
        break;
      case "o":
        if (open?.url) window.open(open.url, "_blank", "noreferrer");
        break;
      case "c":
        if (open?.comments_url)
          window.open(open.comments_url, "_blank", "noreferrer");
        break;
      case "m":
        if (open) void reader.setRead(open.id, !open.read);
        break;
      case "s":
        if (open) void reader.setStarred(open.id, !open.starred);
        break;
      case "Escape":
        if (settingsOpen) {
          settingsOpen = false;
        } else {
          reader.closeItem();
        }
        break;
      default:
        return;
    }
    event.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<header class="bar">
  <Wordmark />
  <div class="right">
    <button class="mark" onclick={() => reader.markAllRead()}>
      <CheckCheck size={14} /> mark read
    </button>
    <button
      class="mark"
      onclick={() => (settingsOpen = true)}
      aria-label="settings"
    >
      <SlidersHorizontal size={14} />
    </button>
    <span class="build">
      {#if status.data}
        v{status.data.version}{#if !status.data.llm_configured}
          · no llm{:else if !status.data.llm_available}
          · llm offline{/if}
      {:else if status.error}
        offline
      {/if}
    </span>
  </div>
</header>

<main>
  <Sidebar />
  <section class="list" class:with-reader={pane !== "list"}>
    {#if reader.error}
      <p class="error">{reader.error}</p>
    {/if}
    <ItemList selectedId={reader.open?.id ?? reader.opening} />
  </section>
  {#if pane === "item" && reader.open}
    <Reader item={reader.open} />
  {:else if pane === "loading"}
    <!-- The wait is the backend fetching and reducing the linked page, which is
         the only time rosso makes the reader wait on anything. -->
    <p class="loading">fetching the article…</p>
  {/if}
</main>

{#if settingsOpen}
  <SettingsDialog onclose={() => (settingsOpen = false)} />
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex: none;
    padding: 0.6rem 0.75rem;
    padding-top: calc(0.6rem + env(safe-area-inset-top));
    border-bottom: 1px solid var(--halo-border);
    background: var(--halo-bg-main);
  }

  .right {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .mark {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.5rem;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
  }

  .mark:hover {
    color: var(--halo-text-main);
  }

  .build {
    font-family: var(--halo-font-heading);
    font-size: 0.72rem;
    color: var(--halo-text-muted);
  }

  main {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .list {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
    border-right: 1px solid var(--halo-border);
  }

  .list.with-reader {
    flex: 0 0 22rem;
  }

  .error {
    margin: 0;
    padding: 0.5rem 0.75rem;
    color: var(--halo-error);
    font-size: 0.8rem;
  }

  .loading {
    flex: 1;
    margin: 0;
    padding: 1rem 1.25rem;
    color: var(--halo-text-muted);
    font-size: 0.85rem;
  }

  /* Narrow screens drill down instead of showing three panes at once. */
  @media (max-width: 800px) {
    .list.with-reader {
      display: none;
    }
  }
</style>
