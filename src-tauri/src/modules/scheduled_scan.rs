//! Scheduled scan (C11): the normal read-only cleaner scan, run on a schedule,
//! with a short report of what it found.
//!
//! There is no OS scheduler behind this. K-021 rules out scheduled tasks, so the
//! UI asks `scheduled_scan_run_due` at start and every few minutes while the
//! app is open, and the scan runs once its slot has passed. A slot missed while
//! the app was closed runs at the next start.
//!
//! Auto-quarantine is off by default. When the user turns it on, only green
//! rows whose action is a plain quarantine go to the executor — never yellow,
//! red, informational or the recycle bin — and the executor's own gates still
//! re-check each one (K-006). Nothing is deleted (K-016).

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Days, Local, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::state::ScanResult;
use crate::core::types::{Action, Finding, FindingKind, Recommendation, RiskBucket};

/// Module id; also the settings key suffix and the UI route.
pub const MODULE_ID: &str = "scheduled-scan";

/// Where the last run's report is kept.
pub const KEY_LAST_REPORT: &str = "scheduled-scan.last_report";

const DAYS_PER_WEEK: u8 = 7;
const HOURS_PER_DAY: u8 = 24;
const MINUTES_PER_HOUR: u8 = 60;

/// How often the scan runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Frequency {
    /// Never; the default.
    #[default]
    Off,
    /// Every day at the set time.
    Daily,
    /// Once a week; `weekday` counts from Monday = 0.
    Weekly {
        /// 0 (Monday) to 6 (Sunday).
        weekday: u8,
    },
}

/// The user's schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduledScanSettings {
    /// Off, daily or weekly.
    pub frequency: Frequency,
    /// Local hour of the slot, 0-23.
    pub hour: u8,
    /// Local minute of the slot, 0-59.
    pub minute: u8,
    /// Move the green rows to quarantine after the scan. Off unless the user
    /// opts in.
    pub auto_quarantine: bool,
    /// When the current schedule was set. A slot before this never runs, so
    /// turning the schedule on does not fire a scan straight away. Set by the
    /// backend; whatever the UI sends is ignored.
    pub armed_at: Option<DateTime<Utc>>,
}

impl ScheduledScanSettings {
    /// Reject a clock time or weekday that does not exist.
    ///
    /// # Errors
    /// [`CoreError::InvalidTimer`] naming the bad field.
    pub fn validate(&self) -> Result<()> {
        if self.hour >= HOURS_PER_DAY || self.minute >= MINUTES_PER_HOUR {
            return Err(CoreError::InvalidTimer(format!(
                "{:02}:{:02} is not a time of day",
                self.hour, self.minute
            )));
        }
        if let Frequency::Weekly { weekday } = self.frequency {
            if weekday >= DAYS_PER_WEEK {
                return Err(CoreError::InvalidTimer(format!("{weekday} is not a weekday")));
            }
        }
        Ok(())
    }

    /// The settings to store for `requested`, given what is stored now: the
    /// arming time moves only when the schedule itself changed.
    pub fn updated(previous: &Self, requested: &Self, now: DateTime<Utc>) -> Self {
        let same_slot = previous.frequency == requested.frequency
            && previous.hour == requested.hour
            && previous.minute == requested.minute;
        Self {
            armed_at: if same_slot && previous.armed_at.is_some() {
                previous.armed_at
            } else {
                Some(now)
            },
            ..*requested
        }
    }
}

/// The most recent scheduled slot at or before `now`, in local time.
pub fn latest_slot(settings: &ScheduledScanSettings, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let time = NaiveTime::from_hms_opt(settings.hour.into(), settings.minute.into(), 0)?;
    let (days_back, period) = match settings.frequency {
        Frequency::Off => return None,
        Frequency::Daily => (0, 1),
        Frequency::Weekly { weekday } => {
            let today = u8::try_from(now.weekday().num_days_from_monday()).ok()?;
            ((today + DAYS_PER_WEEK - weekday) % DAYS_PER_WEEK, DAYS_PER_WEEK)
        }
    };
    let slot = now.date().checked_sub_days(Days::new(days_back.into()))?.and_time(time);
    if slot <= now {
        Some(slot)
    } else {
        slot.checked_sub_days(Days::new(period.into()))
    }
}

