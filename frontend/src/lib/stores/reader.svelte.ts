// Shared reader state: which view is active, the feeds, the current page of
// items, and the open item. Anything two components both read lives here rather
// than being threaded through the page as props.

import {
  api,
  type Digest,
  type Feed,
  type Item,
  type ItemDetail,
  type ItemView,
  type LiveEvent,
  type SearchMode,
  type Settings,
  type SettingsResponse,
  type Topic,
} from "$lib/api";

let feeds = $state<Feed[]>([]);
let items = $state<Item[]>([]);
let open = $state<ItemDetail | null>(null);
// The id being fetched. Opening an item whose article has not been extracted yet
// blocks on that fetch, so the pane needs something to show meanwhile.
let opening = $state<number | null>(null);
let view = $state<ItemView>("unread");
let feedId = $state<number | null>(null);
let tag = $state<string | null>(null);
let q = $state("");
let mode = $state<SearchMode>("text");
// What the last search actually ran as, which is not always what was asked for.
let ranAs = $state<SearchMode | null>(null);
let loadingItems = $state(false);
let error = $state<string | null>(null);
let settings = $state<SettingsResponse | null>(null);
let topics = $state<Topic[]>([]);
// The digest takes over both content panes: the list shows the days, the reader
// shows the day. It is a document, not a filtered list, so it does not belong in
// `view` alongside unread and starred.
let digestMode = $state(false);
let digestDays = $state<string[]>([]);
let digest = $state<Digest | null>(null);

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

async function loadFeeds() {
  try {
    feeds = await api.feeds();
  } catch (e) {
    error = message(e);
  }
}

async function loadItems() {
  loadingItems = true;
  try {
    const page = await api.items({
      view,
      feed_id: feedId ?? undefined,
      tag: tag ?? undefined,
      q: q || undefined,
      mode: q ? mode : undefined,
    });
    items = page.items;
    ranAs = page.mode;
    error = null;
  } catch (e) {
    error = message(e);
  } finally {
    loadingItems = false;
  }
}

async function loadTopics() {
  try {
    topics = await api.topics();
  } catch (e) {
    error = message(e);
  }
}

/**
 * Feed counts and topic counts are both counts of unread items, so whatever
 * moves one moves the other — reloading them together keeps the sidebar from
 * showing two numbers that disagree.
 */
async function refreshCounts() {
  await Promise.all([loadFeeds(), loadTopics()]);
}

let countsTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * The live channel fires once per item, and draining a morning's backlog fires
 * hundreds in a row. The sidebar only ever needs the number they end on.
 */
function refreshCountsSoon() {
  if (countsTimer) return;
  countsTimer = setTimeout(() => {
    countsTimer = null;
    void refreshCounts();
  }, 2000);
}

// Not `$state`: only the live handler reads it, to decide whether reloading the
// list under the user would yank the page they are reading out from under them.
let listAtTop = true;

function handle(event: LiveEvent) {
  if (event.kind === "item-enriched") {
    const { item_id, ...enriched } = event;
    items = items.map((i) => (i.id === item_id ? { ...i, ...enriched } : i));
    if (open?.id === item_id) open = { ...open, ...enriched };
    // A new score can move an item into the interesting view, and new tags can
    // create a topic, so the sidebar moves too.
    refreshCountsSoon();
    return;
  }
  refreshCountsSoon();
  // New items go at the top, so a reload is only invisible when the top is what
  // you are looking at and nothing is open behind it. Otherwise the counts move
  // and the list waits until you next ask for it.
  if (event.kind === "items-new" && listAtTop && !open && !opening && !q) {
    void loadItems();
  }
}

