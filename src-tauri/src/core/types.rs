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

/// Where an autostart entry lives, which is also how it is disabled.
///
/// In `core` rather than in `modules::startup` because [`Justification`] names
/// it and the executor has to re-derive it — the same reason [`FindingKind`] has
/// `RegistryEntry` and `ScheduledTask` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum StartupSource {
    /// A value under a `Run` or `RunOnce` key in `HKEY_CURRENT_USER`.
    CurrentUserRun,
    /// The same, in `HKEY_LOCAL_MACHINE`.
    LocalMachineRun,
    /// The same, under `WOW6432Node` — the 32-bit view of `HKLM`.
    LocalMachineRun32,
    /// A shortcut in this user's Startup folder.
    UserStartupFolder,
    /// A shortcut in the all-users Startup folder.
    CommonStartupFolder,
    /// A scheduled task with a logon trigger.
    ScheduledTask,
}

/// **Why** a recommendation proposes what it proposes, in a form the executor
/// can re-derive from the machine at execute time.
///
/// # Why this is part of the contract rather than a convention
///
/// The executor's fourth gate re-checks that a recommendation is still
/// *warranted*, not merely still permitted (`core::executor`). Until phase 3 it
/// did that by sniffing a finding's free-form `meta` for shapes it recognised,
/// so a module that recorded nothing fell through to the first three gates and
/// its actions were never re-verified. Every shipped module happened to record
/// something, so nothing reachable was unchecked — but "happened to" is not a
/// guarantee, and the fix the architect asked for is structural: a mutating
/// action **cannot be constructed** without a justification gate 4 understands
/// ([`Recommendation::validate`]).
///
/// [`Justification::Informational`] is the escape hatch and is deliberately
/// useless: it is only valid on a non-mutating action, so choosing it does not
/// let a module act without being checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum Justification {
    /// Nothing is proposed, so there is nothing to re-derive. Rejected on any
    /// mutating action.
    Informational,
    /// A rule pack's rule matched. The executor re-loads the rule and re-tests
    /// the criteria that can have changed since the scan.
    #[serde(rename_all = "camelCase")]
    Rule {
        /// Pack that matched.
        pack_id: String,
        /// Rule inside it.
        rule_id: String,
    },
    /// The file is a redundant copy. The executor re-hashes it and checks the
    /// keeper is still there.
    #[serde(rename_all = "camelCase")]
    DuplicateOf {
        /// blake3 of the contents when the set was formed.
        content_hash: String,
        /// Whether this copy is the one the set nominated to keep.
        keeper: bool,
        /// The copy this one was declared identical to.
        keeper_path: Option<PathBuf>,
    },
    /// An autostart entry. The executor re-reads it from its source and refuses
    /// if it has moved, vanished, or now points at a different program.
    #[serde(rename_all = "camelCase")]
    StartupEntry {
        /// Where it lives, which is also how it gets disabled.
        source: StartupSource,
        /// Value name, shortcut file name or task path — whatever identifies it
        /// inside that source.
        key: String,
        /// The command line as the scan read it. A different one now means the
        /// entry was replaced while the review screen was open.
        command: String,
    },
}

impl Justification {
    /// Whether this justification is something gate 4 can actually re-derive.
    ///
    /// Only [`Justification::Informational`] is not, which is why it may never
    /// accompany a mutating action.
    pub fn is_verifiable(&self) -> bool {
        !matches!(self, Self::Informational)
    }
}

/// A suggestion about one or more findings. Always carries a `reason` — no
/// unexplained suggestions (CLAUDE.md hard rules) — and, when it proposes to
/// change anything, a [`Justification`] the executor can re-derive.
///
/// # Every construction site validates
///
/// [`Recommendation::new`] validates, and so does the executor's first gate. But
/// the fields are public and the type derived `Deserialize`, so a value arriving
/// over IPC or out of the `actions` table reached existence without either — the
/// invariant held only because the executor happened to re-check it
/// (security review L15). It is enforced at construction now: deserialisation
/// goes through [`RecommendationWire`] and [`Recommendation::validate`], so
/// there is no path to an invalid value at all. The executor's re-check stays,
/// because the point of gate 1 is to re-derive rather than to trust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(try_from = "RecommendationWire")]
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
    /// The machine-checkable half of `reason`, re-derived by the executor's
    /// fourth gate before anything happens.
    pub justification: Justification,
}