/// Whether a scan is owed: a slot has passed since the last run and since the
/// schedule was set. All times are local wall-clock.
pub fn is_due(
    settings: &ScheduledScanSettings,
    last_run: Option<NaiveDateTime>,
    armed_at: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> bool {
    let Some(slot) = latest_slot(settings, now) else {
        return false;
    };
    last_run.max(armed_at).is_none_or(|seen| seen < slot)
}

/// [`is_due`] against the real local clock.
pub fn is_due_now(settings: &ScheduledScanSettings, last_run: Option<DateTime<Utc>>) -> bool {
    let local = |at: DateTime<Utc>| at.with_timezone(&Local).naive_local();
    is_due(
        settings,
        last_run.map(local),
        settings.armed_at.map(local),
        Local::now().naive_local(),
    )
}

/// The recommendations auto-quarantine may hand to the executor.
///
/// Green, a plain quarantine, and every finding it names is known and is not
/// the recycle bin (emptying that is permanent and needs its own consent). The
/// executor re-checks each one; this is the first filter, not the only one.
pub fn auto_quarantine_ids(recommendations: &[Recommendation], findings: &[Finding]) -> Vec<Uuid> {
    let kinds: HashMap<Uuid, FindingKind> = findings.iter().map(|f| (f.id, f.kind)).collect();
    recommendations
        .iter()
        .filter(|rec| rec.risk == RiskBucket::Safe && rec.action == Action::Quarantine)
        .filter(|rec| {
            !rec.finding_ids.is_empty()
                && rec.finding_ids.iter().all(|id| {
                    kinds
                        .get(id)
                        .is_some_and(|kind| *kind != FindingKind::RecycleBinItem)
                })
        })
        .map(|rec| rec.id)
        .collect()
}

/// Rows and bytes in one bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketTotals {
    /// Recommendations in the bucket.
    pub count: u64,
    /// Bytes they would free.
    pub bytes: u64,
}

/// Why auto-quarantine did not run after a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// Another clean or restore was running.
    Busy,
    /// The scan was stopped, so its list is incomplete.
    Cancelled,
    /// The executor refused the batch; the log has the detail.
    Failed,
    /// A manual scan was still waiting in Review; moving files from under it
    /// would change what the user is looking at.
    ManualReview,
}

/// What auto-quarantine did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutoQuarantine {
    /// The batch ran.
    #[serde(rename_all = "camelCase")]
    Ran {
        /// Items moved to quarantine.
        moved: u64,
        /// Items the executor refused or failed.
        not_moved: u64,
        /// Bytes moved.
        bytes: u64,
        /// The quarantine batch, when anything moved.
        batch_id: Option<Uuid>,
    },
    /// It did not run.
    Skipped {
        /// Why.
        reason: SkipReason,
    },
}

/// The report a scheduled run leaves behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledReport {
    /// The scan, for "Review" while it is still retained.
    pub scan_id: Uuid,
    /// When the scan ran.
    pub ran_at: DateTime<Utc>,
    /// Whether the scan was stopped before the end.
    pub cancelled: bool,
    /// Green.
    pub safe: BucketTotals,
    /// Yellow.
    pub review: BucketTotals,
    /// Red; count only, bytes are not on offer.
    pub blocked: BucketTotals,
    /// `None` when auto-quarantine is off.
    pub auto_quarantine: Option<AutoQuarantine>,
}

impl ScheduledReport {
    /// Count a finished scan, bucket by bucket.
    pub fn from_scan(scan: &ScanResult, ran_at: DateTime<Utc>) -> Self {
        let mut report = Self {
            scan_id: scan.scan_id,
            ran_at,
            cancelled: scan.cancelled,
            safe: BucketTotals::default(),
            review: BucketTotals::default(),
            blocked: BucketTotals::default(),
            auto_quarantine: None,
        };
        for rec in &scan.recommendations {
            let bucket = match rec.risk {
                RiskBucket::Safe => &mut report.safe,
                RiskBucket::NeedsReview => &mut report.review,
                RiskBucket::DoNotDelete => &mut report.blocked,
            };
            bucket.count += 1;
            if rec.risk != RiskBucket::DoNotDelete {
                bucket.bytes += rec.savings_bytes;
            }
        }
        report
    }
}

