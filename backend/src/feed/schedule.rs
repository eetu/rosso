//! Per-feed poll cadence.
//!
//! A feed that posted something gets visited sooner; a feed that has been quiet
//! drifts toward the ceiling; a feed that is failing backs off hard. The point is
//! that a hundred subscriptions cost roughly what their actual publishing rate
//! costs, rather than a hundred fixed-rate polls.

use std::time::Duration;

use rand::RngExt;

pub const MIN_INTERVAL_S: u64 = 900; // 15 min
pub const DEFAULT_INTERVAL_S: u64 = 1800; // 30 min
pub const MAX_INTERVAL_S: u64 = 21_600; // 6 h

/// The ceiling on a publisher's *own* request, which is allowed past
/// `MAX_INTERVAL_S`.
///
/// Our ceiling exists so a quiet feed stops costing polls; it is our guess about
/// a feed nobody told us anything about. A `ttl` is the publisher telling us, and
/// clamping that down to six hours meant a feed asking for twelve was polled
/// twice as often as it asked — while the code claimed to never undercut a ttl.
/// This is only a guard against a `ttl` so large it silently retires the feed.
pub const MAX_PUBLISHER_INTERVAL_S: u64 = 7 * 24 * 3600; // 7 days

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollResult {
    /// The feed had entries we had not seen.
    NewItems,
    /// A 304, or a 200 whose entries we already had.
    Unchanged,
    Failed,
}

/// The next interval for a feed, in seconds.
///
/// `failures` is the count *after* this poll, so the first failure already backs
/// off. `ttl_minutes` is the publisher's own request and acts as a floor — we
/// never poll faster than a feed asks to be polled.
pub fn next_interval(
    current: u64,
    result: PollResult,
    failures: u32,
    ttl_minutes: Option<u32>,
) -> u64 {
    let base = match result {
        // Exponential, capped — a feed that is down stops costing anything.
        PollResult::Failed => {
            let shift = failures.clamp(1, 5) - 1;
            (DEFAULT_INTERVAL_S << shift).min(MAX_INTERVAL_S)
        }
        PollResult::NewItems => (current * 3 / 4).max(MIN_INTERVAL_S),
        PollResult::Unchanged => (current * 3 / 2).min(MAX_INTERVAL_S),
    };

    // Our own bounds first, then the publisher's floor — which may exceed them,
    // because it is the one number here that is not a guess.
    let ours = base.clamp(MIN_INTERVAL_S, MAX_INTERVAL_S);
    let floor = ttl_minutes.map_or(0, |m| u64::from(m) * 60);
    ours.max(floor).min(MAX_PUBLISHER_INTERVAL_S)
}

/// `base` ± up to 10%, so a batch of feeds added together does not stay in
/// lockstep and hammer their hosts on the same tick forever.
pub fn jitter(base: Duration) -> Duration {
    let secs = base.as_secs_f64();
    let factor = rand::rng().random_range(0.9..1.1);
    Duration::from_secs_f64(secs * factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_feed_is_visited_sooner_and_a_quiet_one_later() {
        assert_eq!(next_interval(1800, PollResult::NewItems, 0, None), 1350);
        assert_eq!(next_interval(1800, PollResult::Unchanged, 0, None), 2700);
    }

    #[test]
    fn intervals_stay_inside_the_bounds() {
        // Repeated success never drops below the floor…
        let mut interval = DEFAULT_INTERVAL_S;
        for _ in 0..20 {
            interval = next_interval(interval, PollResult::NewItems, 0, None);
        }
        assert_eq!(interval, MIN_INTERVAL_S);

        // …and repeated silence never exceeds the ceiling.
        let mut interval = DEFAULT_INTERVAL_S;
        for _ in 0..20 {
            interval = next_interval(interval, PollResult::Unchanged, 0, None);
        }
        assert_eq!(interval, MAX_INTERVAL_S);
    }

    #[test]
    fn failures_back_off_exponentially_then_flatten() {
        let at = |failures| next_interval(DEFAULT_INTERVAL_S, PollResult::Failed, failures, None);
        assert_eq!(at(1), 1800);
        assert_eq!(at(2), 3600);
        assert_eq!(at(3), 7200);
        assert_eq!(at(4), 14_400);
        assert_eq!(at(5), MAX_INTERVAL_S);
        assert_eq!(at(50), MAX_INTERVAL_S);
    }

    #[test]
    fn a_publishers_ttl_is_a_floor_we_do_not_undercut() {
        // 120 min > the 22.5 min a success would otherwise pick.
        assert_eq!(
            next_interval(1800, PollResult::NewItems, 0, Some(120)),
            7200
        );
        // A ttl *may* push us past our own ceiling: it is the publisher saying
        // so, not our guess about a feed that told us nothing. Clamping it to six
        // hours polled a feed asking for a week 28 times as often as it asked.
        assert_eq!(
            next_interval(1800, PollResult::NewItems, 0, Some(10_000)),
            600_000
        );
        // Only an absurd one is capped, and only so it cannot silently retire a
        // feed.
        assert_eq!(
            next_interval(1800, PollResult::NewItems, 0, Some(1_000_000)),
            MAX_PUBLISHER_INTERVAL_S
        );
    }

    #[test]
    fn jitter_stays_within_ten_percent() {
        let base = Duration::from_secs(600);
        for _ in 0..200 {
            let j = jitter(base).as_secs_f64();
            assert!((540.0..=660.0).contains(&j), "{j} out of range");
        }
    }
}
