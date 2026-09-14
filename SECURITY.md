# Security

rosso is a self-hosted, single-household feed reader. It runs on a home Pi behind
oauth2-proxy forward-auth and Traefik — it is **not** a public, multi-tenant
service. The threat model is a hostile *feed*, not a hostile user: rosso fetches
and renders arbitrary third-party content chosen by its owner, so the job is to
keep publisher-controlled HTML from executing in the browser and to keep an
oversized or malicious response from exhausting a 1 GB board.

## Trust boundaries

- **Edge auth (forward-auth).** rosso trusts `X-Auth-Request-User` only because
  Traefik deletes any client-supplied copy of the header before forwarding (its
  `aliasHeadersStrategy: delete` on the websecure entrypoint). The `Auth`
  extractor 401s when the header is absent, as defence in depth against being
  reached by something other than the edge. The header is never logged (PII).
  `DEV_AUTH=1` / `ROSSO_OPEN=1` bypasses the gate — local dev and the integration
  harness only.

- **Feed content is untrusted input.** Item HTML is sanitized before it is ever
  stored or rendered; the CSP has no `script-src 'unsafe-inline'`, so even a
  sanitizer miss cannot execute. `img-src`/`media-src` allow any https origin
  because feed bodies embed publisher-hosted media, and stripping it would gut the
  reader; nothing else may be loaded cross-origin, and `connect-src` stays
  `'self'`.

- **Outbound fetches.** Feed and article fetches go through a shared client with
  a 10 s connect and 60 s total timeout, and response bodies are size-capped so
  one hostile feed cannot exhaust memory. The Ollama upstream is a LAN address
  supplied by config, never by a feed.

- **Extraction is an SSRF boundary.** A *feed* URL is typed by the operator, but
  an *item* URL is chosen by the publisher, so full-text extraction is a request
  rosso makes on a stranger's say-so. Before fetching, the host is resolved and
  every resulting address must be public: loopback, private, link-local
  (including the cloud metadata range), unique-local IPv6, carrier-NAT and
  IPv4-mapped forms are all refused, and only `http`/`https` are allowed.
  Resolving rather than pattern-matching the literal address is deliberate — a
  hostname pointing inward is the easier attack, and it is what a literal-only
  check misses. `ROSSO_EXTRACT_ALLOW_PRIVATE=1` disables the guard for a LAN-only
  setup; the integration harness sets it because wiremock serves from loopback.

- **Filesystem.** The only path built from external input is the SPA asset path,
  which is canonicalized and checked to stay under `STATIC_DIR` before any read.
  Feed content never reaches the filesystem.

- **SQL.** Every query is parameterized (`rusqlite params!`). The handful of
  statements that interpolate identifiers use hardcoded literals, never user
  input.

## Secrets

Secrets are injected at runtime via env, never baked into images or committed:
`SESSION_KEY` (from the 1Password `rosso` item via keel's sealed env). `.env`,
`*.db*` and data dirs are gitignored; `.env.example` holds placeholders only. The
container runs as UID 1000.

## Accepted risks

- **No per-user isolation.** Single-user by design — feeds, read state and the
  interest profile are global. Revisit if the app is ever shared.
- **The LLM sees article text.** Summaries and scores are produced by Ollama on
  the LAN mini. Nothing leaves the house, but any article rosso fetches is sent
  to that box. Revisit if the model host ever moves off-LAN.
- **No outbound host allowlist.** rosso fetches whatever public URL a subscribed
  feed points at, which is the same trust the owner extended by subscribing. The
  size caps, timeouts and the internal-address guard are the controls, not an
  allowlist.
- **DNS rebinding is not closed.** The extraction guard resolves the host and
  then `reqwest` resolves it again when it connects, so a record that changes in
  between would slip past. Closing it needs a connector pinned to the address
  already checked. Only `GET` is ever issued and the response is size-capped,
  which is what bounds the damage meanwhile. Revisit if rosso ever fetches on
  behalf of anyone but its single operator.

## Out of scope

Multi-tenant hardening, rate-limiting against a hostile internal user, and any
third-party sync API (Fever/Google Reader) — none of which exist, so none of them
are secured.

## Reporting

This is a personal project. Flag an issue privately to the maintainer rather than
opening a public issue with exploit detail.
