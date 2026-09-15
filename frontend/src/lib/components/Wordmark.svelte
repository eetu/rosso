<!-- The rosso wordmark. Full riff on wide screens, collapses to `rosso.` on
     narrow. Convention: halo-design's short, dry pop-culture riff ending in the
     app name, with exactly one accent element.

     Plain inline flow, like every other app in the family. An inline-flex
     wrapper with a `gap` looks equivalent but is not: the gap falls between
     every flex child, so it lands between the name and its accent dot too and
     the mark reads `rosso .` — the one place in the wordmark where spacing is
     not a free choice. -->
<span class="halo-wordmark wordmark">
  <svg class="glyph" viewBox="0 0 64 64" fill="none" aria-hidden="true">
    <path
      d="M18 16 A30 30 0 0 1 48 46"
      stroke="currentColor"
      stroke-width="3"
      stroke-linecap="round"
    />
    <path
      d="M18 30 A16 16 0 0 1 34 46"
      stroke="currentColor"
      stroke-width="2.5"
      stroke-linecap="round"
    />
    <circle cx="18" cy="46" r="5" fill="#f78f08" />
  </svg><span class="riff">il buono, il brutto, il </span>rosso<span
    class="accent">.</span
  >
</span>

<style>
  .wordmark {
    font-size: 1.05rem;
    user-select: none;
  }

  .glyph {
    width: 1.2em;
    height: 1.2em;
    /* Sits on the text baseline rather than being laid out beside it. */
    vertical-align: -0.28em;
    margin-right: 0.4em;
    color: var(--halo-text-main);
  }

  .riff {
    font-weight: 400;
    color: var(--halo-text-muted);
  }

  /* The riff is the joke, not the name — drop it before it wraps. */
  @media (max-width: 640px) {
    .riff {
      display: none;
    }
  }
</style>
