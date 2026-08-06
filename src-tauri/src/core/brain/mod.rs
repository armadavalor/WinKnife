//! Rule engine and hard gates (ARCHITECTURE section 4).
//!
//! **Phase 1 scope: gates only.** The scoring pipeline the architecture
//! describes has four steps; this phase implements step 1 (hard gates) and a
//! degenerate step 2 (a rule declares a bucket and that is the bucket). Steps 3
//! and 4 — heuristic modifiers and score thresholds — are phase 2. `score.base`
//! is parsed and validated so today's packs stay valid, and deliberately not
//! consulted, because a half-implemented score is worse than none.
//!
//! The gates, in the order they run:
//!
//! 1. **Protected path** → `DoNotDelete`, confidence 100, stop. Nothing a rule
//!    pack says can override this, which is why the list is Rust and not JSON.
//! 2. **Locked file** → `DoNotDelete`. A file another program is holding open
//!    is not ours to move.
//! 3. **Rule match** → the rule's declared bucket, its reason, its action.
//! 4. **No rule matched** → `DoNotDelete` with an honest "we do not recognise
//!    this" reason. The red bucket is produced, not targeted (module spec 02).

pub mod protected;
pub mod rules;

use std::path::Path;
use std::sync::Arc;

use crate::core::platform::{AlwaysUnlocked, LockProbe, LockState};
use crate::core::types::{Action, RiskBucket};

pub use protected::{ProtectedEntry, ProtectedPaths, Scope};
pub use rules::{CompiledRule, MaxBucket, RuleAction, RulePack, RuleSpec};

/// Confidence reported for a hard-gate decision: no rule, no heuristic, no
/// doubt.
pub const GATE_CONFIDENCE: u8 = 100;

/// Confidence reported for a plain rule match while scoring is unimplemented.
/// Phase 2 replaces this with the computed score.
pub const RULE_MATCH_CONFIDENCE: u8 = 90;

/// Why a verdict came out the way it did. Useful in tests and in the log; the
/// UI only ever sees `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictSource {
    /// A protected path forced the answer.
    ProtectedPath,
    /// The file is in use or unreadable.
    Locked,
    /// A rule pack matched.
    Rule,
    /// Nothing matched.
    Unknown,
}

/// What the brain concluded about one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The bucket the UI puts it in.
    pub bucket: RiskBucket,
    /// 0-100.
    pub confidence: u8,
    /// What we propose. Always [`Action::InfoOnly`] for a red verdict.
    pub action: Action,
    /// Plain-language justification, shown verbatim next to the item.
    pub reason: String,
    /// Which gate or rule decided.
    pub source: VerdictSource,
    /// Pack and rule that matched, when one did.
    pub rule: Option<(String, String)>,
}

impl Verdict {
    /// Whether the UI may offer a button for this.
    pub fn is_actionable(&self) -> bool {
        self.bucket.is_actionable() && self.action.is_mutating()
    }
}

/// Shared analysis context handed to every module's `analyze`.
#[derive(Clone)]
pub struct Brain {
    protected: ProtectedPaths,
    packs: Arc<Vec<RulePack>>,
    lock_probe: Arc<dyn LockProbe>,
}

