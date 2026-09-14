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
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsPatch {
    pub interest_profile: Option<String>,
    pub score_threshold: Option<i64>,
    pub llm_model: Option<String>,
}

impl Settings {
    pub fn scoring_enabled(&self) -> bool {
        !self.interest_profile.trim().is_empty()
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
