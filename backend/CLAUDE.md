# backend

axum service. Serves `/status`, the `/api/*` surface, and the built SPA.

## Modules

```text
lib.rs        boot: dotenv → tracing → crypto provider → Config → Db → poller → serve
config.rs     env → Config; the four contract fields are fixed by the house seam
db.rs         Arc<Mutex<Connection>> wrapper + the declarative schema
store.rs      every SQL statement, plus the structs the SPA mirrors
api.rs        the /api/* handlers
events.rs     broadcast channel behind /api/stream (SSE)
extract.rs    readability full-text: background worker + the on-demand path
settings.rs   user-editable settings rows; config supplies the defaults
llm/          ollama client, enrichment worker, embeddings + dedupe, the daily
              digest, prompts, the health probe
util.rs       fnv1a — persisted hashes, so never DefaultHasher
routes.rs     router, /status, SPA fallback handler, CSP layer
auth.rs       forward-auth extractor (+ the dev_auth bypass)
error.rs      AppError → IntoResponse
retention.rs  prunes read items once a retention window is set
feed/         fetch (conditional GET + size cap), parse (feed-rs + sanitize),
              discover (url → feed url), schedule (adaptive interval), poller,
              opml (import/export), favicon (site icons, inlined)
shutdown.rs   bounded graceful drain
```

## Notes that bite

- **`reqwest` uses `rustls-no-provider`.** Every crate that builds a client must
  install the ring provider first, or `Client::builder().build()` *panics*
  (it does not return an error). `run_server` does it for the binary; the
  integration harness does its own.
- **SPA fallback is a handler, not `ServeDir.not_found_service`.** The latter
  leaks a 404 status onto every client route.
- **`db.with(|c| …)` blocks a runtime worker thread** for the closure's duration.
  Keep closures short; never hold one across an await.
- **The shutdown watchdog exists because SSE streams never close on their own** —
  without the bounded drain, `with_graceful_shutdown` would wait forever.
- **An optional filter must still name its bound parameter.** rusqlite rejects a
  parameter the statement doesn't mention, so `list_items` writes
  `:feed_id IS NULL OR …` rather than dropping the clause — otherwise an
  unfiltered list 500s.
- **`Entry::id` from feed-rs is not always stable.** It synthesizes one from the
  link and title when the source has no guid, but falls back to a *random UUID*
  when there is neither — which would re-insert the item on every poll.
  `feed::parse::stable_guid` detects that case and hashes the content instead.
- **`truncated` and `extracted` mean different things.** `truncated` records what
  the feed shipped and never changes; `extracted` records whether the article
  itself has since been fetched. The extraction worker's candidate set is
  `truncated AND NOT extracted`, which is why a full-text feed costs no outbound
  requests at all.
- **Readability's types are not `Send`.** The whole parse must stay inside one
  `spawn_blocking` and hand back owned `String`s.
- **Integration fixtures must serve every URL they hand the backend.** An item
  whose link points at the real internet will actually be fetched now that
  extraction runs on open — one test escaped that way and started depending on a
  stranger's uptime.
- **Extraction refuses internal addresses**, and the harness sets
  `ROSSO_EXTRACT_ALLOW_PRIVATE=1` so wiremock's loopback works. Without that flag
  every extraction test would go green by being refused before it reached what it
  was testing — if you add one, check *why* it passes.
- **`sqlite-vec` does not build; do not reach for it again.** 0.1.9 typedefs
  `u_int64_t`, a glibc/BSD name musl does not define, so it fails on *any* musl
  target — the cross-compile was never the problem. 0.1.10-alpha.4 removed that
  block but `#include`s `sqlite-vec-diskann.c`, which the published crate omits,
  so it fails everywhere including macOS. Embeddings are a `BLOB` of
  little-endian f32 in `item_embeddings`, unit-normalized on write so cosine is a
  dot product — the fallback the plan named, and the shape `chat` already runs.
