//! Shared data model exchanged between modules, the brain and the UI.
//!
//! Why: every module speaks the same three nouns — a [`Finding`] is something
//! that exists on the machine, a [`Recommendation`] is what we suggest doing
//! about it, an [`ExecReport`] is what actually happened. The brain scores all
//! modules' findings in one risk language (ARCHITECTURE section 2).
//!
//! These structs are mirrored by hand in `src/lib/types.ts`. Change one,
//! change the other in the same commit (CONVENTIONS.md).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};

/// Confidence is expressed on a 0-100 scale.
pub const MAX_CONFIDENCE: u8 = 100;

/// What kind of thing a [`Finding`] points at.
///
/// Not every finding is a plain file: the recycle bin is queried through the
/// shell API, the startup manager reads registry values and scheduled tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FindingKind {
    /// A single file on disk.
    File,
    /// A directory, sized recursively.
    Directory,
    /// An entry inside the Windows recycle bin.
    RecycleBinItem,
    /// A registry value (for example a `Run` key entry).
    RegistryEntry,
    /// A Windows Task Scheduler entry.
    ScheduledTask,
    /// Anything a module needs that does not fit above.
    Other,
}

/// Something a module found while scanning. Findings are facts, never opinions
/// — the opinion lives in [`Recommendation`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable id for this finding within a scan.
    pub id: Uuid,
    /// Id of the module that produced it.
    pub module: String,
    /// Absolute path (or registry path) the finding refers to.
    pub path: PathBuf,
    /// What the path points at.
    pub kind: FindingKind,
    /// Size on disk in bytes; `0` when the concept does not apply.
    pub size_bytes: u64,
    /// Creation timestamp, when the filesystem exposes one.
    pub created: Option<DateTime<Utc>>,
    /// Last modification timestamp.
    pub modified: Option<DateTime<Utc>>,
    /// Last access timestamp (may be disabled system-wide on Windows).
    pub accessed: Option<DateTime<Utc>>,
    /// Module-specific extras, e.g. the id of the rule that matched.
    pub meta: serde_json::Value,
}

impl Finding {
    /// Create a finding with a fresh id, no timestamps and empty metadata.
    pub fn new(module: impl Into<String>, path: impl Into<PathBuf>, kind: FindingKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            module: module.into(),
            path: path.into(),
            kind,
            size_bytes: 0,
            created: None,
            modified: None,
            accessed: None,
            meta: serde_json::Value::Null,
        }
    }
}

/// The three risk buckets the whole app speaks in (UI: green / yellow / red).
///
/// Protected paths can never be rated [`RiskBucket::Safe`]; that gate lives in
/// the brain and is enforced in code, not in rule data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskBucket {
    /// Green. Removing this is uncontroversial; one-click clean.
    Safe,
    /// Yellow. A human has to decide, item by item.
    NeedsReview,
    /// Red. Informational only — the UI shows no action button.
    DoNotDelete,
}

impl RiskBucket {
    /// Whether the UI is allowed to offer an action for this bucket.
    pub fn is_actionable(self) -> bool {
        !matches!(self, Self::DoNotDelete)
    }
}

/// What we propose doing with the findings of a recommendation.
///
/// Deliberately has no raw `Delete`: removal always means "move to quarantine",
/// which is reversible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Action {
    /// Move to the quarantine directory; restorable for the retention window.
    Quarantine,
    /// Disable (not remove) an autostart entry.
    DisableStartupEntry,
    /// Explicitly leave it alone; remembered so we stop re-suggesting it.
    Ignore,
    /// Nothing to do — shown for information only.
    InfoOnly,
}

impl Action {
    /// Whether applying this action changes the system.
    pub fn is_mutating(self) -> bool {
        matches!(self, Self::Quarantine | Self::DisableStartupEntry)
    }
}

/// A suggestion about one or more findings. Always carries a `reason` — no
/// unexplained suggestions (CLAUDE.md hard rules).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    /// Stable id, used as the approval token in `execute`.
    pub id: Uuid,
    /// Findings this recommendation covers.
    pub finding_ids: Vec<Uuid>,
    /// Risk bucket assigned by the brain.
    pub risk: RiskBucket,
    /// How sure the brain is, 0-100.
    pub confidence: u8,
    /// What we propose doing.
    pub action: Action,
    /// Human-readable justification shown next to every item in the UI.
    pub reason: String,
    /// Bytes reclaimed if the user approves.
    pub savings_bytes: u64,
}

