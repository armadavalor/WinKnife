//! In-memory scan results, held between `start_scan` and `execute`.
//!
//! Why in memory: findings are facts about a filesystem that is changing under
//! us. Persisting them would invite the UI to act on a snapshot from last week.
//! They live for the app session, capped so a long-running window cannot grow
//! without bound, and the executor re-checks every path anyway.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};
use crate::core::types::{Finding, Recommendation};

/// How many completed scans stay available for review.
pub const MAX_RETAINED_SCANS: usize = 8;

/// What one scan produced. This is the payload the review screen renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    /// Id the UI passes back to `execute`.
    pub scan_id: Uuid,
    /// Module that produced it.
    pub module: String,
    /// Facts.
    pub findings: Vec<Finding>,
    /// Opinions, one per finding.
    pub recommendations: Vec<Recommendation>,
    /// Rules that could not run because an environment variable was unset,
    /// as `pack/rule: VARIABLE`. Surfaced so a partial scan says so.
    #[serde(default)]
    pub skipped_rules: Vec<String>,
}

impl ScanResult {
    /// Total bytes across recommendations in the green bucket.
    pub fn safe_bytes(&self) -> u64 {
        self.bytes_in(crate::core::types::RiskBucket::Safe)
    }

    /// Total bytes across recommendations in the yellow bucket.
    pub fn review_bytes(&self) -> u64 {
        self.bytes_in(crate::core::types::RiskBucket::NeedsReview)
    }

    fn bytes_in(&self, bucket: crate::core::types::RiskBucket) -> u64 {
        self.recommendations
            .iter()
            .filter(|rec| rec.risk == bucket)
            .map(|rec| rec.savings_bytes)
            .sum()
    }
}

/// Tauri-managed store of recent scans, newest last.
#[derive(Debug, Default)]
pub struct ScanStore {
    scans: Mutex<VecDeque<ScanResult>>,
}

impl ScanStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember a scan, evicting the oldest once the cap is reached.
    pub fn insert(&self, result: ScanResult) {
        let mut scans = self.lock();
        scans.push_back(result);
        while scans.len() > MAX_RETAINED_SCANS {
            scans.pop_front();
        }
    }

    /// Fetch a scan by id.
    ///
    /// # Errors
    /// Returns [`CoreError::UnknownScan`] when the id is not retained.
    pub fn get(&self, scan_id: Uuid) -> Result<ScanResult> {
        self.lock()
            .iter()
            .find(|scan| scan.scan_id == scan_id)
            .cloned()
            .ok_or_else(|| CoreError::UnknownScan(scan_id.to_string()))
    }

    /// How many scans are retained.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing is retained.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<ScanResult>> {
        self.scans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The cancellation tokens of the scans that are still running.
///
/// ARCHITECTURE section 3 lists `cancel_scan(scan_id)`, and phase 1 shipped the
/// token without a way to reach it: the cleaner's walk checked a token nothing
/// could trip. The duplicate finder makes that visible — a full-hash pass over a
/// Pictures folder is minutes of work, and a user who started it by accident
/// needs a stop button that does something.
///
/// A token is registered when a scan starts and removed when it finishes, so
/// cancelling a scan that already ended is a no-op rather than an error.
#[derive(Debug, Default)]
pub struct CancelRegistry {
    tokens: Mutex<std::collections::HashMap<Uuid, crate::core::scanner::CancellationToken>>,
}

