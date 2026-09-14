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
} from "$lib/api";

let feeds = $state<Feed[]>([]);
let items = $state<Item[]>([]);
let open = $state<ItemDetail | null>(null);
// The id being fetched. Opening an item whose article has not been extracted yet
// blocks on that fetch, so the pane needs something to show meanwhile.
let opening = $state<number | null>(null);
let view = $state<ItemView>("unread");
let feedId = $state<number | null>(null);
let loadingItems = $state(false);
let error = $state<string | null>(null);
let settings = $state<SettingsResponse | null>(null);

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
    const page = await api.items({ view, feed_id: feedId ?? undefined });
    items = page.items;
    error = null;
  } catch (e) {
    error = message(e);
  } finally {
    loadingItems = false;
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
    await Promise.all([loadFeeds(), loadItems(), this.loadSettings()]);
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

  async select(next: { view?: ItemView; feedId?: number | null }) {
    if (next.view !== undefined) view = next.view;
    if (next.feedId !== undefined) feedId = next.feedId;
    open = null;
    await loadItems();
  },

  /** Throws so the form can show the failure inline; everything else swallows. */
  async addFeed(url: string) {
    const feed = await api.addFeed(url);
    await Promise.all([loadFeeds(), loadItems()]);
    return feed;
  },

  async removeFeed(id: number) {
    await api.deleteFeed(id);
    if (feedId === id) feedId = null;
    await Promise.all([loadFeeds(), loadItems()]);
  },

  async refreshFeed(id: number) {
    await api.refreshFeed(id);
    await Promise.all([loadFeeds(), loadItems()]);
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
    await loadFeeds();
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
    await Promise.all([loadFeeds(), loadItems()]);
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
