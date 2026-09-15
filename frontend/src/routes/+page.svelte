<script lang="ts">
  import ArrowLeft from "@lucide/svelte/icons/arrow-left";
  import CheckCheck from "@lucide/svelte/icons/check-check";
  import Menu from "@lucide/svelte/icons/menu";
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
  let navOpen = $state(false);

  /**
   * Which single pane a phone shows. Desktop ignores this entirely and lays all
   * three out side by side; the class is only consumed inside a media query.
   *
   * The open item wins over the nav, so reading is never interrupted by a
   * sidebar that was left open behind it.
   */
  const mobilePane = $derived(
    reader.open || reader.opening !== null ? "item" : navOpen ? "nav" : "list",
  );

  const backLabel = $derived(
    reader.open ? "back to the list" : navOpen ? "close sources" : "sources",
  );

  function back() {
    if (reader.open || reader.opening !== null) {
      reader.closeItem();
    } else {
      navOpen = !navOpen;
    }
  }

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
  <!-- The only way between the three panes on a phone, where exactly one of
       them is on screen at a time. Invisible on desktop, which shows all three
       at once and needs no navigation at all. -->
  <button class="pane-nav" onclick={back} aria-label={backLabel}>
    {#if reader.open}<ArrowLeft size={18} />{:else}<Menu size={18} />{/if}
  </button>
  <Wordmark />
  <div class="right">
    <button class="mark" onclick={() => reader.markAllRead()}>
      <CheckCheck size={14} /> <span class="label">mark read</span>
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

<main class="showing-{mobilePane}">
  <Sidebar onselect={() => (navOpen = false)} />
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
    gap: 0.5rem;
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

  .pane-nav {
    display: none;
    place-items: center;
    padding: 0.25rem;
    border: none;
    border-radius: var(--halo-radius);
    background: none;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  /* One pane at a time, with the header button moving between them. Showing the
     sidebar alongside the content left the content about 150px wide, and hiding
     the list when an item opened stranded the reader with no way back to it. */
  @media (max-width: 800px) {
    .pane-nav {
      display: grid;
    }

    /* The label goes; the icon carries it. */
    .mark .label {
      display: none;
    }

    .build {
      display: none;
    }

    main > :global(aside) {
      display: none;
    }

    main.showing-nav > :global(aside) {
      display: flex;
    }

    main.showing-nav .list,
    main.showing-item .list {
      display: none;
    }

    /* Undo the desktop split: whichever pane is showing gets the whole width. */
    .list.with-reader {
      flex: 1;
    }
  }
</style>