/// Settings plus the last report, for the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledScanState {
    /// The schedule.
    pub settings: ScheduledScanSettings,
    /// The last run, if any.
    pub last_report: Option<ScheduledReport>,
}

/// Registry entry; the work is in `commands.rs`.
#[derive(Debug, Default)]
pub struct ScheduledScanModule;

impl ToolModule for ScheduledScanModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Scheduled Scan".to_owned(),
            description:
                "Runs the cleaner's read-only scan daily or weekly and keeps a report for you to review."
                    .to_owned(),
            icon: "clock".to_owned(),
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Justification;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        // 2026-10-26 is a Monday.
        NaiveDate::from_ymd_opt(2026, 10, day)
            .and_then(|d| d.and_hms_opt(hour, minute, 0))
            .expect("valid time")
    }

    fn schedule(frequency: Frequency, hour: u8, minute: u8) -> ScheduledScanSettings {
        ScheduledScanSettings {
            frequency,
            hour,
            minute,
            ..ScheduledScanSettings::default()
        }
    }

    #[test]
    fn off_is_never_due() {
        let off = schedule(Frequency::Off, 3, 0);
        assert!(!is_due(&off, None, None, at(26, 12, 0)));
    }

    #[test]
    fn daily_slot_is_today_after_the_time_and_yesterday_before_it() {
        let daily = schedule(Frequency::Daily, 3, 30);
        assert_eq!(latest_slot(&daily, at(26, 3, 30)), Some(at(26, 3, 30)));
        assert_eq!(latest_slot(&daily, at(26, 12, 0)), Some(at(26, 3, 30)));
        assert_eq!(latest_slot(&daily, at(26, 3, 29)), Some(at(25, 3, 30)));
    }

    #[test]
    fn weekly_slot_lands_on_the_chosen_weekday() {
        // Wednesday = 2.
        let weekly = schedule(Frequency::Weekly { weekday: 2 }, 9, 0);
        assert_eq!(latest_slot(&weekly, at(26, 12, 0)), Some(at(21, 9, 0)));
        assert_eq!(latest_slot(&weekly, at(28, 8, 59)), Some(at(21, 9, 0)));
        assert_eq!(latest_slot(&weekly, at(28, 9, 0)), Some(at(28, 9, 0)));
        // Monday = 0, the same day as "now".
        let monday = schedule(Frequency::Weekly { weekday: 0 }, 9, 0);
        assert_eq!(latest_slot(&monday, at(26, 8, 0)), Some(at(19, 9, 0)));
    }

    #[test]
    fn due_once_per_slot() {
        let daily = schedule(Frequency::Daily, 3, 0);
        let armed = Some(at(20, 12, 0));
        assert!(is_due(&daily, None, armed, at(26, 12, 0)));
        assert!(is_due(&daily, Some(at(25, 3, 1)), armed, at(26, 12, 0)));
        assert!(!is_due(&daily, Some(at(26, 3, 5)), armed, at(26, 12, 0)));
        assert!(!is_due(&daily, Some(at(26, 3, 5)), armed, at(26, 23, 59)));
    }

    #[test]
    fn arming_after_the_slot_waits_for_the_next_one() {
        let daily = schedule(Frequency::Daily, 3, 0);
        let armed = Some(at(26, 10, 0));
        assert!(!is_due(&daily, None, armed, at(26, 11, 0)));
        assert!(is_due(&daily, None, armed, at(27, 3, 0)));
    }

    #[test]
    fn rearming_moves_only_when_the_slot_changes() {
        let then = Utc::now() - chrono::Duration::days(3);
        let now = Utc::now();
        let stored = ScheduledScanSettings {
            armed_at: Some(then),
            ..schedule(Frequency::Daily, 3, 0)
        };
        let toggled = ScheduledScanSettings {
            auto_quarantine: true,
            armed_at: None,
            ..stored
        };
        assert_eq!(ScheduledScanSettings::updated(&stored, &toggled, now).armed_at, Some(then));
        let moved = schedule(Frequency::Daily, 4, 0);
        assert_eq!(ScheduledScanSettings::updated(&stored, &moved, now).armed_at, Some(now));
    }

    #[test]
    fn validate_rejects_impossible_times() {
        assert!(schedule(Frequency::Daily, 23, 59).validate().is_ok());
        assert!(schedule(Frequency::Daily, 24, 0).validate().is_err());
        assert!(schedule(Frequency::Daily, 0, 60).validate().is_err());
        assert!(schedule(Frequency::Weekly { weekday: 7 }, 0, 0).validate().is_err());
    }

    fn finding(kind: FindingKind) -> Finding {
        Finding {
            id: Uuid::new_v4(),
            module: "cleaner".to_owned(),
            path: PathBuf::from(r"C:\Users\u\AppData\Local\Temp\x.tmp"),
            kind,
            size_bytes: 10,
            created: None,
            modified: None,
            accessed: None,
            meta: serde_json::Value::Null,
        }
    }

    fn rule() -> Justification {
        Justification::Rule {
            pack_id: "p".to_owned(),
            rule_id: "r".to_owned(),
        }
    }

    fn rec(finding: &Finding, risk: RiskBucket, action: Action) -> Recommendation {
        Recommendation::new(vec![finding.id], risk, 90, action, "a reason", 10, rule())
            .expect("valid recommendation")
    }

    #[test]
    fn auto_quarantine_takes_only_green_quarantine_rows() {
        let green = finding(FindingKind::File);
        let yellow = finding(FindingKind::File);
        let red = finding(FindingKind::File);
        let bin = finding(FindingKind::RecycleBinItem);
        let info = finding(FindingKind::File);
        let startup = finding(FindingKind::File);
        let forged_red = finding(FindingKind::File);

        let keep = rec(&green, RiskBucket::Safe, Action::Quarantine);
        // `Recommendation::new` refuses an action on a red row; forge one to
        // prove the filter does not trust that check alone.
        let mut forged = rec(&forged_red, RiskBucket::Safe, Action::Quarantine);
        forged.risk = RiskBucket::DoNotDelete;
        let recs = vec![
            keep.clone(),
            rec(&yellow, RiskBucket::NeedsReview, Action::Quarantine),
            Recommendation::informational(vec![red.id], RiskBucket::DoNotDelete, 90, "protected", 0)
                .expect("valid"),
            rec(&bin, RiskBucket::Safe, Action::Quarantine),
            Recommendation::informational(vec![info.id], RiskBucket::Safe, 90, "fyi", 0)
                .expect("valid"),
            rec(&startup, RiskBucket::Safe, Action::DisableStartupEntry),
            forged,
        ];
        let findings = vec![green, yellow, red, bin, info, startup, forged_red];

        assert_eq!(auto_quarantine_ids(&recs, &findings), vec![keep.id]);
    }

    #[test]
    fn auto_quarantine_skips_rows_whose_finding_is_missing() {
        let green = finding(FindingKind::File);
        let recs = vec![rec(&green, RiskBucket::Safe, Action::Quarantine)];
        assert!(auto_quarantine_ids(&recs, &[]).is_empty());
    }

    #[test]
    fn report_counts_each_bucket() {
        let a = finding(FindingKind::File);
        let b = finding(FindingKind::File);
        let c = finding(FindingKind::File);
        let scan = ScanResult {
            scan_id: Uuid::new_v4(),
            module: "cleaner".to_owned(),
            recommendations: vec![
                rec(&a, RiskBucket::Safe, Action::Quarantine),
                rec(&b, RiskBucket::NeedsReview, Action::Quarantine),
                Recommendation::informational(vec![c.id], RiskBucket::DoNotDelete, 90, "no", 0)
                    .expect("valid"),
            ],
            findings: vec![a, b, c],
            skipped_rules: Vec::new(),
            cancelled: false,
        };
        let report = ScheduledReport::from_scan(&scan, Utc::now());
        assert_eq!(report.safe, BucketTotals { count: 1, bytes: 10 });
        assert_eq!(report.review, BucketTotals { count: 1, bytes: 10 });
        assert_eq!(report.blocked.count, 1);
        assert_eq!(report.auto_quarantine, None);
    }
}
