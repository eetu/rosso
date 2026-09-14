# rosso — repo overview

A background-polling feed reader with a local-LLM summary/scoring layer. Sibling
of `../chat`, `../scribe`, `../nib`; deployed by `../keel`; the LLM host is the
Mac mini provisioned by `../mini`.

## Layout

```text
backend/      Rust axum service — polling, storage, LLM jobs, serves the SPA
frontend/     SvelteKit SPA (pure client, built to dist/ and embedded)
integration/  spawned-binary tests: real backend, wiremocked feeds + Ollama
```

## Conventions

- **The LLM is optional, always.** Every LLM-derived column on `items` is
  nullable, every LLM call goes through the retrying job queue, and no reader path
  reads one without a fallback. A change that makes the reader depend on Ollama
  being up is a bug.
- **Auth is the edge's job.** `auth: "edge"` in keel: oauth2-proxy vouches, the
  binary only checks `X-Auth-Request-User` is present. Single user — there is no
  per-user scoping anywhere, and adding a second user means a schema change, not a
  filter.
- **Timestamps are ISO 8601 TEXT.** Never let a column mix storage classes;
  SQLite sorts INTEGER before TEXT, so one stray integer silently reorders a feed.
- **The schema is declarative and re-applied every boot.** `user_version` is
  informational and does not gate migrations — it is read only for genuine
  one-shot data fixes. Add columns via the add-if-missing helper, never by
  editing a `CREATE TABLE` and expecting an existing DB to follow.
- **Feed content is untrusted.** Sanitize before storing, cap every response body,
  and never build a filesystem path from it.
- **Memory is the binding constraint.** The target board is a 1 GB Pi 4 running
  the whole house catalog; rosso's cap is 256 MB. Bound concurrency and response
  sizes rather than assuming headroom.

## Working on this repo

- Ports: backend `3008`, Vite `5173` (proxies `/api` and `/status`).
- `just dev` runs both; `DEV_AUTH=1` in `backend/.env` bypasses forward-auth.
- `ROSSO_OLLAMA_URL` unset = plain reader, no LLM features. That is a supported
  mode, not a broken one — test in it.
- Integration tests are `#[ignore]`: `just test-integration`.

## Out of scope

- Third-party sync APIs (Fever, Google Reader) — the web UI is the only client.
- Multi-user anything.
- Server-side rendering; the SPA is pure client, `fallback: index.html`.