impl std::fmt::Debug for Brain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Brain")
            .field("protected_entries", &self.protected.entries().len())
            .field(
                "packs",
                &self.packs.iter().map(RulePack::id).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for Brain {
    fn default() -> Self {
        Self::new()
    }
}

impl Brain {
    /// A brain with no rules and no protected paths. Only useful as a
    /// placeholder; the app always uses [`Brain::builder`].
    pub fn new() -> Self {
        Self {
            protected: ProtectedPaths::empty(),
            packs: Arc::new(Vec::new()),
            lock_probe: Arc::new(AlwaysUnlocked),
        }
    }

    /// Start building a configured brain.
    pub fn builder() -> BrainBuilder {
        BrainBuilder::default()
    }

    /// The protected-path list in force.
    pub fn protected_paths(&self) -> &ProtectedPaths {
        &self.protected
    }

    /// Loaded packs.
    pub fn packs(&self) -> &[RulePack] {
        &self.packs
    }

    /// Why `path` is protected, or `None`. The executor calls this again at
    /// execute time rather than trusting the analyze-time verdict.
    pub fn protection_reason(&self, path: &Path) -> Option<String> {
        self.protected.reason_for(path).map(|reason| {
            format!("protected location, not cleanable: {reason}")
        })
    }

    /// Current lock state of `path`.
    pub fn lock_state(&self, path: &Path) -> LockState {
        self.lock_probe.probe(path)
    }

    /// Classify a path that a rule already matched.
    ///
    /// `check_lock` exists because the two callers want different things: the
    /// cleaner's `analyze` probes the filesystem, while table-driven tests pass
    /// paths that do not exist and only care about the path gates.
    pub fn classify(&self, path: &Path, matched: Option<&CompiledRule>) -> Verdict {
        // Gate 1: protected paths beat everything, including a rule that
        // claims otherwise. This is the line CLAUDE.md calls a hard rule.
        if let Some(reason) = self.protection_reason(path) {
            return Verdict {
                bucket: RiskBucket::DoNotDelete,
                confidence: GATE_CONFIDENCE,
                action: Action::InfoOnly,
                reason,
                source: VerdictSource::ProtectedPath,
                rule: matched.map(|rule| (rule.pack_id.clone(), rule.spec.id.clone())),
            };
        }

        // Gate 2: a file something else is holding open.
        let lock = self.lock_state(path);
        if !lock.is_free() {
            return Verdict {
                bucket: RiskBucket::DoNotDelete,
                confidence: GATE_CONFIDENCE,
                action: Action::InfoOnly,
                reason: format!("left alone: {}", lock.reason()),
                source: VerdictSource::Locked,
                rule: matched.map(|rule| (rule.pack_id.clone(), rule.spec.id.clone())),
            };
        }

        let Some(rule) = matched else {
            // Gate 4: honest ignorance. The red bucket is produced, not
            // targeted — this is the honesty feature from module spec 02.
            return Verdict {
                bucket: RiskBucket::DoNotDelete,
                confidence: GATE_CONFIDENCE,
                action: Action::InfoOnly,
                reason: "no rule recognises this location, so it is left alone".to_owned(),
                source: VerdictSource::Unknown,
                rule: None,
            };
        };

        // Step 2: the rule's declared bucket, unmodified. No scoring in phase 1.
        let bucket = rule.bucket();
        let action = match (bucket, rule.spec.action) {
            (RiskBucket::DoNotDelete, _) | (_, RuleAction::InfoOnly) => Action::InfoOnly,
            (_, RuleAction::Quarantine | RuleAction::EmptyRecycleBin) => Action::Quarantine,
        };

        Verdict {
            bucket,
            confidence: if bucket == RiskBucket::DoNotDelete {
                GATE_CONFIDENCE
            } else {
                RULE_MATCH_CONFIDENCE
            },
            action,
            reason: rule.spec.reason.clone(),
            source: VerdictSource::Rule,
            rule: Some((rule.pack_id.clone(), rule.spec.id.clone())),
        }
    }
}

/// Assembles a [`Brain`]. Everything is injectable so a test never depends on
/// the machine it runs on.
#[derive(Default)]
pub struct BrainBuilder {
    protected: Option<ProtectedPaths>,
    packs: Vec<RulePack>,
    lock_probe: Option<Arc<dyn LockProbe>>,
}

impl BrainBuilder {
    /// Set the protected-path list.
    #[must_use]
    pub fn protected_paths(mut self, protected: ProtectedPaths) -> Self {
        self.protected = Some(protected);
        self
    }

    /// Add rule packs.
    #[must_use]
    pub fn packs(mut self, packs: Vec<RulePack>) -> Self {
        self.packs = packs;
        self
    }

    /// Set the lock probe.
    #[must_use]
    pub fn lock_probe(mut self, probe: Arc<dyn LockProbe>) -> Self {
        self.lock_probe = Some(probe);
        self
    }

    /// Build it.
    pub fn build(self) -> Brain {
        Brain {
            protected: self.protected.unwrap_or_else(ProtectedPaths::empty),
            packs: Arc::new(self.packs),
            lock_probe: self.lock_probe.unwrap_or_else(|| Arc::new(AlwaysUnlocked)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::rules::compile_pack;
    use crate::core::platform::MapEnv;
    use std::path::PathBuf;

    const PACK: &str = r#"{
        "id": "windows-temp",
        "name": "Windows Temp Files",
        "version": 1,
        "module": "cleaner",
        "enabled_by_default": true,
        "description": "Temporary files.",
        "rules": [
            {
                "id": "user-temp-files",
                "paths": ["%TEMP%\\**"],
                "score": { "base": 90, "max_bucket": "safe" },
                "action": "quarantine",
                "reason": "Temporary file older than 24h in the user temp folder."
            },
            {
                "id": "review-me",
                "paths": ["%TEMP%\\keep\\**"],
                "score": { "base": 50, "max_bucket": "needs_review" },
                "action": "quarantine",
                "reason": "Looks disposable but has not been checked."
            }
        ]
    }"#;

    fn pack() -> RulePack {
        compile_pack(
            "windows-temp",
            PACK,
            &MapEnv::new().with("TEMP", "C:\\Users\\dev\\AppData\\Local\\Temp"),
        )
        .expect("compile")
    }

    fn brain() -> Brain {
        let env = MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("ProgramFiles", "C:\\Program Files")
            .with("USERPROFILE", "C:\\Users\\dev");
        Brain::builder()
            .protected_paths(ProtectedPaths::from_env(
                &env,
                Path::new("C:\\app"),
                Path::new("C:\\app\\quarantine"),
            ))
            .packs(vec![pack()])
            .build()
    }

    fn rule<'a>(pack: &'a RulePack, id: &str) -> &'a CompiledRule {
        pack.rules
            .iter()
            .find(|rule| rule.spec.id == id)
            .expect("rule exists")
    }

    /// The mandatory table-driven gate test: path + matched rule → bucket.
    #[test]
    fn hard_gate_bucket_table() {
        let brain = brain();
        let pack = pack();
        let safe = rule(&pack, "user-temp-files");
        let review = rule(&pack, "review-me");

        struct Case<'a> {
            path: &'a str,
            matched: Option<&'a CompiledRule>,
            bucket: RiskBucket,
            source: VerdictSource,
            why: &'a str,
        }

        let cases = [
            Case {
                path: "C:\\Users\\dev\\AppData\\Local\\Temp\\a.tmp",
                matched: Some(safe),
                bucket: RiskBucket::Safe,
                source: VerdictSource::Rule,
                why: "a safe rule in an unprotected place",
            },
            Case {
                path: "C:\\Users\\dev\\AppData\\Local\\Temp\\keep\\b.tmp",
                matched: Some(review),
                bucket: RiskBucket::NeedsReview,
                source: VerdictSource::Rule,
                why: "the rule's declared bucket is used verbatim",
            },
            Case {
                path: "C:\\Windows\\System32\\ntoskrnl.exe",
                matched: Some(safe),
                bucket: RiskBucket::DoNotDelete,
                source: VerdictSource::ProtectedPath,
                why: "a protected path beats a safe rule",
            },
            Case {
                path: "C:\\Program Files\\App\\app.exe",
                matched: Some(safe),
                bucket: RiskBucket::DoNotDelete,
                source: VerdictSource::ProtectedPath,
                why: "installed programs are protected",
            },
            Case {
                path: "C:\\Users\\dev",
                matched: Some(safe),
                bucket: RiskBucket::DoNotDelete,
                source: VerdictSource::ProtectedPath,
                why: "the profile root itself is protected",
            },
            Case {
                path: "C:\\app\\quarantine\\batch\\item",
                matched: Some(safe),
                bucket: RiskBucket::DoNotDelete,
                source: VerdictSource::ProtectedPath,
                why: "we never clean our own quarantine",
            },
            Case {
                path: "C:\\",
                matched: Some(safe),
                bucket: RiskBucket::DoNotDelete,
                source: VerdictSource::ProtectedPath,
                why: "drive roots are protected",
            },
            Case {
                path: "C:\\Some\\Unknown\\Place\\file.bin",
                matched: None,
                bucket: RiskBucket::DoNotDelete,
                source: VerdictSource::Unknown,
                why: "no rule means no suggestion",
            },
        ];

        for case in cases {
            let verdict = brain.classify(Path::new(case.path), case.matched);
            assert_eq!(
                verdict.bucket, case.bucket,
                "{} — {}",
                case.path, case.why
            );
            assert_eq!(verdict.source, case.source, "{}", case.path);
            assert!(
                !verdict.reason.trim().is_empty(),
                "{} produced a verdict with no reason",
                case.path
            );
        }
    }

    /// The hard rule, stated as its own test so it cannot be diluted: no path in
    /// the protected list can ever come back green or carry a delete action.
    #[test]
    fn a_protected_path_can_never_be_safe_or_actionable() {
        let brain = brain();
        let pack = pack();
        let safe_rule = rule(&pack, "user-temp-files");

        for path in [
            "C:\\Windows",
            "C:\\Windows\\Temp\\junk.tmp",
            "C:\\Program Files\\Thing\\a.dll",
            "C:\\Users\\dev",
            "C:\\",
            "C:\\app\\quarantine\\x",
            "C:\\System Volume Information\\tracking.log",
        ] {
            let verdict = brain.classify(Path::new(path), Some(safe_rule));
            assert_eq!(verdict.bucket, RiskBucket::DoNotDelete, "{path}");
            assert_ne!(verdict.bucket, RiskBucket::Safe, "{path}");
            assert_eq!(verdict.action, Action::InfoOnly, "{path}");
            assert!(!verdict.action.is_mutating(), "{path}");
            assert!(!verdict.is_actionable(), "{path}");
            assert_eq!(verdict.confidence, GATE_CONFIDENCE, "{path}");
        }
    }

    #[test]
    fn a_locked_file_is_gated_to_red_even_with_a_safe_rule() {
        struct Locked;
        impl LockProbe for Locked {
            fn probe(&self, _path: &Path) -> LockState {
                LockState::InUse
            }
        }

        let pack = pack();
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::empty())
            .packs(vec![pack.clone()])
            .lock_probe(Arc::new(Locked))
            .build();

        let verdict = brain.classify(
            Path::new("C:\\Users\\dev\\AppData\\Local\\Temp\\busy.tmp"),
            Some(rule(&pack, "user-temp-files")),
        );

        assert_eq!(verdict.bucket, RiskBucket::DoNotDelete);
        assert_eq!(verdict.source, VerdictSource::Locked);
        assert_eq!(verdict.action, Action::InfoOnly);
        assert!(verdict.reason.contains("in use"), "got `{}`", verdict.reason);
    }

    #[test]
    fn an_inaccessible_file_is_gated_too() {
        struct NoAccess;
        impl LockProbe for NoAccess {
            fn probe(&self, _path: &Path) -> LockState {
                LockState::Inaccessible
            }
        }

        let pack = pack();
        let brain = Brain::builder()
            .packs(vec![pack.clone()])
            .lock_probe(Arc::new(NoAccess))
            .build();

        let verdict = brain.classify(
            Path::new("C:\\Users\\dev\\AppData\\Local\\Temp\\denied.tmp"),
            Some(rule(&pack, "user-temp-files")),
        );
        assert_eq!(verdict.bucket, RiskBucket::DoNotDelete);
        assert_eq!(verdict.source, VerdictSource::Locked);
    }

    #[test]
    fn protected_paths_are_checked_before_locks() {
        struct Locked;
        impl LockProbe for Locked {
            fn probe(&self, _path: &Path) -> LockState {
                LockState::InUse
            }
        }

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([PathBuf::from("C:\\Windows")]))
            .lock_probe(Arc::new(Locked))
            .build();

        let verdict = brain.classify(Path::new("C:\\Windows\\a.dll"), None);
        assert_eq!(verdict.source, VerdictSource::ProtectedPath);
    }

    /// The protected-path injection test from module spec 02: plant a
    /// `C:\Windows` stand-in inside a fixture root and it must still land red.
    #[test]
    fn an_injected_protected_path_inside_a_fixture_lands_red() {
        let fixture_root = PathBuf::from("C:\\fixtures\\dirty-profile");
        let planted = fixture_root.join("Windows");
        let pack = pack();

        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([planted.clone()]))
            .packs(vec![pack.clone()])
            .build();

        // A rule that claims this is safe, as a buggy community pack would.
        let verdict = brain.classify(
            &planted.join("System32").join("victim.dll"),
            Some(rule(&pack, "user-temp-files")),
        );

        assert_eq!(verdict.bucket, RiskBucket::DoNotDelete);
        assert_eq!(verdict.source, VerdictSource::ProtectedPath);
        assert!(!verdict.is_actionable());
    }

    #[test]
    fn an_unmatched_path_says_so_in_plain_language() {
        let verdict = brain().classify(Path::new("C:\\Nowhere\\thing.bin"), None);
        assert_eq!(verdict.source, VerdictSource::Unknown);
        assert!(verdict.reason.contains("no rule"), "got `{}`", verdict.reason);
        assert_eq!(verdict.rule, None);
    }

    #[test]
    fn a_rule_verdict_names_its_pack_and_rule() {
        let pack = pack();
        let brain = Brain::builder().packs(vec![pack.clone()]).build();
        let verdict = brain.classify(
            Path::new("C:\\Users\\dev\\AppData\\Local\\Temp\\a.tmp"),
            Some(rule(&pack, "user-temp-files")),
        );
        assert_eq!(
            verdict.rule,
            Some(("windows-temp".to_owned(), "user-temp-files".to_owned()))
        );
        assert_eq!(verdict.confidence, RULE_MATCH_CONFIDENCE);
    }

    #[test]
    fn the_rules_reason_reaches_the_verdict_verbatim() {
        let pack = pack();
        let brain = Brain::builder().packs(vec![pack.clone()]).build();
        let verdict = brain.classify(
            Path::new("C:\\Users\\dev\\AppData\\Local\\Temp\\a.tmp"),
            Some(rule(&pack, "user-temp-files")),
        );
        assert_eq!(
            verdict.reason,
            "Temporary file older than 24h in the user temp folder."
        );
    }

    #[test]
    fn an_empty_brain_recognises_nothing_and_protects_nothing() {
        let brain = Brain::new();
        assert!(brain.packs().is_empty());
        assert!(brain.protected_paths().entries().is_empty());
        let verdict = brain.classify(Path::new("C:\\Windows\\System32"), None);
        assert_eq!(verdict.source, VerdictSource::Unknown);
    }
}
