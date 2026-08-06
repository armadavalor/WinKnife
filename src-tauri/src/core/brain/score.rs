//! Steps 2-4 of the scoring pipeline (ARCHITECTURE section 4).
//!
//! Phase 1 stopped after the hard gates and used `score.max_bucket` as the
//! bucket, ignoring `score.base` entirely. This module turns the field on:
//!
//! ```text
//! base score (the rule)                      step 2
//!   + heuristic modifiers                    step 3
//!   -> thresholds >=85 safe / >=40 review    step 4
//!   -> capped by the rule's max_bucket       step 4
//! ```
//!
//! # The ceiling is a ceiling, never a floor
//!
//! `max_bucket` says how *good* a rating a rule is allowed to earn. Heuristics
//! may push an item down from it and never up through it, so a community pack
//! that writes `"base": 100` still cannot make its own rule green if it also
//! wrote `"max_bucket": "needs_review"`. And nothing here can lift a protected
//! path: gate 1 in [`crate::core::brain::Brain::classify`] returns before this
//! module is reached at all.
//!
//! # Why the modifiers are the ones they are
//!
//! - **Idle age** is the single best signal a cleaner has. A file nothing has
//!   opened in six months is a different proposition from one touched this
//!   morning, whatever folder it sits in — so age both adds (old) and subtracts
//!   (fresh).
//! - **Size** only moves *review candidates*. A big file is not safer than a
//!   small one; it is more worth a human's attention, and confidence is what
//!   orders the review list.
//! - **A known cache directory** is corroboration: the rule already said this
//!   looks disposable, and the path agreeing with it is a small nudge.
//! - **The owning process running** is the one modifier that moves a ceiling.
//!   A browser cache with the browser open is not safe to take, however
//!   confident the rule is (module spec 02).

use serde::{Deserialize, Serialize};

use super::rules::MaxBucket;
use crate::core::types::RiskBucket;

/// Score at or above which an item is green.
pub const SAFE_THRESHOLD: u8 = 85;

/// Score at or above which an item is yellow. Below it, red.
pub const REVIEW_THRESHOLD: u8 = 40;

/// Untouched for this long and a file counts as forgotten.
pub const STALE_DAYS: i64 = 180;

/// Untouched for this long and a file counts as cold.
pub const COLD_DAYS: i64 = 30;

/// Untouched for this long and a file counts as merely idle.
pub const WARM_DAYS: i64 = 7;

/// Touched inside this window and a file counts as in active use.
pub const FRESH_DAYS: i64 = 1;

/// A review candidate at least this large is worth ranking above its peers.
pub const LARGE_FILE_BYTES: u64 = 100 * 1024 * 1024;

/// Bonus for a file nothing has opened in [`STALE_DAYS`].
pub const STALE_BONUS: i16 = 10;

/// Bonus for a file nothing has opened in [`COLD_DAYS`].
pub const COLD_BONUS: i16 = 5;

/// Bonus for a file nothing has opened in [`WARM_DAYS`].
pub const WARM_BONUS: i16 = 2;

/// Penalty for a file touched inside [`FRESH_DAYS`].
pub const FRESH_PENALTY: i16 = -15;

/// Bonus for sitting inside a directory whose name says "cache".
pub const CACHE_DIR_BONUS: i16 = 5;

/// Bonus for a large review candidate.
pub const LARGE_REVIEW_BONUS: i16 = 4;

/// Penalty for the process that owns this data being open right now.
pub const OWNER_RUNNING_PENALTY: i16 = -25;

/// Directory names that mean "this is a cache" on Windows.
///
/// Compared as **component names**, not as paths: the question is "is one of
/// this path's folders called `Cache`", which is about a name and not about
/// where two paths point, so it does not go through
/// [`crate::core::path_compare`] (standing rule, `docs/DECISIONS-NEEDED.md`).
const CACHE_DIR_NAMES: &[&str] = &[
    "cache",
    "cache_data",
    "caches",
    "cache2",
    "code cache",
    "gpucache",
    "shadercache",
    "crashdumps",
    "temp",
    "tmp",
    "temporary internet files",
    "inetcache",
];