impl Recommendation {
    /// Build a validated recommendation.
    ///
    /// # Errors
    /// Returns [`CoreError::InvalidRecommendation`] when the reason is blank,
    /// the confidence is out of range, no findings are referenced, an action is
    /// proposed for a [`RiskBucket::DoNotDelete`] item, or a mutating action
    /// carries a justification the executor cannot re-derive.
    pub fn new(
        finding_ids: Vec<Uuid>,
        risk: RiskBucket,
        confidence: u8,
        action: Action,
        reason: impl Into<String>,
        savings_bytes: u64,
        justification: Justification,
    ) -> Result<Self> {
        let candidate = Self {
            id: Uuid::new_v4(),
            finding_ids,
            risk,
            confidence,
            action,
            reason: reason.into(),
            savings_bytes,
            justification,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// A recommendation that proposes nothing, for the red bucket and for rows
    /// that exist only to tell the user something.
    ///
    /// # Errors
    /// See [`Recommendation::new`].
    pub fn informational(
        finding_ids: Vec<Uuid>,
        risk: RiskBucket,
        confidence: u8,
        reason: impl Into<String>,
        savings_bytes: u64,
    ) -> Result<Self> {
        Self::new(
            finding_ids,
            risk,
            confidence,
            Action::InfoOnly,
            reason,
            savings_bytes,
            Justification::Informational,
        )
    }

    /// Re-check the invariants. The executor calls this before touching
    /// anything, so a hand-built recommendation cannot smuggle a mutating
    /// action past the red bucket — or past gate 4.
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
        // The structural half of the executor's fourth gate: something that
        // changes the machine has to arrive with a reason the executor can check
        // against the machine, or it cannot be built at all.
        if self.action.is_mutating() && !self.justification.is_verifiable() {
            return invalid(format!(
                "action {:?} needs a justification the executor can re-verify, \
                 and `Informational` is not one",
                self.action
            ));
        }
        Ok(())
    }
}

/// The shape a [`Recommendation`] arrives in before it has been checked.
///
/// It exists only so `#[serde(try_from)]` has somewhere to land: the fields are
/// identical, and the conversion is [`Recommendation::validate`]. Keeping it
/// private means nothing outside this module can hold an unvalidated one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecommendationWire {
    id: Uuid,
    finding_ids: Vec<Uuid>,
    risk: RiskBucket,
    confidence: u8,
    action: Action,
    reason: String,
    savings_bytes: u64,
    justification: Justification,
}

impl TryFrom<RecommendationWire> for Recommendation {
    type Error = CoreError;