impl Recommendation {
    /// Build a validated recommendation.
    ///
    /// # Errors
    /// Returns [`CoreError::InvalidRecommendation`] when the reason is blank,
    /// the confidence is out of range, no findings are referenced, or an
    /// action is proposed for a [`RiskBucket::DoNotDelete`] item.
    pub fn new(
        finding_ids: Vec<Uuid>,
        risk: RiskBucket,
        confidence: u8,
        action: Action,
        reason: impl Into<String>,
        savings_bytes: u64,
    ) -> Result<Self> {
        let candidate = Self {
            id: Uuid::new_v4(),
            finding_ids,
            risk,
            confidence,
            action,
            reason: reason.into(),
            savings_bytes,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Re-check the invariants. The executor calls this before touching
    /// anything, so a hand-built recommendation cannot smuggle a mutating
    /// action past the red bucket.
    ///
    /// # Errors
    /// See [`Recommendation::new`].
    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: String| Err(CoreError::InvalidRecommendation(msg));

        if self.reason.trim().is_empty() {
            return invalid("reason must not be empty".to_owned());
        }
        if self.confidence > MAX_CONFIDENCE {
            return invalid(format!(
                "confidence {} is above {MAX_CONFIDENCE}",
                self.confidence
            ));
        }
        if self.finding_ids.is_empty() {
            return invalid("must reference at least one finding".to_owned());
        }
        if !self.risk.is_actionable() && self.action.is_mutating() {
            return invalid(format!(
                "action {:?} is not allowed for a {:?} item",
                self.action, self.risk
            ));
        }
        Ok(())
    }
}

/// Outcome of applying one recommendation to one finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ExecOutcome {
    /// Applied successfully.
    Succeeded,
    /// Deliberately not applied (locked file, permission denied, user filter).
    Skipped {
        /// Why it was skipped, shown per item in the post-run report.
        reason: String,
    },
    /// Attempted and failed.
    Failed {
        /// The error, already flattened to a string for the UI.
        error: String,
    },
}

/// Per-item result inside an [`ExecReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecItemResult {
    /// The recommendation that produced this item.
    pub recommendation_id: Uuid,
    /// The finding that was acted on.
    pub finding_id: Uuid,
    /// Path acted on, echoed for the log and the UI.
    pub path: PathBuf,
    /// What happened.
    pub outcome: ExecOutcome,
    /// Bytes reclaimed by this item (`0` unless it succeeded).
    pub reclaimed_bytes: u64,
}

/// What an `execute` run actually did. Failures never abort the batch — every
/// item reports for itself.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecReport {
    /// Quarantine batch the moved items landed in, when anything was moved.
    pub batch_id: Option<Uuid>,
    /// One entry per attempted item.
    pub items: Vec<ExecItemResult>,
    /// Sum of `reclaimed_bytes` over successful items.
    pub reclaimed_bytes: u64,
}

impl ExecReport {
    /// Build a report from its items, summing the reclaimed bytes.
    pub fn from_items(batch_id: Option<Uuid>, items: Vec<ExecItemResult>) -> Self {
        let reclaimed_bytes = items
            .iter()
            .filter(|item| item.outcome == ExecOutcome::Succeeded)
            .map(|item| item.reclaimed_bytes)
            .sum();
        Self {
            batch_id,
            items,
            reclaimed_bytes,
        }
    }