/// What the filesystem and the running system say about one candidate, beyond
/// the rule that matched it.
///
/// [`Signals::unknown`] is the "we were not told anything" value: it produces no
/// modifiers at all, so a caller with no metadata gets the rule's base score
/// unchanged rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signals {
    /// Days since the file was last opened, falling back to its modification
    /// time when Windows has last-access updates turned off. `None` when the
    /// filesystem gave us neither.
    pub idle_days: Option<i64>,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Whether a component of the path names a known cache directory.
    pub in_known_cache_dir: bool,
    /// Whether the process the matched rule names in `owner_process_absent` is
    /// running right now.
    pub owner_process_running: bool,
}

impl Signals {
    /// Signals that say nothing, so scoring is the rule's base score alone.
    pub fn unknown() -> Self {
        Self::default()
    }
}

/// One heuristic that moved the score, kept for the log and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Modifier {
    /// Short machine-ish label, e.g. `idle:stale`.
    pub label: &'static str,
    /// How much it added (or removed).
    pub delta: i16,
}

/// The outcome of steps 2-4 for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scored {
    /// The rule's declared starting score.
    pub base: u8,
    /// After modifiers, clamped to 0-100. This is the reported confidence.
    pub score: u8,
    /// What the thresholds alone said.
    pub by_threshold: RiskBucket,
    /// The ceiling the rule wrote.
    pub declared_ceiling: RiskBucket,
    /// The ceiling actually applied, after `owner_process_absent` may have
    /// lowered the declared one.
    pub ceiling: RiskBucket,
    /// The answer: the worse of [`Scored::by_threshold`] and [`Scored::ceiling`].
    pub bucket: RiskBucket,
    /// Every modifier that fired, in the order they were applied.
    pub modifiers: Vec<Modifier>,
}

impl Scored {
    /// Whether a heuristic made the outcome worse than the rule asked for.
    ///
    /// The UI uses this to decide whether the rule's own `reason` still tells
    /// the whole story, or whether it needs a sentence explaining the downgrade.
    pub fn was_downgraded(&self) -> bool {
        self.bucket > self.declared_ceiling
    }

    /// Whether a named modifier fired.
    pub fn fired(&self, label: &str) -> bool {
        self.modifiers.iter().any(|m| m.label == label)
    }
}

/// Label of the modifier that fires when the owning process is open.
pub const OWNER_RUNNING_LABEL: &str = "owner-process-running";

/// Which bucket a bare score lands in, before any ceiling.
pub fn bucket_for(score: u8) -> RiskBucket {
    if score >= SAFE_THRESHOLD {
        RiskBucket::Safe
    } else if score >= REVIEW_THRESHOLD {
        RiskBucket::NeedsReview
    } else {
        RiskBucket::DoNotDelete
    }
}

/// Apply a ceiling: the answer is the *worse* of the two.
///
/// [`RiskBucket`] orders `Safe < NeedsReview < DoNotDelete`, so "worse" is
/// `max`. Written out rather than inlined because "a ceiling is a max" reads
/// backwards until you know which way the enum points.
pub fn apply_ceiling(bucket: RiskBucket, ceiling: RiskBucket) -> RiskBucket {
    bucket.max(ceiling)
}