    fn try_from(wire: RecommendationWire) -> Result<Self> {
        let candidate = Self {
            id: wire.id,
            finding_ids: wire.finding_ids,
            risk: wire.risk,
            confidence: wire.confidence,
            action: wire.action,
            reason: wire.reason,
            savings_bytes: wire.savings_bytes,
            justification: wire.justification,
        };
        candidate.validate()?;
        Ok(candidate)
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

    /// A justification gate 4 can re-derive, for the tests that are about
    /// something else.
    fn a_rule() -> Justification {
        Justification::Rule {
            pack_id: "windows-temp".to_owned(),
            rule_id: "temp-old".to_owned(),
        }
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
            a_rule(),
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
            a_rule(),
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
            a_rule(),
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
                a_rule(),
            )
            .expect_err("red bucket must never carry a mutating action");
            assert!(matches!(err, CoreError::InvalidRecommendation(_)));
        }
    }

    #[test]
    fn do_not_delete_may_carry_info_only() {
        let rec = Recommendation::informational(
            finding_ids(),
            RiskBucket::DoNotDelete,
            100,
            "protected system path",
            0,
        )
        .expect("info-only is allowed for the red bucket");
        assert_eq!(rec.action, Action::InfoOnly);
        assert_eq!(rec.justification, Justification::Informational);
    }

    /// **Architect addition A.** A module cannot build something that changes
    /// the machine without handing the executor a reason it can re-derive. This
    /// is the structural half of gate 4: before phase 3 the gate skipped any
    /// finding that recorded nothing, and the guarantee was "modules that record
    /// a justification get it verified" rather than "every action is verified".
    #[test]
    fn a_mutating_action_cannot_be_built_without_a_verifiable_justification() {
        for action in [Action::Quarantine, Action::DisableStartupEntry] {
            let err = Recommendation::new(
                finding_ids(),
                RiskBucket::Safe,
                95,
                action,
                "looks fine to me",
                10,
                Justification::Informational,
            )
            .expect_err("a mutating action needs a re-derivable justification");
            match err {
                CoreError::InvalidRecommendation(message) => assert!(
                    message.contains("re-verify"),
                    "the refusal must say why: `{message}`"
                ),
                other => panic!("expected an invalid-recommendation error, got {other:?}"),
            }
        }
    }

    #[test]
    fn every_justification_except_informational_is_verifiable() {
        assert!(!Justification::Informational.is_verifiable());
        assert!(a_rule().is_verifiable());
        assert!(Justification::DuplicateOf {
            content_hash: "abc".to_owned(),
            keeper: false,
            keeper_path: Some(PathBuf::from("C:\\a.bin")),
        }
        .is_verifiable());
        assert!(Justification::StartupEntry {
            source: StartupSource::CurrentUserRun,
            key: "OneDrive".to_owned(),
            command: "C:\\OneDrive.exe".to_owned(),
        }
        .is_verifiable());
    }

    #[test]
    fn justification_is_a_tagged_union_for_typescript() {
        let json = serde_json::to_string(&a_rule()).expect("serialize");
        assert_eq!(
            json,
            r#"{"kind":"rule","packId":"windows-temp","ruleId":"temp-old"}"#
        );
        let startup = serde_json::to_string(&Justification::StartupEntry {
            source: StartupSource::LocalMachineRun32,
            key: "Updater".to_owned(),
            command: "u.exe".to_owned(),
        })
        .expect("serialize");
        assert_eq!(
            startup,
            r#"{"kind":"startupEntry","source":"localMachineRun32","key":"Updater","command":"u.exe"}"#
        );
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
            a_rule(),
        )
        .expect("valid recommendation");
        let json = serde_json::to_string(&rec).expect("serialize");
        let back: Recommendation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(rec, back);
        assert!(json.contains(r#""savingsBytes":2048"#), "got {json}");
        assert!(
            json.contains(r#""justification":{"kind":"rule""#),
            "got {json}"
        );
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

    /* --- deserialisation validates too (security review L15) ------------ */

    /// A recommendation arriving over IPC or out of the `actions` table used to
    /// skip every check the constructor makes, because the fields are public and
    /// the derive is unconditional. The executor re-checked and refused it, so
    /// nothing broke — but the invariant held by luck rather than by type.
    #[test]
    fn a_forged_recommendation_cannot_be_deserialised() {
        let forgeries = [
            // Deleting a red-bucket item.
            serde_json::json!({
                "id": Uuid::new_v4(),
                "findingIds": [Uuid::new_v4()],
                "risk": "doNotDelete",
                "confidence": 99,
                "action": {"kind": "quarantine"},
                "reason": "looks fine to me",
                "savingsBytes": 1,
                "justification": {"kind": "informational"}
            }),
            // A mutating action with a justification gate 4 cannot re-derive.
            serde_json::json!({
                "id": Uuid::new_v4(),
                "findingIds": [Uuid::new_v4()],
                "risk": "safe",
                "confidence": 90,
                "action": {"kind": "quarantine"},
                "reason": "trust me",
                "savingsBytes": 1,
                "justification": {"kind": "informational"}
            }),
            // No reason at all, which CLAUDE.md forbids outright.
            serde_json::json!({
                "id": Uuid::new_v4(),
                "findingIds": [Uuid::new_v4()],
                "risk": "safe",
                "confidence": 90,
                "action": {"kind": "infoOnly"},
                "reason": "   ",
                "savingsBytes": 0,
                "justification": {"kind": "informational"}
            }),
            // No findings to point at.
            serde_json::json!({
                "id": Uuid::new_v4(),
                "findingIds": [],
                "risk": "safe",
                "confidence": 90,
                "action": {"kind": "infoOnly"},
                "reason": "about nothing",
                "savingsBytes": 0,
                "justification": {"kind": "informational"}
            }),
        ];

        for forgery in forgeries {
            let parsed = serde_json::from_value::<Recommendation>(forgery.clone());
            assert!(
                parsed.is_err(),
                "a recommendation that cannot be constructed must not be deserialisable: {forgery}"
            );
        }
    }

    #[test]
    fn a_valid_recommendation_still_round_trips() {
        let finding = Uuid::new_v4();
        let original = Recommendation::informational(
            vec![finding],
            RiskBucket::DoNotDelete,
            50,
            "this one is only here to be explained",
            0,
        )
        .expect("valid");

        let json = serde_json::to_string(&original).expect("serialize");
        let back: Recommendation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, original);
    }
}
