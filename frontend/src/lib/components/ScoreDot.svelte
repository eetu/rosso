<script lang="ts">
  let { score, reason }: { score: number; reason?: string | null } = $props();

  // Three bands, not a gradient. The model's calibration is not fine enough to
  // justify distinguishing 71 from 74, and a continuous ramp would imply it is.
  const band = $derived(score >= 80 ? "high" : score >= 60 ? "mid" : "low");
</script>

<span
  class="dot {band}"
  title={reason ? `${score} — ${reason}` : String(score)}
  aria-label="score {score}"
></span>

<style>
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--halo-text-light);
  }

  .mid {
    background: var(--halo-accent);
    opacity: 0.45;
  }

  .high {
    background: var(--halo-accent);
  }
</style>