/// Run steps 2-4 for one candidate.
///
/// `base` and `ceiling` come from the matched rule; `signals` from the
/// filesystem and the process list. Pure — no I/O, so the whole table of cases
/// is a unit test.
pub fn score(base: u8, ceiling: MaxBucket, signals: &Signals) -> Scored {
    let mut modifiers = Vec::new();
    let mut running = i16::from(base);
    let declared_ceiling = RiskBucket::from(ceiling);
    let mut ceiling = declared_ceiling;

    // Step 3a: idle age. The strongest signal a cleaner has, in both directions.
    if let Some(days) = signals.idle_days {
        let modifier = if days >= STALE_DAYS {
            Some(Modifier {
                label: "idle:stale",
                delta: STALE_BONUS,
            })
        } else if days >= COLD_DAYS {
            Some(Modifier {
                label: "idle:cold",
                delta: COLD_BONUS,
            })
        } else if days >= WARM_DAYS {
            Some(Modifier {
                label: "idle:warm",
                delta: WARM_BONUS,
            })
        } else if days < FRESH_DAYS {
            Some(Modifier {
                label: "idle:fresh",
                delta: FRESH_PENALTY,
            })
        } else {
            None
        };
        if let Some(modifier) = modifier {
            running += modifier.delta;
            modifiers.push(modifier);
        }
    }

    // Step 3b: the path agreeing with the rule.
    if signals.in_known_cache_dir {
        let modifier = Modifier {
            label: "known-cache-dir",
            delta: CACHE_DIR_BONUS,
        };
        running += modifier.delta;
        modifiers.push(modifier);
    }

    // Step 3c: size, and only for review candidates — either the rule caps this
    // at review, or the running score has not reached the green threshold. A
    // large file is not *safer*, it is more worth looking at, and confidence is
    // what orders the review list.
    let is_review_candidate = ceiling != RiskBucket::Safe || running < i16::from(SAFE_THRESHOLD);
    if is_review_candidate && signals.size_bytes >= LARGE_FILE_BYTES {
        let modifier = Modifier {
            label: "large-review-candidate",
            delta: LARGE_REVIEW_BONUS,
        };
        running += modifier.delta;
        modifiers.push(modifier);
    }

    // Step 3d: the one modifier that moves a ceiling. A browser cache with the
    // browser open is not safe to take, whatever the rule's confidence
    // (module spec 02: "skip if browser running -> NeedsReview").
    if signals.owner_process_running {
        let modifier = Modifier {
            label: OWNER_RUNNING_LABEL,
            delta: OWNER_RUNNING_PENALTY,
        };
        running += modifier.delta;
        modifiers.push(modifier);
        ceiling = apply_ceiling(ceiling, RiskBucket::NeedsReview);
    }

    let score = running.clamp(0, i16::from(crate::core::types::MAX_CONFIDENCE)) as u8;
    let by_threshold = bucket_for(score);

    Scored {
        base,
        score,
        by_threshold,
        declared_ceiling,
        ceiling,
        bucket: apply_ceiling(by_threshold, ceiling),
        modifiers,
    }
}