    /// Number of items that succeeded.
    pub fn succeeded(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == ExecOutcome::Succeeded)
            .count()
    }

    /// Number of items that failed.
    pub fn failed(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item.outcome, ExecOutcome::Failed { .. }))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding_ids() -> Vec<Uuid> {
        vec![Uuid::nil()]
    }

    #[test]
    fn risk_bucket_wire_format_matches_typescript() {
        let json = serde_json::to_string(&[
            RiskBucket::Safe,
            RiskBucket::NeedsReview,
            RiskBucket::DoNotDelete,
        ])
        .expect("serialize");
        assert_eq!(json, r#"["safe","needsReview","doNotDelete"]"#);
    }

    #[test]
    fn action_wire_format_matches_typescript() {
        let json = serde_json::to_string(&[
            Action::Quarantine,
            Action::DisableStartupEntry,
            Action::Ignore,
            Action::InfoOnly,
        ])
        .expect("serialize");
        assert_eq!(
            json,
            r#"["quarantine","disableStartupEntry","ignore","infoOnly"]"#
        );
    }

    #[test]
    fn finding_kind_wire_format_matches_typescript() {
        let json = serde_json::to_string(&FindingKind::RecycleBinItem).expect("serialize");
        assert_eq!(json, r#""recycleBinItem""#);
    }

    #[test]
    fn finding_uses_camel_case_field_names() {
        let finding = Finding::new("dummy", "C:\\Temp\\a.log", FindingKind::File);
        let json = serde_json::to_value(&finding).expect("serialize");
        assert!(json.get("sizeBytes").is_some(), "got {json}");
        assert!(json.get("size_bytes").is_none(), "got {json}");
    }

    #[test]
    fn only_do_not_delete_blocks_actions() {
        assert!(RiskBucket::Safe.is_actionable());
        assert!(RiskBucket::NeedsReview.is_actionable());
        assert!(!RiskBucket::DoNotDelete.is_actionable());
    }

    #[test]
    fn recommendation_requires_a_reason() {
        let err = Recommendation::new(
            finding_ids(),
            RiskBucket::Safe,
            90,
            Action::Quarantine,
            "   ",
            10,
        )
        .expect_err("blank reason must be rejected");
        assert!(matches!(err, CoreError::InvalidRecommendation(_)));
    }

    #[test]
    fn recommendation_rejects_confidence_above_100() {
        let err = Recommendation::new(
            finding_ids(),
            RiskBucket::Safe,
            101,
            Action::Quarantine,
            "temp file",
            10,
        )
        .expect_err("confidence above 100 must be rejected");
        assert!(matches!(err, CoreError::InvalidRecommendation(_)));
    }

    #[test]
    fn recommendation_rejects_empty_finding_list() {
        let err = Recommendation::new(
            Vec::new(),
            RiskBucket::Safe,
            90,
            Action::Quarantine,
            "temp file",
            10,
        )
        .expect_err("a recommendation without findings must be rejected");
        assert!(matches!(err, CoreError::InvalidRecommendation(_)));
    }

    #[test]
    fn do_not_delete_may_not_carry_a_mutating_action() {
        for action in [Action::Quarantine, Action::DisableStartupEntry] {
            let err = Recommendation::new(
                finding_ids(),
                RiskBucket::DoNotDelete,
                100,
                action,
                "inside C:\\Windows",
                0,
            )
            .expect_err("red bucket must never carry a mutating action");
            assert!(matches!(err, CoreError::InvalidRecommendation(_)));
        }
    }

    #[test]
    fn do_not_delete_may_carry_info_only() {
        let rec = Recommendation::new(
            finding_ids(),
            RiskBucket::DoNotDelete,
            100,
            Action::InfoOnly,
            "protected system path",
            0,
        )
        .expect("info-only is allowed for the red bucket");
        assert_eq!(rec.action, Action::InfoOnly);
    }

    #[test]
    fn valid_recommendation_round_trips_through_json() {
        let rec = Recommendation::new(
            finding_ids(),
            RiskBucket::NeedsReview,
            60,
            Action::Quarantine,
            "not opened in 11 months",
            2_048,
        )
        .expect("valid recommendation");
        let json = serde_json::to_string(&rec).expect("serialize");
        let back: Recommendation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(rec, back);
        assert!(json.contains(r#""savingsBytes":2048"#), "got {json}");
    }

    #[test]
    fn exec_report_sums_only_successful_items() {
        let item = |outcome: ExecOutcome, bytes: u64| ExecItemResult {
            recommendation_id: Uuid::nil(),
            finding_id: Uuid::nil(),
            path: PathBuf::from("C:\\Temp\\a.log"),
            outcome,
            reclaimed_bytes: bytes,
        };
        let report = ExecReport::from_items(
            Some(Uuid::nil()),
            vec![
                item(ExecOutcome::Succeeded, 100),
                item(
                    ExecOutcome::Failed {
                        error: "locked".to_owned(),
                    },
                    900,
                ),
                item(
                    ExecOutcome::Skipped {
                        reason: "in use".to_owned(),
                    },
                    50,
                ),
                item(ExecOutcome::Succeeded, 23),
            ],
        );
        assert_eq!(report.reclaimed_bytes, 123);
        assert_eq!(report.succeeded(), 2);
        assert_eq!(report.failed(), 1);
    }

    #[test]
    fn exec_outcome_is_a_tagged_union_for_typescript() {
        let json = serde_json::to_string(&ExecOutcome::Skipped {
            reason: "locked".to_owned(),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"status":"skipped","reason":"locked"}"#);
    }
}