impl CancelRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh token for `scan_id` and hand back a clone.
    pub fn start(&self, scan_id: Uuid) -> crate::core::scanner::CancellationToken {
        let token = crate::core::scanner::CancellationToken::new();
        self.lock().insert(scan_id, token.clone());
        token
    }

    /// Forget a scan that has finished.
    pub fn finish(&self, scan_id: Uuid) {
        self.lock().remove(&scan_id);
    }

    /// Ask a running scan to stop. `false` when it is already over.
    pub fn cancel(&self, scan_id: Uuid) -> bool {
        match self.lock().get(&scan_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// How many scans are running.
    pub fn running(&self) -> usize {
        self.lock().len()
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<
        '_,
        std::collections::HashMap<Uuid, crate::core::scanner::CancellationToken>,
    > {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Action, FindingKind, Justification, RiskBucket};

    fn result(module: &str) -> ScanResult {
        let finding = Finding::new(module, "C:\\Temp\\a.tmp", FindingKind::File);
        let rec = Recommendation::new(
            vec![finding.id],
            RiskBucket::Safe,
            90,
            Action::Quarantine,
            "temp file",
            100,
            Justification::Rule {
                pack_id: "windows-temp".to_owned(),
                rule_id: "temp-old".to_owned(),
            },
        )
        .expect("recommendation");
        ScanResult {
            scan_id: Uuid::new_v4(),
            module: module.to_owned(),
            findings: vec![finding],
            recommendations: vec![rec],
            skipped_rules: Vec::new(),
        }
    }

    #[test]
    fn a_stored_scan_can_be_fetched_by_id() {
        let store = ScanStore::new();
        let scan = result("cleaner");
        let id = scan.scan_id;
        store.insert(scan.clone());

        assert_eq!(store.get(id).expect("fetch"), scan);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn an_unknown_scan_id_is_an_error() {
        let store = ScanStore::new();
        assert!(store.is_empty());
        let err = store.get(Uuid::new_v4()).expect_err("unknown id");
        assert!(matches!(err, CoreError::UnknownScan(_)));
    }

    #[test]
    fn the_store_evicts_the_oldest_scan_past_the_cap() {
        let store = ScanStore::new();
        let first = result("cleaner");
        let first_id = first.scan_id;
        store.insert(first);
        for _ in 0..MAX_RETAINED_SCANS {
            store.insert(result("cleaner"));
        }

        assert_eq!(store.len(), MAX_RETAINED_SCANS);
        assert!(
            store.get(first_id).is_err(),
            "the oldest scan should have been evicted"
        );
    }

    #[test]
    fn bucket_totals_are_summed_per_bucket() {
        let finding = Finding::new("cleaner", "C:\\a", FindingKind::File);
        let finding_id = finding.id;
        let make = |risk, bytes| {
            Recommendation::new(
                vec![finding_id],
                risk,
                50,
                if risk == RiskBucket::DoNotDelete {
                    Action::InfoOnly
                } else {
                    Action::Quarantine
                },
                "because",
                bytes,
                Justification::Rule {
                    pack_id: "windows-temp".to_owned(),
                    rule_id: "temp-old".to_owned(),
                },
            )
            .expect("recommendation")
        };
        let scan = ScanResult {
            scan_id: Uuid::new_v4(),
            module: "cleaner".to_owned(),
            findings: vec![finding],
            recommendations: vec![
                make(RiskBucket::Safe, 100),
                make(RiskBucket::Safe, 50),
                make(RiskBucket::NeedsReview, 700),
                make(RiskBucket::DoNotDelete, 9_000),
            ],
            skipped_rules: Vec::new(),
        };

        assert_eq!(scan.safe_bytes(), 150);
        assert_eq!(scan.review_bytes(), 700);
    }

    #[test]
    fn a_running_scan_can_be_cancelled_and_a_finished_one_cannot() {
        let registry = CancelRegistry::new();
        let scan_id = Uuid::new_v4();

        assert!(
            !registry.cancel(scan_id),
            "cancelling a scan that never started is a no-op, not an error"
        );

        let token = registry.start(scan_id);
        assert_eq!(registry.running(), 1);
        assert!(!token.is_cancelled());

        assert!(registry.cancel(scan_id));
        assert!(
            token.is_cancelled(),
            "the token the scan is holding must be the one that gets tripped"
        );

        registry.finish(scan_id);
        assert_eq!(registry.running(), 0);
        assert!(!registry.cancel(scan_id));
    }

    #[test]
    fn two_scans_have_two_independent_tokens() {
        let registry = CancelRegistry::new();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let a = registry.start(first);
        let b = registry.start(second);

        registry.cancel(first);
        assert!(a.is_cancelled());
        assert!(!b.is_cancelled(), "one cancel must not stop the other scan");
    }

    #[test]
    fn scan_results_serialise_in_camel_case() {
        let json = serde_json::to_value(result("cleaner")).expect("serialize");
        assert!(json.get("scanId").is_some(), "got {json}");
        assert!(json.get("skippedRules").is_some(), "got {json}");
    }
}
