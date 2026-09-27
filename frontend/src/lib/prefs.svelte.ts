// Layout preferences: how this reader likes the window arranged. Global, not
// per-feed or per-view, and kept in the browser rather than on the server —
// the right sidebar width on a laptop is the wrong one on a desktop monitor,
// and neither is a fact about the feeds.
//
// One storage key, behind this module, so a private window or a blocked
// `localStorage` degrades to "the app forgets" rather than throwing.

const KEY = "rosso.prefs";

/** Bounds on the sidebar, in CSS px. Below the floor the counts collide with
 *  the names; above the ceiling it is eating the list it exists to navigate. */
export const SIDEBAR_MIN = 192;
export const SIDEBAR_MAX = 480;
export const SIDEBAR_DEFAULT = 240;

type Stored = { sidebarWidth?: number };

function read(): Stored {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Stored) : {};
  } catch {
    return {};
  }
}

function write(value: Stored) {
  try {
    localStorage.setItem(KEY, JSON.stringify(value));
  } catch {
    // Private mode, quota, or blocked site data. The width still applies for
    // this session; it just won't be here next time.
  }
}

export function clampSidebar(px: number): number {
  return Math.round(Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, px)));
}

class Prefs {
  sidebarWidth = $state(clampSidebar(read().sidebarWidth ?? SIDEBAR_DEFAULT));

  setSidebarWidth(px: number) {
    this.sidebarWidth = clampSidebar(px);
  }

  /** Written once a drag ends, not on every pointer move. */
  save() {
    write({ sidebarWidth: this.sidebarWidth });
  }
}

export const prefs = new Prefs();
