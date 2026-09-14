# rosso

A self-hosted feed reader that polls on its own schedule and uses a local LLM to
tell you which of the 200 unread items are worth your time.

Two things set it apart from the readers it grew out of:

- **Fetching is a server-side loop, not a UI action.** The backend polls feeds on
  a schedule whether or not a browser is open, with conditional GETs and per-feed
  adaptive intervals. The UI reads the database.
- **It ranks, summarizes and de-duplicates.** A local Ollama model on the LAN
  writes a one-line summary per item, scores it against a written interest
  profile, collapses the same story across five feeds into one card, and produces
  a daily digest.

The LLM layer is additive. Every LLM-derived column is nullable and every LLM call
retries through a job queue, so when the model host is off rosso is still a
complete RSS reader.

## Using it

Paste anything into the add-feed box — a feed URL, a site URL, or a bare
hostname (`example.com`). rosso follows the page's `<link rel="alternate">` tags
to find the feed, subscribes, and keeps the items from that same fetch so a new
feed has content immediately.

Keyboard: `j`/`k` next/previous, `o` open the original, `c` open the discussion,
`m` toggle read, `s` star, `Esc` close.

Aggregator feeds (Hacker News, Lobsters, a subreddit) ship no article body —
just a link and a pointer to the thread. rosso detects those bodies and drops
them rather than printing "Comments" where the article should be, and surfaces
the discussion as its own link beside the article.

Open settings and write, in your own words, what you want to read. From then on
every item gets a two-sentence summary and a 0-100 score against that
description, and the **interesting** view holds the unread ones above your
cutoff, best first. Thumbs up or down on an item become labelled examples in the
scoring prompt, so the judgement drifts toward yours.

Editing the profile re-scores everything already summarized — cheaply, because
the summary is reused and only the score is redone. Leave the profile empty and
items are summarized but never scored; a number with nothing to judge against
would look exactly like a real one.

When a feed gives only a teaser, rosso fetches the linked page and reduces it to
the article. A background worker drains the backlog one request at a time, and
opening an item that has not been reached yet extracts it on the spot. Feeds that
already publish full text are never fetched twice. `ROSSO_EXTRACT=0` turns the
whole thing off.

## Stack

Rust (axum) backend embedding a SvelteKit SPA, SQLite for everything, shipped as
one arm64 scratch container.

## Development

```sh
./install-hooks.sh                # once, after cloning
just install
cp backend/.env.example backend/.env
just dev                          # backend :3008 + Vite :5173
```

`just` lists the rest — `build`, `lint`, `format`, `test`, `test-integration`.

Local dev runs with `DEV_AUTH=1`, which bypasses the forward-auth gate that
oauth2-proxy provides in production.

## Documentation

- `CLAUDE.md` — repo map and the invariants a change could silently break.
- `SECURITY.md` — threat model and trust boundaries.
