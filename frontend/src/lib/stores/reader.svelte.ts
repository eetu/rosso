// Shared reader state: which view is active, the feeds, the current page of
// items, and the open item. Anything two components both read lives here rather
// than being threaded through the page as props.

import {
  api,
  type Feed,
  type Item,
  type ItemDetail,
  type ItemView,
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
let loadingItems = $state(false);
let error = $state<string | null>(null);
let settings = $state<SettingsResponse | null>(null);
let topics = $state<Topic[]>([]);

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
    });
    items = page.items;
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
  get topics() {
    return topics;
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
    open = null;
    await loadItems();
  },

  /** Debouncing belongs to the box; this runs whatever it is handed. */
  async search(next: string) {
    if (next === q) return;
    q = next;
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

  closeItem() {
    open = null;
    opening = null;
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

  async markAllRead() {
    await api.markRead(feedId ?? undefined);
    await Promise.all([refreshCounts(), loadItems()]);
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
