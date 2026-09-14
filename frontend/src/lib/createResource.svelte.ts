// A small SWR-ish data resource — the house alternative to swr/@tanstack/query.
// Runes work in `.svelte.ts` modules, so reactive state lives here and
// components read the getters.
//
//   const status = createResource(() => api.status(), { intervalMs: 30_000 });
//   $effect(() => status.start());

import { ApiError } from "./api";

type Options = {
  /** If set, refetch every N ms while started. Omit for one-shot. */
  intervalMs?: number;
};

export function createResource<T>(
  fetcher: () => Promise<T>,
  options: Options = {},
) {
  let data = $state<T | undefined>(undefined);
  let error = $state<ApiError | Error | undefined>(undefined);
  let loading = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;

  const load = async () => {
    loading = true;
    try {
      data = await fetcher();
      error = undefined;
    } catch (e) {
      error = e instanceof Error ? e : new Error(String(e));
    } finally {
      loading = false;
    }
  };

  return {
    get data() {
      return data;
    },
    get error() {
      return error;
    },
    get loading() {
      return loading;
    },
    /** Manual refetch, e.g. after a mutation. */
    refresh: load,
    /**
     * Start the resource. Call from an `$effect` so it tears down on unmount —
     * the returned cleanup clears any poll interval.
     */
    start() {
      void load();
      if (options.intervalMs) {
        timer = setInterval(load, options.intervalMs);
      }
      return () => this.stop();
    },
    stop() {
      if (timer) {
        clearInterval(timer);
        timer = undefined;
      }
    },
  };
}
