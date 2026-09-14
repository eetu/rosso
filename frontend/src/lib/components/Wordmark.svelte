<span class="wordmark halo-wordmark">
  <!-- Family stroke language: currentColor outline, one warm accent dot. The
       square-ground copy for favicons/PWA icons lives in static/favicon.svg. -->
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
  </svg>
  <span class="riff">il buono, il brutto, il </span>rosso<span class="accent"
    >.</span
  >
</span>

<style>
  .wordmark {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1.05rem;
  }

  .glyph {
    width: 22px;
    height: 22px;
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