export const reader = {
  get feeds() {
    return feeds;
  },
  get items() {
    return items;
  },
  get open() {
    return open;
  },
  get opening() {
    return opening;
  },
  get view() {
    return view;
  },
  get feedId() {
    return feedId;
  },
  get tag() {
    return tag;
  },
  get q() {
    return q;
  },
  get mode() {
    return mode;
  },
  /** What the last search ran as — `"text"` while `mode` says `"semantic"` is
   * the model host being asleep, which the box says out loud. */
  get ranAs() {
    return ranAs;
  },
  get topics() {
    return topics;
  },
  get digestMode() {
    return digestMode;
  },
  get digestDays() {
    return digestDays;
  },
  get digest() {
    return digest;
  },
  /** Whether the detail pane has anything in it, whichever mode is on. */
  get detailOpen() {
    return open !== null || opening !== null || (digestMode && digest !== null);
  },
  get loadingItems() {
    return loadingItems;
  },
  get error() {
    return error;
  },
  get settings() {
    return settings;
  },
  get totalUnread() {
    return feeds.reduce((sum, f) => sum + f.unread, 0);
  },

  async init() {
    await Promise.all([refreshCounts(), loadItems(), this.loadSettings()]);
  },

  /**
   * Open the live channel. Returns the teardown, so the page can hand it
   * straight to an effect.
   *
   * Everything it carries is something a reload would show anyway — it is how a
   * tab left open all morning stops being a morning old, not a source of truth.
   * `EventSource` reconnects on its own, so a dropped connection needs nothing
   * here; the reconnect's first reload is what recovers whatever was missed.
   */
  live() {
    const source = new EventSource("/api/stream");
    source.onmessage = (e) => handle(JSON.parse(e.data) as LiveEvent);
    return () => source.close();
  },

  /** The list tells the store whether reloading it under the user is safe. */
  setListAtTop(atTop: boolean) {
    listAtTop = atTop;
  },

  async loadSettings() {
    try {
      settings = await api.settings();
    } catch (e) {
      error = message(e);
    }
  },

  /**
   * Saving a profile costs nothing here, but it makes every already-scored item
   * a rescore candidate on the backend — so the list is reloaded to pick the new
   * scores up as they land.
   */
  async saveSettings(patch: Partial<Settings>) {
    await api.saveSettings(patch);
    await Promise.all([this.loadSettings(), loadItems()]);
  },

  /**
   * A feed and a topic are alternative ways to narrow the same list, so
   * choosing one clears the other — holding both would leave a selection the
   * sidebar shows in two places at once.
   */
  async select(next: {
    view?: ItemView;
    feedId?: number | null;
    tag?: string | null;
  }) {
    // A search spans the archive, so the view selector does nothing while one is
    // running — picking a view is how you say you are done searching. A feed or a
    // topic still narrows a search, so those leave it alone.
    if (next.view !== undefined) {
      view = next.view;
      q = "";
    }
    if (next.feedId !== undefined) {
      feedId = next.feedId;
      if (next.feedId !== null) tag = null;
    }
    if (next.tag !== undefined) {
      tag = next.tag;
      if (next.tag !== null) feedId = null;
    }
    // Choosing anything from the sidebar is leaving the digest.
    digestMode = false;
    open = null;
    await loadItems();
  },

  /** Open the digest section, on the most recent day there is. */
  async showDigests() {
    digestMode = true;
    open = null;
    opening = null;
    try {
      digestDays = await api.digestDays();
      error = null;
    } catch (e) {
      error = message(e);
      return;
    }
    const newest = digestDays[0];
    if (newest && digest?.day !== newest) await this.openDigest(newest);
  },

  async openDigest(day: string) {
    try {
      digest = await api.digest(day);
      error = null;
    } catch (e) {
      error = message(e);
    }
  },

  /**
   * Write one now rather than waiting for the hour. Throws so the button can
   * report a day that held too little, or a model host that is not answering.
   */
  async makeDigest(day?: string) {
    const made = await api.makeDigest(day);
    digestDays = await api.digestDays();
    await this.openDigest(made.day);
    return made;
  },

  /** Debouncing belongs to the box; this runs whatever it is handed. */
  async search(next: string, nextMode: SearchMode = mode) {
    if (next === q && nextMode === mode) return;
    q = next;
    mode = nextMode;
    if (!q) ranAs = null;
    open = null;
    await loadItems();
  },

  /** Throws so the form can show the failure inline; everything else swallows. */
  async addFeed(url: string) {
    const feed = await api.addFeed(url);
    await Promise.all([refreshCounts(), loadItems()]);
    return feed;
  },

  /** Throws like `addFeed`, so the dialog can report a file it made nothing of. */
  async importOpml(xml: string) {
    const result = await api.importOpml(xml);
    await Promise.all([refreshCounts(), loadItems()]);
    return result;
  },

  async removeFeed(id: number) {
    await api.deleteFeed(id);
    if (feedId === id) feedId = null;
    await Promise.all([refreshCounts(), loadItems()]);
  },

  async refreshFeed(id: number) {
    await api.refreshFeed(id);
    await Promise.all([refreshCounts(), loadItems()]);
  },

  /** Opening an item marks it read, which is what every reader does. */
  async openItem(id: number) {
    opening = id;
    try {
      const detail = await api.item(id);
      // A second open may have overtaken this one while it waited on extraction.
      if (opening !== id) return;
      open = detail;
      if (!detail.read) {
        await this.setRead(id, true);
      }
    } catch (e) {
      error = message(e);
    } finally {
      if (opening === id) opening = null;
    }
  },

  /**
   * The back button. In the digest section it unwinds one step at a time — an
   * item opened from a thread returns to the digest, and the digest returns to
   * the list of days, rather than either dropping you straight out.
   */
  closeItem() {
    if (open !== null || opening !== null) {
      open = null;
      opening = null;
      return;
    }
    if (digestMode) digest = null;
  },

  async setRead(id: number, read: boolean) {
    const updated = await api.updateItem(id, { read });
    patchLocal(updated);
    // The unread counts live on the feed rows, so they need a reload to match.
    await refreshCounts();
  },

  async setStarred(id: number, starred: boolean) {
    patchLocal(await api.updateItem(id, { starred }));
  },

  /** A thumb steers future scoring; pressing the same one again clears it. */
  async setFeedback(id: number, feedback: number) {
    const current = open?.id === id ? open.feedback : 0;
    patchLocal(
      await api.updateItem(id, {
        feedback: current === feedback ? 0 : feedback,
      }),
    );
  },

  /**
   * What the button says: mark read what you are looking at. It used to send
   * only the feed, so pressing it while filtered to one topic cleared the whole
   * archive — a destructive action on a selection it could not see.
   */
  get markReadScope() {
    if (feedId !== null) {
      return feeds.find((f) => f.id === feedId)?.title ?? "this feed";
    }
    if (tag) return `the ${tag} topic`;
    if (q) return `these search results`;
    return null;
  },

  async markAllRead() {
    const { marked } = await api.markRead({
      feed_id: feedId ?? undefined,
      tag: tag ?? undefined,
      q: q || undefined,
    });
    await Promise.all([refreshCounts(), loadItems()]);
    return marked;
  },
};

/** Keep the list row and the open item in step without refetching the page. */
function patchLocal(updated: ItemDetail) {
  items = items.map((i) =>
    i.id === updated.id ? { ...i, ...toRow(updated) } : i,
  );
  if (open?.id === updated.id) open = updated;
}

function toRow(detail: ItemDetail): Item {
  const { content_html: _content, ...row } = detail;
  return row;
}
