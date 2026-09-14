# backend

axum service. Serves `/status`, the `/api/*` surface, and the built SPA.

## Modules

```text
lib.rs        boot: dotenv → tracing → crypto provider → Config → Db → poller → serve
config.rs     env → Config; the four contract fields are fixed by the house seam
db.rs         Arc<Mutex<Connection>> wrapper + the declarative schema
store.rs      every SQL statement, plus the structs the SPA mirrors
api.rs        the /api/* handlers
extract.rs    readability full-text: background worker + the on-demand path
settings.rs   user-editable settings rows; config supplies the defaults
llm/          ollama client, enrichment worker, prompts, the health probe
util.rs       fnv1a — persisted hashes, so never DefaultHasher
routes.rs     router, /status, SPA fallback handler, CSP layer
auth.rs       forward-auth extractor (+ the dev_auth bypass)
error.rs      AppError → IntoResponse
feed/         fetch (conditional GET + size cap), parse (feed-rs + sanitize),
              discover (url → feed url), schedule (adaptive interval), poller
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
- **Attempt counters reset on success.** They exist to retire something
  permanently broken; left cumulative, three unlucky failures over a month would
  retire a healthy item.
- **Layout cleanup is deliberately small.** `drop_empty_elements` removes the
  wrapper scaffolding that stripping a publisher's classes leaves behind, and the
  reader's CSS supplies the typography the stylesheet used to. Beyond that, an
  ugly article is a CSS question, not a parsing one — yarr, the reference
  implementation here, does no structural cleanup at all.
