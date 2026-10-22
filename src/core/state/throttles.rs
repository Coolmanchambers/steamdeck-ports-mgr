
use super::{now_timestamp, StateManager};
use chrono::{DateTime, Duration, Utc};

// Fork (upstream bug O7): 12 h like the catalog, so an app left open re-checks twice a day (upstream: 24 h).
const LAUNCHER_UPDATE_CHECK_INTERVAL_HOURS: i64 = 12;
const PORT_UPDATE_STALE_HOURS: i64 = 24;
const CATALOG_CHECK_INTERVAL_HOURS: i64 = 12;
const THEMES_CHECK_INTERVAL_HOURS: i64 = 12;

fn is_older_than(timestamp: &str, hours: i64) -> bool {
    DateTime::parse_from_rfc3339(timestamp).map_or(true, |last| Utc::now().signed_duration_since(last) >= Duration::hours(hours))
}

// The stamp that makes `is_older_than(stamp, interval_hours)` turn true `retry_in` from now.
fn deferred_check_stamp(interval_hours: i64, retry_in: std::time::Duration) -> String {
    let retry_in = Duration::from_std(retry_in).unwrap_or_else(|_| Duration::hours(interval_hours));
    (Utc::now() - Duration::hours(interval_hours) + retry_in).to_rfc3339()
}

pub fn is_stale_for_update_check(installed_at: &str) -> bool {
    is_older_than(installed_at, PORT_UPDATE_STALE_HOURS)
}

impl StateManager {
    pub fn should_check_launcher_update(&self) -> bool {
        self.release_sync
            && !self.launcher_update_available
            && !self.last_launcher_update_check.is_empty()
            && is_older_than(&self.last_launcher_update_check, LAUNCHER_UPDATE_CHECK_INTERVAL_HOURS)
    }

    pub fn mark_launcher_update_check(&mut self) {
        self.last_launcher_update_check = now_timestamp();
        self.save();
    }

    pub fn set_launcher_update_available(&mut self, value: bool) {
        self.launcher_update_available = value;
        self.save();
    }

    pub fn set_release_sync(&mut self, value: bool) {
        self.release_sync = value;
        self.save();
    }

    pub fn should_check_catalog(&self) -> bool {
        self.catalog_sync && is_older_than(&self.last_catalog_check, CATALOG_CHECK_INTERVAL_HOURS)
    }

    pub fn mark_catalog_check(&mut self, etag: String) {
        self.last_catalog_check = now_timestamp();
        self.last_catalog_etag = etag;
        self.save();
    }

    /// Fork (upstream bug O6): after a failed check, make the next one due in `retry_in` instead of at the
    /// next launch. The saved ETag is kept. Stored by back-dating the last check, so state.json keeps its fields.
    pub fn defer_catalog_check(&mut self, retry_in: std::time::Duration) {
        self.last_catalog_check = deferred_check_stamp(CATALOG_CHECK_INTERVAL_HOURS, retry_in);
        self.save();
    }

    pub fn defer_themes_check(&mut self, retry_in: std::time::Duration) {
        self.last_themes_check = deferred_check_stamp(THEMES_CHECK_INTERVAL_HOURS, retry_in);
        self.save();
    }

    pub fn should_check_themes(&self) -> bool {
        self.catalog_sync && is_older_than(&self.last_themes_check, THEMES_CHECK_INTERVAL_HOURS)
    }

    pub fn mark_themes_check(&mut self, etag: String) {
        self.last_themes_check = now_timestamp();
        self.last_themes_etag = etag;
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_state_path;
    use super::*;
    use std::fs;

    #[test]
    fn launcher_jamais_verifie_ne_declenche_pas() {
        let path = test_state_path("throttle_never");
        assert!(!StateManager::load(&path).should_check_launcher_update());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn launcher_verifie_recemment_ne_declenche_pas() {
        let path = test_state_path("throttle_recent");
        let mut state = StateManager::load(&path);
        state.mark_launcher_update_check();
        assert!(!state.should_check_launcher_update());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn launcher_verifie_il_y_a_longtemps_declenche() {
        let path = test_state_path("throttle_old");
        let mut state = StateManager::load(&path);
        state.last_launcher_update_check = (Utc::now() - Duration::hours(LAUNCHER_UPDATE_CHECK_INTERVAL_HOURS + 1)).to_rfc3339();
        assert!(state.should_check_launcher_update());

        state.set_release_sync(false);
        assert!(!state.should_check_launcher_update());
        state.set_release_sync(true);
        state.set_launcher_update_available(true);
        assert!(!state.should_check_launcher_update());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn launcher_update_available_persiste() {
        let path = test_state_path("launcher_update_available_roundtrip");
        StateManager::load(&path).set_launcher_update_available(true);
        assert!(StateManager::load(&path).launcher_update_available);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn catalogue_jamais_verifie_declenche() {
        let path = test_state_path("catalog_throttle_never");
        assert!(StateManager::load(&path).should_check_catalog());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn catalogue_verifie_recemment_ne_declenche_pas() {
        let path = test_state_path("catalog_throttle_recent");
        let mut state = StateManager::load(&path);
        state.mark_catalog_check("\"abc123\"".to_string());
        assert!(!state.should_check_catalog());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn catalogue_ancien_declenche_sauf_si_desactive() {
        let path = test_state_path("catalog_throttle_old");
        let mut state = StateManager::load(&path);
        state.last_catalog_check = (Utc::now() - Duration::hours(25)).to_rfc3339();
        assert!(state.should_check_catalog());
        state.catalog_sync = false;
        assert!(!state.should_check_catalog());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn release_sync_vrai_par_defaut_et_persiste() {
        let path = test_state_path("release_sync_default");
        let mut state = StateManager::load(&path);
        assert!(state.release_sync);
        state.set_release_sync(false);
        assert!(!StateManager::load(&path).release_sync);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn mark_catalog_check_persiste_l_etag() {
        let path = test_state_path("catalog_etag_roundtrip");
        StateManager::load(&path).mark_catalog_check("\"abc123\"".to_string());
        let reloaded = StateManager::load(&path);
        assert_eq!(reloaded.last_catalog_etag, "\"abc123\"");
        assert!(!reloaded.last_catalog_check.is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_failed_catalog_or_themes_check_is_retried_after_the_backoff_not_at_once() {
        let path = test_state_path("catalog_backoff");
        let mut state = StateManager::load(&path);
        state.last_catalog_etag = "\"e1\"".to_string();
        assert!(state.should_check_catalog() && state.should_check_themes());
        state.defer_catalog_check(std::time::Duration::from_secs(3600));
        state.defer_themes_check(std::time::Duration::from_secs(3600));
        let reloaded = StateManager::load(&path);
        assert!(!reloaded.should_check_catalog() && !reloaded.should_check_themes());
        assert_eq!(reloaded.last_catalog_etag, "\"e1\"", "a failure keeps the ETag");
        state.defer_catalog_check(std::time::Duration::ZERO);
        assert!(state.should_check_catalog(), "a zero backoff is due at once");
        state.defer_catalog_check(std::time::Duration::from_secs(20 * 3600));
        assert!(!state.should_check_catalog(), "a backoff longer than the interval works too");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn is_stale_for_update_check_vide_ou_ancien() {
        assert!(is_stale_for_update_check(""));
        assert!(is_stale_for_update_check(&(Utc::now() - Duration::hours(25)).to_rfc3339()));
        assert!(!is_stale_for_update_check(&(Utc::now() - Duration::hours(1)).to_rfc3339()));
    }
}
