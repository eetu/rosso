//! User-editable settings, stored as rows rather than environment variables
//! because they change from the UI while the app is running.
//!
//! Config supplies the defaults; a row overrides it. That split matters for the
//! model name in particular: the deploy sets a sensible one, and the user can
//! still switch to the bigger model for an evening without a redeploy.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::db::Db;
use crate::util::fingerprint;

pub const INTEREST_PROFILE: &str = "interest_profile";
pub const SCORE_THRESHOLD: &str = "score_threshold";
pub const LLM_MODEL: &str = "llm_model";
pub const DEDUPE_THRESHOLD: &str = "dedupe_threshold";

/// Cosine similarity at which two items are the same story. 0.90 is the plan's
/// starting point, to be tuned against real data rather than trusted.
///
/// It is a setting and not a constant because the cost of getting it wrong is
/// asymmetric: too high only leaves duplicates in the list, while too low hides
/// unrelated items behind each other. 1.0 or above turns clustering off, which
/// is the escape hatch if it ever starts hiding things it should not.
pub const DEFAULT_DEDUPE_THRESHOLD: f32 = 0.90;

/// Default cutoff for the `interesting` view. Deliberately not 50: the midpoint
/// of a 0-100 scale is "unremarkable", and a view of unremarkable things is the
/// pile this app exists to avoid.
pub const DEFAULT_SCORE_THRESHOLD: i64 = 65;

#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    /// Free text describing what the user wants to read. Empty means scoring is
    /// off — a number produced with nothing to judge against is noise, so items
    /// still get summarized but are left unscored.
    pub interest_profile: String,
    pub score_threshold: i64,
    pub llm_model: String,
    pub dedupe_threshold: f32,
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsPatch {
    pub interest_profile: Option<String>,
    pub score_threshold: Option<i64>,
    pub llm_model: Option<String>,
    pub dedupe_threshold: Option<f32>,
}

impl Settings {
    pub fn scoring_enabled(&self) -> bool {
        !self.interest_profile.trim().is_empty()
    }

    /// Below 1.0 there is some similarity that counts as a duplicate; at or above
    /// it, nothing can ever match and every item stays its own row.
    pub fn dedupe_enabled(&self) -> bool {
        self.dedupe_threshold < 1.0
    }

    /// Fingerprint of the profile an item was scored against. Stored per item so
    /// editing the profile makes every item a rescore candidate automatically —
    /// no invalidation pass, no queue to enqueue into.
    pub fn profile_fingerprint(&self) -> String {
        fingerprint(self.interest_profile.trim())
    }
}

pub async fn load(db: &Db, cfg: &Config) -> rusqlite::Result<Settings> {
    let default_model = cfg.llm_model.clone();
    db.with(move |c| {
        let get = |key: &str| -> rusqlite::Result<Option<String>> {
            let mut stmt = c.prepare("SELECT value FROM settings WHERE key = ?1")?;
            let mut rows = stmt.query([key])?;
            Ok(match rows.next()? {
                Some(row) => Some(row.get(0)?),
                None => None,
            })
        };
        Ok(Settings {
            interest_profile: get(INTEREST_PROFILE)?.unwrap_or_default(),
            score_threshold: get(SCORE_THRESHOLD)?
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_SCORE_THRESHOLD),
            llm_model: get(LLM_MODEL)?
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(default_model),
            dedupe_threshold: get(DEDUPE_THRESHOLD)?
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_DEDUPE_THRESHOLD),
        })
    })
    .await
}

pub async fn save(db: &Db, patch: SettingsPatch) -> rusqlite::Result<()> {
    db.with(move |c| {
        let mut put = c.prepare(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?;
        if let Some(profile) = patch.interest_profile {
            put.execute((INTEREST_PROFILE, profile))?;
        }
        if let Some(threshold) = patch.score_threshold {
            put.execute((SCORE_THRESHOLD, threshold.clamp(0, 100).to_string()))?;
        }
        if let Some(model) = patch.llm_model {
            put.execute((LLM_MODEL, model))?;
        }
        if let Some(threshold) = patch.dedupe_threshold {
            // Floored at 0.5 as well as capped: a low threshold would put
            // unrelated items in one cluster and collapse the list to a few
            // rows. 1.0 is reachable on purpose — it is how clustering is
            // turned off.
            put.execute((DEDUPE_THRESHOLD, threshold.clamp(0.5, 1.0).to_string()))?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(profile: &str) -> Settings {
        Settings {
            interest_profile: profile.into(),
            score_threshold: DEFAULT_SCORE_THRESHOLD,
            llm_model: "m".into(),
            dedupe_threshold: DEFAULT_DEDUPE_THRESHOLD,
        }
    }

    #[test]
    fn an_empty_profile_turns_scoring_off() {
        assert!(!settings("").scoring_enabled());
        assert!(!settings("   \n ").scoring_enabled());
        assert!(settings("rust, sqlite").scoring_enabled());
    }

    #[test]
    fn only_a_meaningful_edit_invalidates_existing_scores() {
        // Whitespace-only changes must not send every item back through the
        // model; a real edit must.
        assert_eq!(
            settings("rust, sqlite").profile_fingerprint(),
            settings("  rust, sqlite\n").profile_fingerprint()
        );
        assert_ne!(
            settings("rust, sqlite").profile_fingerprint(),
            settings("rust, sqlite, wasm").profile_fingerprint()
        );
    }
}