- **A cluster is identified by its head's own id**, so the head carries
  `cluster_id = id` and every member carries the head's. That is what lets the
  list, the feed counts and the topic counts filter to one row per story with a
  column comparison rather than a correlated subquery per row — and why all three
  must carry the same clause or the sidebar will count rows the list never shows.
- **A favicon is stored, never linked.** `feeds.icon` holds a `data:` URI the
  backend fetched once. An `<img>` pointing at the publisher's host would
  announce the reader to twenty sites on every page load, from whatever network
  it was opened on — which is most of what a self-hosted reader exists to avoid.
- **`sanitize` promotes lazy image sources before ammonia runs.** ammonia keeps
  only `align alt height src width` on an `img`, so `data-src` is stripped and a
  one-pixel placeholder is all that survives — the article renders with blanks
  where its pictures were, which looks exactly like a publisher who shipped
  none. The promotion has to happen *before* sanitizing; afterwards there is
  nothing left to promote.
- **Retention is off by default and never deletes three things**: starred,
  unread, and anything a digest refers to. The first two are the reader saying it
  still matters; the third is what keeps a digest from becoming dead links.
- **Days are UTC, and the digest hour says so.** The runtime image is `scratch`
  and carries no tzdata, so `chrono::Local` resolves to UTC whatever `TZ` is set
  to. A "local hour" setting that quietly is not one is worse than an honest UTC
  one; the settings dialog labels it. Shipping tzdata to fix this is a real
  choice, not an oversight to correct in passing.
- **A model's item ids are not trustworthy just because the schema said
  `integer`.** The digest asks for the ids in each thread so the reader can link
  to them, and `prune` drops the ones that were not in the candidate list. A
  constrained *type* is not a constrained *value*.
- **Search does not collapse clusters.** The list hiding a duplicate is the
  feature; a *search* that hid the report you went looking for, because another
  outlet ran it first, is a bug you cannot see from the outside.
- **The event stream is additive, like the LLM.** Nothing it carries is state
  the reader cannot get from a reload, nothing is persisted, and no caller checks
  whether a send reached anyone — a tab with the page closed is the normal case.
  A change that makes a count correct *only* via the stream is a bug.
- **Raw input never reaches FTS5 `MATCH`.** An unbalanced quote or a leading `-`
  is a syntax error, not an empty result, so `fts_query` re-emits every token
  quoted. A query it empties means "matched nothing", never "no filter" — the
  distinction is decided before the SQL is built.
- **Work sets are derived from row state, not pushed onto a queue.** Extraction
  keys off `truncated AND NOT extracted`; enrichment off `enriched_at IS NULL OR
  scored_profile IS NOT <current>`. Editing the interest profile is therefore its
  own invalidation — nothing walks the table to enqueue rescores. There is no
  jobs table, and adding one needs a reason beyond "the plan said so".
- **The two workers are ordered by a SQL clause, not a sleep.** A truncated item
  holds the *teaser* until extraction replaces it, so enriching first would
  summarize the teaser and never look again. `due_for_enrichment` excludes items
  still pending extraction; no boot delay is load-bearing.
- **`format` constrains the JSON, not the packaging around it.** gemma on the
  real host prefixes answers with a bare `json` line and no fence. `extract_json`
  takes the outermost braces rather than enumerating wrappers.
- **A refusal is not a retry.** 401/403/404/410 from an article page retire the
  item after one attempt instead of spending the budget to hear the same answer
  twice more; the budget is for a host that is slow or briefly broken. Some
  publishers sit behind a bot challenge no HTTP client can pass (`cf-mitigated`
  in the response says so outright) — that is a host declining, and the reader
  falls back to the feed's own teaser rather than working around it.
- **Attempt counters reset on success.** They exist to retire something
  permanently broken; left cumulative, three unlucky failures over a month would
  retire a healthy item.
- **Layout cleanup is deliberately small.** `drop_empty_elements` removes the
  wrapper scaffolding that stripping a publisher's classes leaves behind, and the
  reader's CSS supplies the typography the stylesheet used to. Beyond that, an
  ugly article is a CSS question, not a parsing one — yarr, the reference
  implementation here, does no structural cleanup at all.
