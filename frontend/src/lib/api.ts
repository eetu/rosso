// Thin fetch layer over the backend's JSON API. Types are hand-written to match
// the Rust structs — no codegen. Keep in sync with backend/src/routes.rs.

/** The unauth liveness payload from `/status`. */
export type StatusResponse = {
  service: string;
  version: string;
  db_healthy: boolean;
  /** A model host is set in the environment. */
  llm_configured: boolean;
  /** It actually answered. Only this one means summaries are coming. */
  llm_available: boolean;
  extract_enabled: boolean;
};

export type Feed = {
  id: number;
  url: string;
  site_url: string | null;
  /** Already resolved server-side: the user's override, else the feed's own. */
  title: string;
  folder_id: number | null;
  icon: string | null;
  unread: number;
  last_fetch_at: string | null;
  next_fetch_at: string;
  last_error: string | null;
  disabled: boolean;
};

export type Item = {
  id: number;
  feed_id: number;
  feed_title: string;
  url: string | null;
  /** Set when the discussion lives apart from the article, as on an aggregator. */
  comments_url: string | null;
  title: string;
  author: string | null;
  published_at: string | null;
  /** Null until the LLM has got to it — and forever, if it never does. */
  summary: string | null;
  score: number | null;
  score_reason: string | null;
  /** Topic tags from the model. Empty until the item has been enriched. */
  tags: string[];
  read: boolean;
  starred: boolean;
  /** -1, 0 or 1. Steers future scoring. */
  feedback: number;
};

export type Settings = {
  /** Free text. Empty means scoring is off and items are only summarized. */
  interest_profile: string;
  score_threshold: number;
  llm_model: string;
};

export type SettingsResponse = Settings & {
  /** Installed models, for the picker. Empty when the host is asleep. */
  models: string[];
  llm_available: boolean;
};

export type ItemDetail = Item & { content_html: string | null };

export type ItemView = "unread" | "starred" | "interesting" | "all";

export type ItemsQuery = {
  view?: ItemView;
  feed_id?: number;
  tag?: string;
  cursor?: string;
  limit?: number;
  /** Full-text. Searches the whole archive, so it overrides `view`. */
  q?: string;
};

/**
 * A push from `/api/stream`. Mirrors the `Event` enum in `backend/src/events.rs`
 * — `kind` is the serde tag, the rest are that variant's fields.
 */
export type LiveEvent =
  | { kind: "items-new"; feed_id: number; count: number }
  | { kind: "feed-updated"; feed_id: number }
  | {
      kind: "item-enriched";
      item_id: number;
      summary: string | null;
      score: number | null;
      score_reason: string | null;
      tags: string[];
    };

/** A tag seen often enough to be worth grouping by, with its unread count. */
export type Topic = {
  tag: string;
  unread: number;
};

/** Thrown for any non-2xx response; carries the HTTP status. */
export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    headers: {
      accept: "application/json",
      ...(init?.body ? { "content-type": "application/json" } : {}),
    },
    ...init,
  });
  if (!res.ok) {
    // The backend puts a human-readable reason in `error` for the statuses a
    // user can actually cause (a URL with no feed, a duplicate subscription).
    // Showing "POST /api/feeds → 400" instead would waste that.
    const detail = await res
      .json()
      .then((body: unknown) =>
        typeof body === "object" && body !== null && "error" in body
          ? String((body as { error: unknown }).error)
          : null,
      )
      .catch(() => null);
    throw new ApiError(
      res.status,
      detail ?? `${init?.method ?? "GET"} ${path} → ${res.status}`,
    );
  }
  if (res.status === 204) {
    return undefined as T;
  }
  return res.json() as Promise<T>;
}

// One object, one method per endpoint. `/status` is unauth; everything else
// lives under `/api` behind the forward-auth gate.
export const api = {
  status: () => request<StatusResponse>("/status"),

  feeds: () => request<{ feeds: Feed[] }>("/api/feeds").then((r) => r.feeds),
  /** `url` may be a feed, a site, or a bare hostname — the backend resolves it. */
  addFeed: (url: string) =>
    request<Feed>("/api/feeds", {
      method: "POST",
      body: JSON.stringify({ url }),
    }),
  deleteFeed: (id: number) =>
    request<unknown>(`/api/feeds/${id}`, { method: "DELETE" }),
  refreshFeed: (id: number) =>
    request<Feed>(`/api/feeds/${id}/refresh`, { method: "POST" }),

  items: (query: ItemsQuery = {}) => {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined) params.set(key, String(value));
    }
    const suffix = params.size > 0 ? `?${params}` : "";
    return request<{ items: Item[]; next_cursor: string | null }>(
      `/api/items${suffix}`,
    );
  },
  item: (id: number) => request<ItemDetail>(`/api/items/${id}`),
  updateItem: (
    id: number,
    patch: { read?: boolean; starred?: boolean; feedback?: number },
  ) =>
    request<ItemDetail>(`/api/items/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  topics: () =>
    request<{ topics: Topic[] }>("/api/topics").then((r) => r.topics),
  settings: () => request<SettingsResponse>("/api/settings"),
  saveSettings: (patch: Partial<Settings>) =>
    request<Settings>("/api/settings", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  /** The whole file's text. Export is a plain link — the browser downloads it. */
  importOpml: (xml: string) =>
    request<{ added: number; skipped: number }>("/api/opml/import", {
      method: "POST",
      headers: { accept: "application/json", "content-type": "text/xml" },
      body: xml,
    }),

  markRead: (feed_id?: number) =>
    request<{ marked: number }>("/api/items/mark-read", {
      method: "POST",
      body: JSON.stringify({ feed_id: feed_id ?? null }),
    }),
};