/// Whether any component of `path` names a known cache directory.
///
/// A name test, not a path comparison — see [`CACHE_DIR_NAMES`].
pub fn in_known_cache_dir(path: &std::path::Path) -> bool {
    path.to_string_lossy()
        .replace('/', "\\")
        .split('\\')
        .any(|part| {
            let name = part.trim().to_lowercase();
            CACHE_DIR_NAMES.contains(&name.as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn signals() -> Signals {
        Signals::unknown()
    }

    #[test]
    fn the_thresholds_are_the_ones_the_architecture_states() {
        assert_eq!(SAFE_THRESHOLD, 85);
        assert_eq!(REVIEW_THRESHOLD, 40);

        assert_eq!(bucket_for(100), RiskBucket::Safe);
        assert_eq!(bucket_for(85), RiskBucket::Safe);
        assert_eq!(bucket_for(84), RiskBucket::NeedsReview);
        assert_eq!(bucket_for(40), RiskBucket::NeedsReview);
        assert_eq!(bucket_for(39), RiskBucket::DoNotDelete);
        assert_eq!(bucket_for(0), RiskBucket::DoNotDelete);
    }

    #[test]
    fn a_ceiling_is_the_worse_of_the_two() {
        use RiskBucket::{DoNotDelete, NeedsReview, Safe};
        assert_eq!(apply_ceiling(Safe, NeedsReview), NeedsReview);
        assert_eq!(apply_ceiling(NeedsReview, Safe), NeedsReview);
        assert_eq!(apply_ceiling(Safe, Safe), Safe);
        assert_eq!(apply_ceiling(Safe, DoNotDelete), DoNotDelete);
        assert_eq!(apply_ceiling(DoNotDelete, Safe), DoNotDelete);
    }

    #[test]
    fn unknown_signals_leave_the_base_score_untouched() {
        let scored = score(90, MaxBucket::Safe, &signals());
        assert_eq!(scored.score, 90);
        assert!(scored.modifiers.is_empty());
        assert_eq!(scored.bucket, RiskBucket::Safe);
        assert!(!scored.was_downgraded());
    }

    #[test]
    fn the_score_never_leaves_the_zero_to_one_hundred_range() {
        let high = score(
            100,
            MaxBucket::Safe,
            &Signals {
                idle_days: Some(400),
                in_known_cache_dir: true,
                ..signals()
            },
        );
        assert_eq!(high.score, 100, "modifiers must not overflow the scale");

        let low = score(
            5,
            MaxBucket::NeedsReview,
            &Signals {
                idle_days: Some(0),
                owner_process_running: true,
                ..signals()
            },
        );
        assert_eq!(low.score, 0, "modifiers must not go negative");
    }

    #[test]
    fn every_idle_band_fires_exactly_once() {
        let at = |days: i64| {
            score(
                50,
                MaxBucket::NeedsReview,
                &Signals {
                    idle_days: Some(days),
                    ..signals()
                },
            )
        };
        assert_eq!(at(400).modifiers.len(), 1);
        assert_eq!(at(400).modifiers[0].label, "idle:stale");
        assert_eq!(at(60).modifiers[0].label, "idle:cold");
        assert_eq!(at(10).modifiers[0].label, "idle:warm");
        assert!(
            at(3).modifiers.is_empty(),
            "the band between warm and fresh"
        );
        assert_eq!(at(0).modifiers[0].label, "idle:fresh");
    }

    #[test]
    fn size_only_moves_review_candidates() {
        let big = 200 * 1024 * 1024;

        // A comfortably-green item: size must not touch it.
        let green = score(
            95,
            MaxBucket::Safe,
            &Signals {
                size_bytes: big,
                ..signals()
            },
        );
        assert!(green.modifiers.is_empty(), "got {:?}", green.modifiers);

        // Same size, a rule that caps at review: the bonus fires.
        let yellow = score(
            95,
            MaxBucket::NeedsReview,
            &Signals {
                size_bytes: big,
                ..signals()
            },
        );
        assert_eq!(yellow.modifiers[0].label, "large-review-candidate");
        assert_eq!(yellow.score, 99);
        assert_eq!(yellow.bucket, RiskBucket::NeedsReview, "the ceiling holds");
    }

    #[test]
    fn a_running_owner_process_lowers_the_ceiling_as_well_as_the_score() {
        let scored = score(
            95,
            MaxBucket::Safe,
            &Signals {
                owner_process_running: true,
                ..signals()
            },
        );
        assert_eq!(scored.score, 70);
        assert_eq!(scored.ceiling, RiskBucket::NeedsReview);
        assert_eq!(scored.bucket, RiskBucket::NeedsReview);
        assert!(scored.was_downgraded());
    }

    /// Even with a score that clears the green threshold on its own, an open
    /// owner process caps the answer at yellow — the ceiling, not the score, is
    /// what does it.
    #[test]
    fn a_running_owner_process_caps_a_still_green_score_at_review() {
        let scored = score(
            100,
            MaxBucket::Safe,
            &Signals {
                idle_days: Some(365),
                in_known_cache_dir: true,
                owner_process_running: true,
                ..signals()
            },
        );
        assert!(scored.score >= SAFE_THRESHOLD, "score {}", scored.score);
        assert_eq!(scored.by_threshold, RiskBucket::Safe);
        assert_eq!(scored.bucket, RiskBucket::NeedsReview);
    }

    #[test]
    fn known_cache_directories_are_recognised_by_component_name() {
        for path in [
            "C:\\Users\\dev\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache\\f_00001",
            "C:/Users/dev/AppData/Local/Mozilla/Firefox/Profiles/x/cache2/entries/abc",
            "C:\\Users\\dev\\AppData\\Local\\Temp\\a.tmp",
            "C:\\Users\\dev\\AppData\\Local\\CrashDumps\\app.dmp",
            "C:\\Users\\dev\\AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\Code Cache\\js\\x",
        ] {
            assert!(in_known_cache_dir(Path::new(path)), "{path}");
        }
        for path in [
            "C:\\Users\\dev\\Documents\\report.docx",
            "C:\\Users\\dev\\Downloads\\cached-thing.zip",
            "C:\\Users\\dev\\Pictures\\uncached\\a.png",
        ] {
            assert!(!in_known_cache_dir(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn modifiers_serialise_for_the_log() {
        let json = serde_json::to_string(&Modifier {
            label: "idle:stale",
            delta: 10,
        })
        .expect("serialize");
        assert_eq!(json, r#"{"label":"idle:stale","delta":10}"#);
    }
}
