//! Rule engine, hard gates and scoring (ARCHITECTURE section 4).
//!
//! The full four-step pipeline, in the order it runs:
//!
//! 1. **Hard gates.** A protected path → `DoNotDelete`, confidence 100, stop.
//!    Nothing a rule pack says can override this, which is why the list is Rust
//!    and not JSON. Then a locked file → `DoNotDelete`: a file another program
//!    is holding open is not ours to move.
//! 2. **Rule match** → the rule's `score.base` and its `score.max_bucket`.
//! 3. **Heuristic modifiers** — idle age, size, known cache directory, and the
//!    owning process being open (`core::brain::score`).
//! 4. **Thresholds** — ≥85 Safe, 40–84 NeedsReview, <40 DoNotDelete — capped by
//!    the rule's `max_bucket`, which is a ceiling a rule can never earn its way
//!    through.
//!
//! When nothing matches at all the answer is `DoNotDelete` with an honest "we do
//! not recognise this" reason. The red bucket is produced, not targeted (module
//! spec 02).
//!
//! Phase 1 implemented step 1 and a degenerate step 2 (a rule declared a bucket
//! and that was the bucket) while parsing and ignoring `score.base`. Phase 2
//! turns the field on; packs written against phase 1 keep working, because a
//! caller with no metadata passes [`score::Signals::unknown`] and gets the base
//! score through unchanged.

pub mod protected;
pub mod rules;
pub mod score;

use std::path::Path;
use std::sync::Arc;

use crate::core::platform::{AlwaysUnlocked, LockProbe, LockState, ProcessProbe, StubProcessProbe};
use crate::core::types::{Action, Finding, FindingKind, RiskBucket};

pub use protected::{gate_path, GateVerdict, ProtectedEntry, ProtectedPaths, Scope};
pub use rules::{CompiledRule, MaxBucket, RuleAction, RulePack, RuleSpec};
pub use score::{Scored, Signals};

/// Confidence reported for a hard-gate decision: no rule, no heuristic, no
/// doubt.
pub const GATE_CONFIDENCE: u8 = 100;

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
    /// The score and every modifier that fired, when scoring ran at all. `None`
    /// for a hard-gate verdict, which no score can argue with.
    pub scored: Option<Scored>,
}

impl Verdict {
    /// Whether the UI may offer a button for this.
    pub fn is_actionable(&self) -> bool {
        self.bucket.is_actionable() && self.action.is_mutating()
    }
}

/// Why the hard path gates refuse a path. Produced by [`Brain::path_refusal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRefusal {
    /// Plain-language justification, shown to the user verbatim.
    pub reason: String,
    /// The spelling the path really resolves to, when the gate could work one
    /// out and it was that form which turned out to be protected. For the log —
    /// "`C:\PROGRA~1\x` resolves into `C:\Program Files`" is the interesting
    /// half of the audit line.
    pub canonical: Option<std::path::PathBuf>,
}

/// Shared analysis context handed to every module's `analyze`.
#[derive(Clone)]
pub struct Brain {
    protected: ProtectedPaths,
    packs: Arc<Vec<RulePack>>,
    lock_probe: Arc<dyn LockProbe>,
    process_probe: Arc<dyn ProcessProbe>,
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
            process_probe: Arc::new(StubProcessProbe::none()),
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

    /// Why `path` is protected according to the *lexical* list, or `None`.
    ///
    /// The list alone cannot see through a spelling — an 8.3 alias, a junction
    /// or a symlink names a protected place while writing it as somewhere
    /// harmless. Prefer [`Brain::path_refusal`], which resolves the path first
    /// and consults this with the answer.
    pub fn protection_reason(&self, path: &Path) -> Option<String> {
        self.protected
            .reason_for(path)
            .map(|reason| format!("protected location, not cleanable: {reason}"))
    }

    /// Why the hard path gates refuse `path`, or `None` when they do not.
    ///
    /// This is gate 1 in full, and it is deliberately **one** function with two
    /// callers: [`Brain::classify`], which decides the rating the user is shown,
    /// and `core::executor`, which decides whether the move happens.
    ///
    /// They used to run different checks. `classify` consulted only the lexical
    /// [`ProtectedPaths`] list, while the executor also canonicalised and
    /// re-asked — so a path written in a spelling the list does not recognise
    /// could be rated green at analyze time and then refused at execute time.
    /// The delete was blocked, but CLAUDE.md's hard rule is that a protected
    /// path never receives a Safe rating *or* a delete action, and the rating
    /// half was unenforced (security review H9).
    pub fn path_refusal(&self, path: &Path) -> Option<PathRefusal> {
        // The lexical list first: it answers for paths that are not on disk,
        // and it is the defence-in-depth layer for the spellings the gate
        // refuses outright.
        if let Some(reason) = self.protection_reason(path) {
            return Some(PathRefusal {
                reason,
                canonical: None,
            });
        }

        match gate_path(path) {
            // `C:\PROGRA~1\…`, a junction and a symlink all name one place while
            // spelling it as another. Only the resolved form answers that.
            GateVerdict::Local(canonical) => {
                self.protection_reason(&canonical)
                    .map(|reason| PathRefusal {
                        reason,
                        canonical: Some(canonical),
                    })
            }
            // Nothing is there to protect: a walk finds nothing and a move
            // reports the miss honestly.
            GateVerdict::Missing => None,
            GateVerdict::Refused(why) => Some(PathRefusal {
                reason: format!("left alone: {why}"),
                canonical: None,
            }),
        }
    }

    /// Current lock state of `path`.
    pub fn lock_state(&self, path: &Path) -> LockState {
        self.lock_probe.probe(path)
    }

    /// Whether the process a rule names in `owner_process_absent` is running.
    ///
    /// `false` for a rule that names none — "no owner declared" is not the same
    /// question as "the owner is closed", but it produces the same modifier
    /// (none), which is the honest answer either way.
    pub fn owner_process_running(&self, rule: &CompiledRule) -> bool {
        rule.spec
            .match_
            .owner_process_absent
            .as_deref()
            .is_some_and(|name| self.process_probe.is_running(name))
    }

    /// [`Brain::owner_process_running`], asked of the operating system rather
    /// than of the scan's memo.
    ///
    /// The 2-second cache is right for a scan, which wants one consistent answer
    /// across thousands of files in one burst, and wrong for the executor: the
    /// user can sit on the review screen for an hour and open Chrome in the
    /// meantime, and the cache would still be answering the question the scan
    /// asked (review H10).
    pub fn owner_process_running_uncached(&self, rule: &CompiledRule) -> bool {
        rule.spec
            .match_
            .owner_process_absent
            .as_deref()
            .is_some_and(|name| self.process_probe.is_running_uncached(name))
    }

    /// The rule a pack and rule id name, or `None` if no loaded pack has it.
    ///
    /// The executor needs it to re-check the criteria a recommendation was built
    /// on; the modules index rules themselves for their own scans.
    pub fn rule(&self, pack_id: &str, rule_id: &str) -> Option<&CompiledRule> {
        self.packs
            .iter()
            .find(|pack| pack.id() == pack_id)?
            .rules
            .iter()
            .find(|rule| rule.spec.id == rule_id)
    }

    /// Classify a path that a rule already matched, with no metadata.
    ///
    /// Equivalent to [`Brain::classify_with`] and [`Signals::unknown`]: the rule's
    /// base score goes through untouched. Table-driven gate tests use it,
    /// because they pass paths that do not exist and only care about the gates.
    pub fn classify(&self, path: &Path, matched: Option<&CompiledRule>) -> Verdict {
        self.classify_with(path, matched, &Signals::unknown())
    }

    /// The full pipeline: gates, then the rule's base score, then heuristics.
    pub fn classify_with(
        &self,
        path: &Path,
        matched: Option<&CompiledRule>,
        signals: &Signals,
    ) -> Verdict {
        // Gate 1: protected paths beat everything, including a rule that
        // claims otherwise. This is the line CLAUDE.md calls a hard rule — and
        // it is the *same* gate the executor runs, so the rating the user sees
        // and the decision the executor takes can never disagree about one path
        // (security review H9).
        if let Some(refusal) = self.path_refusal(path) {
            return Verdict {
                bucket: RiskBucket::DoNotDelete,
                confidence: GATE_CONFIDENCE,
                action: Action::InfoOnly,
                reason: refusal.reason,
                source: VerdictSource::ProtectedPath,
                rule: matched.map(|rule| (rule.pack_id.clone(), rule.spec.id.clone())),
                scored: None,
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
                scored: None,
            };
        }

        self.classify_by_rule(matched, signals)
    }

    /// Classify a finding.
    ///
    /// Identical to [`Brain::classify`] for anything that is a real path. The
    /// recycle bin is not: `shell:RecycleBinFolder` is a shell namespace
    /// object, so asking whether it is protected or held open is meaningless —
    /// and asking anyway made an empty-able bin look permanently locked. Those
    /// findings go straight to the rule.
    pub fn classify_finding(&self, finding: &Finding, matched: Option<&CompiledRule>) -> Verdict {
        self.classify_finding_with(finding, matched, &Signals::unknown())
    }

    /// [`Brain::classify_finding`] with the heuristic signals filled in.
    pub fn classify_finding_with(
        &self,
        finding: &Finding,
        matched: Option<&CompiledRule>,
        signals: &Signals,
    ) -> Verdict {
        if finding.kind == FindingKind::RecycleBinItem {
            return self.classify_by_rule(matched, signals);
        }
        self.classify_with(&finding.path, matched, signals)
    }

    /// Steps 2, 3 and 4 of the pipeline, with no path gates.
    fn classify_by_rule(&self, matched: Option<&CompiledRule>, signals: &Signals) -> Verdict {
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
                scored: None,
            };
        };

        // Steps 2-4. `score.base` is the starting point, the heuristics move it,
        // the thresholds read it and `max_bucket` caps the answer.
        //
        // The brain asks the process list itself rather than trusting the caller
        // to remember: a caller that forgot would rate a browser cache green
        // with the browser open, which is the exact bug `owner_process_absent`
        // exists to prevent. The caller's own answer is OR-ed in so a pure
        // `score::score` test can still force the signal.
        let signals = Signals {
            owner_process_running: signals.owner_process_running
                || self.owner_process_running(rule),
            ..*signals
        };
        let scored = score::score(rule.spec.score.base, rule.spec.score.max_bucket, &signals);
        let bucket = scored.bucket;
        let action = match (bucket, rule.spec.action) {
            (RiskBucket::DoNotDelete, _) | (_, RuleAction::InfoOnly) => Action::InfoOnly,
            (_, RuleAction::Quarantine | RuleAction::EmptyRecycleBin) => Action::Quarantine,
        };

        Verdict {
            bucket,
            // The score *is* the confidence: one number, computed once. A red
            // verdict is a certainty rather than a low score, so it reports the
            // gate's confidence instead.
            confidence: if bucket == RiskBucket::DoNotDelete {
                GATE_CONFIDENCE
            } else {
                scored.score
            },
            action,
            reason: reason_for(rule, &scored),
            source: VerdictSource::Rule,
            rule: Some((rule.pack_id.clone(), rule.spec.id.clone())),
            scored: Some(scored),
        }
    }
}

/// The user-facing sentence for a scored rule match.
///
/// The rule's own `reason` ships verbatim (`docs/RULES-FORMAT.md`) and is always
/// the whole string when nothing overrode the rule. When a heuristic pushed the
/// item below the bucket the rule asked for, a second sentence says which one —
/// otherwise the user reads "safe to remove" next to a yellow row.
fn reason_for(rule: &CompiledRule, scored: &Scored) -> String {
    if !scored.was_downgraded() {
        return rule.spec.reason.clone();
    }

    let note = if scored.fired(score::OWNER_RUNNING_LABEL) {
        match rule.spec.match_.owner_process_absent.as_deref() {
            Some(process) => format!(
                " {process} is running right now, so this is left for you to decide rather than \
                 treated as safe."
            ),
            None => {
                " The program that owns this data is running, so this needs your review.".to_owned()
            }
        }
    } else if scored.fired("idle:fresh") {
        " Something used this in the last day, so it needs your review first.".to_owned()
    } else {
        format!(
            " Scored {} out of 100 here, which is below the bar for removing it automatically.",
            scored.score
        )
    };

    format!("{}{note}", rule.spec.reason)
}

/// Assembles a [`Brain`]. Everything is injectable so a test never depends on
/// the machine it runs on.
#[derive(Default)]
pub struct BrainBuilder {
    protected: Option<ProtectedPaths>,
    packs: Vec<RulePack>,
    lock_probe: Option<Arc<dyn LockProbe>>,
    process_probe: Option<Arc<dyn ProcessProbe>>,
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

    /// Set the process probe backing `owner_process_absent`.
    #[must_use]
    pub fn process_probe(mut self, probe: Arc<dyn ProcessProbe>) -> Self {
        self.process_probe = Some(probe);
        self
    }

    /// Build it.
    pub fn build(self) -> Brain {
        Brain {
            protected: self.protected.unwrap_or_else(ProtectedPaths::empty),
            packs: Arc::new(self.packs),
            lock_probe: self.lock_probe.unwrap_or_else(|| Arc::new(AlwaysUnlocked)),
            // Nothing running is the default a test wants: a rule with no
            // `owner_process_absent` never asks, and one that does gets a
            // deterministic answer instead of the developer's browser.
            process_probe: self
                .process_probe
                .unwrap_or_else(|| Arc::new(StubProcessProbe::none())),
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
            assert_eq!(verdict.bucket, case.bucket, "{} — {}", case.path, case.why);
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

    /// Security review H9: the *rating* half of CLAUDE.md's hard rule, which
    /// says a protected path can never receive a Safe rating **or** a delete
    /// action.
    ///
    /// `widen` is one-directional. It registers the canonical twin of an entry
    /// written as an 8.3 alias, but never the alias of an entry written in full
    /// — so probing a long-form entry with the short spelling missed:
    ///
    /// ```text
    /// entry=SHORT probe=LONG  -> protected=true    (fixed one review ago)
    /// entry=LONG  probe=SHORT -> protected=false   (this one)
    /// ```
    ///
    /// The delete was still blocked, because the executor canonicalises before
    /// it re-checks. The rating was not — so analyze and execute disagreed about
    /// one path, and the user was shown a green row for a protected file.
    /// `classify` now runs the executor's gate rather than the lexical list.
    #[cfg(windows)]
    #[test]
    fn a_protected_path_probed_by_its_8_3_alias_is_never_rated_safe() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A long name with spaces is what makes Windows mint an alias.
        let long = dir.path().join("Protected Program Files");
        std::fs::create_dir_all(&long).expect("fixture dir");
        let canonical = dunce::canonicalize(&long).expect("canonicalize");
        let victim = canonical.join("kernel.sys");
        std::fs::write(&victim, b"critical").expect("fixture file");

        let Some(short_victim) =
            crate::core::test_support::short_path_name(&victim).filter(|short| short != &victim)
        else {
            crate::core::test_support::privileged_operation_unavailable(
                "a_protected_path_probed_by_its_8_3_alias_is_never_rated_safe",
                "no 8.3 alias is available here (fsutil 8dot3name), so the two spellings \
                 this test compares cannot be produced",
            );
            return;
        };

        let pack = pack();
        let brain = Brain::builder()
            // Registered in its long spelling — the direction `widen` does not
            // cover, because it only ever adds the *canonical* twin.
            .protected_paths(ProtectedPaths::from_trees([canonical.clone()]))
            .packs(vec![pack.clone()])
            .build();

        assert!(
            brain.protection_reason(&short_victim).is_none(),
            "the premise: the lexical list on its own does not see through `{}`",
            short_victim.display()
        );

        // A rule that claims this is safe, as a buggy community pack would.
        let verdict = brain.classify(&short_victim, Some(rule(&pack, "user-temp-files")));
        assert_eq!(
            verdict.bucket,
            RiskBucket::DoNotDelete,
            "`{}` is `{}` spelled differently, and one of them may not be green",
            short_victim.display(),
            victim.display()
        );
        assert_ne!(verdict.bucket, RiskBucket::Safe);
        assert_eq!(verdict.source, VerdictSource::ProtectedPath);
        assert_eq!(verdict.action, Action::InfoOnly);
        assert!(!verdict.is_actionable());
    }

    /// The other half of H9: analyze and execute must reach the same answer,
    /// because they now ask the same question.
    #[test]
    fn the_gate_the_rating_runs_is_the_gate_the_executor_runs() {
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([PathBuf::from("C:\\Windows")]))
            .packs(vec![pack()])
            .build();

        for path in [
            "C:\\Windows\\System32\\ntoskrnl.exe",
            // Unjudgeable spellings are refused by the gate, not waved through.
            "\\\\localhost\\C$\\Windows\\System32\\ntoskrnl.exe",
            "C:Windows\\System32\\x",
            "C:\\Temp\\..\\Windows",
        ] {
            let path = Path::new(path);
            let refusal = brain
                .path_refusal(path)
                .unwrap_or_else(|| panic!("`{}` must be refused", path.display()));
            let verdict = brain.classify(path, None);

            assert_eq!(
                verdict.reason,
                refusal.reason,
                "`{}` must be refused for the same stated reason at both ends",
                path.display()
            );
            assert_eq!(verdict.bucket, RiskBucket::DoNotDelete);
            assert_eq!(verdict.source, VerdictSource::ProtectedPath);
        }
    }

    /// The nine attack spellings, run against every surface that can now judge
    /// a path — including [`Brain::classify`], which until H9 was the one that
    /// could not.
    ///
    /// Each names `C:\Windows\System32` while spelling it as somewhere else.
    /// The allow-list gate refuses all nine as unjudgeable, and refused is not
    /// allowed: the rating is red and no action is offered. Prints the table it
    /// asserts, so `--nocapture` shows the evidence rather than just a pass.
    #[test]
    fn every_attack_spelling_is_refused_by_every_surface() {
        let brain = brain();
        let pack = pack();
        // A rule claiming these are safe, as a buggy community pack would.
        let safe = rule(&pack, "user-temp-files");

        let spellings = [
            (
                "administrative share by host name",
                "\\\\localhost\\C$\\Windows\\System32\\ntoskrnl.exe",
            ),
            (
                "administrative share by loopback",
                "\\\\127.0.0.1\\C$\\Windows\\System32\\ntoskrnl.exe",
            ),
            (
                "verbatim UNC",
                "\\\\?\\UNC\\localhost\\c$\\Windows\\System32\\ntoskrnl.exe",
            ),
            (
                "NT object namespace",
                "\\??\\C:\\Windows\\System32\\ntoskrnl.exe",
            ),
            (
                "device namespace",
                "\\\\.\\C:\\Windows\\System32\\ntoskrnl.exe",
            ),
            (
                "volume by device name",
                "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows\\System32\\x",
            ),
            ("drive-relative", "C:Windows\\System32\\x"),
            ("rooted, no drive", "\\Windows\\System32\\x"),
            ("remote share", "\\\\server\\share\\x"),
        ];

        println!("spelling                           | gate     | list      | refusal  | rating");
        for (label, spelling) in spellings {
            let path = Path::new(spelling);
            let gate = matches!(gate_path(path), GateVerdict::Refused(_));
            let listed = brain.protected_paths().reason_for(path).is_some();
            let refused = brain.path_refusal(path).is_some();
            let verdict = brain.classify(path, Some(safe));

            println!(
                "{label:<34} | {:<8} | {:<9} | {:<8} | {:?}",
                if gate { "REFUSED" } else { "accepted" },
                if listed { "PROTECTED" } else { "unlisted" },
                if refused { "REFUSED" } else { "cleared" },
                verdict.bucket,
            );

            assert!(gate, "{label}: `{spelling}` must be refused by the gate");
            assert!(listed, "{label}: the lexical list must refuse it too");
            assert!(refused, "{label}: `Brain::path_refusal` must refuse it");
            assert_eq!(
                verdict.bucket,
                RiskBucket::DoNotDelete,
                "{label}: `{spelling}` must never be rated green"
            );
            assert_ne!(verdict.bucket, RiskBucket::Safe, "{label}");
            assert_eq!(verdict.action, Action::InfoOnly, "{label}");
            assert!(!verdict.is_actionable(), "{label}");
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
        assert!(
            verdict.reason.contains("in use"),
            "got `{}`",
            verdict.reason
        );
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
        assert!(
            verdict.reason.contains("no rule"),
            "got `{}`",
            verdict.reason
        );
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
        // With no signals the confidence is the rule's own `score.base`.
        assert_eq!(verdict.confidence, 90);
        assert_eq!(verdict.scored.expect("scored").base, 90);
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

    /// The recycle bin is a shell object; running path gates against
    /// `shell:RecycleBinFolder` made a perfectly emptyable bin look locked.
    #[test]
    fn a_recycle_bin_finding_skips_the_path_gates() {
        struct Locked;
        impl LockProbe for Locked {
            fn probe(&self, _path: &Path) -> LockState {
                LockState::Inaccessible
            }
        }

        let pack = pack();
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([PathBuf::from("shell:")]))
            .packs(vec![pack.clone()])
            .lock_probe(Arc::new(Locked))
            .build();

        let mut finding = Finding::new(
            "cleaner",
            "shell:RecycleBinFolder",
            FindingKind::RecycleBinItem,
        );
        finding.size_bytes = 1_024;

        let verdict = brain.classify_finding(&finding, Some(rule(&pack, "user-temp-files")));
        assert_eq!(verdict.bucket, RiskBucket::Safe);
        assert_eq!(verdict.source, VerdictSource::Rule);
    }

    #[test]
    fn a_file_finding_still_runs_every_path_gate() {
        let brain = Brain::builder()
            .protected_paths(ProtectedPaths::from_trees([PathBuf::from("C:\\Windows")]))
            .packs(vec![pack()])
            .build();

        let finding = Finding::new("cleaner", "C:\\Windows\\a.dll", FindingKind::File);
        let verdict = brain.classify_finding(&finding, None);
        assert_eq!(verdict.source, VerdictSource::ProtectedPath);
    }

    /* ---------------------------------------------------------------------
     * The scoring pipeline (phase 2, task 1)
     * ------------------------------------------------------------------ */

    /// A rule pack written on the fly so a case can name any base score and any
    /// ceiling without editing the shipped packs.
    fn scoring_pack(rules: &str) -> RulePack {
        let json = format!(
            r#"{{
                "id": "scoring",
                "name": "Scoring Fixtures",
                "version": 1,
                "module": "cleaner",
                "enabled_by_default": true,
                "description": "Rules that exist to pin the scoring table.",
                "rules": [{rules}]
            }}"#
        );
        compile_pack(
            "scoring",
            &json,
            &MapEnv::new().with("TEMP", "C:\\Users\\dev\\AppData\\Local\\Temp"),
        )
        .expect("the scoring fixture pack must compile")
    }

    /// One rule, parameterised by the two fields the pipeline reads.
    fn scoring_rule(id: &str, base: u8, max_bucket: &str, extra: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "paths": ["%TEMP%\\**"],
                "score": {{ "base": {base}, "max_bucket": "{max_bucket}" }},
                "action": "{}",
                "reason": "Fixture rule {id}."{extra}
            }}"#,
            if max_bucket == "do_not_delete" {
                "info_only"
            } else {
                "quarantine"
            }
        )
    }

    /// **The scoring table.** Path + rule + signals → bucket, score, source.
    ///
    /// Every modifier in `brain::score` and every gate in `Brain::classify_with`
    /// appears at least once, including the four the acceptance criteria call
    /// out by name: a high base score held down by `max_bucket`, a protected
    /// path with a Safe rule, a locked file, and an old file against a fresh one
    /// across a threshold.
    ///
    /// Prints the table it asserts, so `--nocapture` shows the evidence rather
    /// than just a pass.
    #[test]
    fn the_scoring_table() {
        struct Locked;
        impl LockProbe for Locked {
            fn probe(&self, _path: &Path) -> LockState {
                LockState::InUse
            }
        }

        const TEMP: &str = "C:\\Users\\dev\\AppData\\Local\\Temp";
        const BIG: u64 = 200 * 1024 * 1024;

        let pack = scoring_pack(
            &[
                scoring_rule("green-90", 90, "safe", ""),
                scoring_rule("green-86", 86, "safe", ""),
                scoring_rule("edge-85", 85, "safe", ""),
                scoring_rule("edge-84", 84, "safe", ""),
                scoring_rule("borderline-80", 80, "safe", ""),
                scoring_rule("yellow-50", 50, "needs_review", ""),
                scoring_rule("edge-40", 40, "safe", ""),
                scoring_rule("edge-39", 39, "safe", ""),
                scoring_rule("capped-100", 100, "needs_review", ""),
                scoring_rule("informational", 95, "do_not_delete", ""),
                scoring_rule(
                    "browser-92",
                    92,
                    "safe",
                    r#", "match": { "owner_process_absent": "chrome.exe" }"#,
                ),
                scoring_rule(
                    "browser-100",
                    100,
                    "safe",
                    r#", "match": { "owner_process_absent": "chrome.exe" }"#,
                ),
                scoring_rule(
                    "downloads-45",
                    45,
                    "needs_review",
                    r#", "match": { "min_age_hours": 4320 }"#,
                ),
            ]
            .join(","),
        );

        let env = MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("ProgramFiles", "C:\\Program Files")
            .with("USERPROFILE", "C:\\Users\\dev");
        let protected =
            ProtectedPaths::from_env(&env, Path::new("C:\\app"), Path::new("C:\\app\\quarantine"));

        let brain = |locked: bool, chrome: bool| {
            let mut builder = Brain::builder()
                .protected_paths(protected.clone())
                .packs(vec![pack.clone()])
                .process_probe(Arc::new(if chrome {
                    StubProcessProbe::running(["chrome.exe"])
                } else {
                    StubProcessProbe::none()
                }));
            if locked {
                builder = builder.lock_probe(Arc::new(Locked));
            }
            builder.build()
        };

        struct Case<'a> {
            what: &'a str,
            path: &'a str,
            rule: Option<&'a str>,
            signals: Signals,
            locked: bool,
            chrome: bool,
            bucket: RiskBucket,
            score: Option<u8>,
            source: VerdictSource,
        }

        let case = |what, path, rule: Option<&'static str>, bucket, score, source| Case {
            what,
            path,
            rule,
            signals: Signals::unknown(),
            locked: false,
            chrome: false,
            bucket,
            score,
            source,
        };
        let idle = |days: i64| Signals {
            idle_days: Some(days),
            written_days: Some(days),
            ..Signals::unknown()
        };

        let cases = vec![
            /* --- step 4: bare thresholds, no modifiers ------------------- */
            case(
                "base 90 clears the green threshold",
                "a.tmp",
                Some("green-90"),
                RiskBucket::Safe,
                Some(90),
                VerdictSource::Rule,
            ),
            case(
                "85 is green: the threshold is inclusive",
                "b.tmp",
                Some("edge-85"),
                RiskBucket::Safe,
                Some(85),
                VerdictSource::Rule,
            ),
            case(
                "84 is one point short and lands yellow",
                "c.tmp",
                Some("edge-84"),
                RiskBucket::NeedsReview,
                Some(84),
                VerdictSource::Rule,
            ),
            case(
                "40 is yellow: the lower threshold is inclusive too",
                "d.tmp",
                Some("edge-40"),
                RiskBucket::NeedsReview,
                Some(40),
                VerdictSource::Rule,
            ),
            case(
                "39 is below the bar and lands red",
                "e.tmp",
                Some("edge-39"),
                RiskBucket::DoNotDelete,
                None,
                VerdictSource::Rule,
            ),
            case(
                "a mid-range base score is yellow",
                "f.tmp",
                Some("yellow-50"),
                RiskBucket::NeedsReview,
                Some(50),
                VerdictSource::Rule,
            ),
            /* --- step 4: max_bucket is a ceiling, never a floor ---------- */
            Case {
                what: "a base of 100 held down by max_bucket: needs_review",
                score: Some(100),
                bucket: RiskBucket::NeedsReview,
                ..case(
                    "",
                    "g.tmp",
                    Some("capped-100"),
                    RiskBucket::NeedsReview,
                    Some(100),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "the same rule, with every bonus firing, still cannot go green",
                signals: Signals {
                    idle_days: Some(400),
                    written_days: Some(400),
                    size_bytes: BIG,
                    in_known_cache_dir: true,
                    owner_process_running: false,
                },
                ..case(
                    "",
                    "h.tmp",
                    Some("capped-100"),
                    RiskBucket::NeedsReview,
                    Some(100),
                    VerdictSource::Rule,
                )
            },
            case(
                "max_bucket: do_not_delete makes a base of 95 informational",
                "i.tmp",
                Some("informational"),
                RiskBucket::DoNotDelete,
                None,
                VerdictSource::Rule,
            ),
            /* --- step 3: idle age, both directions ----------------------- */
            Case {
                what: "an old file crosses up over the green threshold",
                signals: idle(400),
                score: Some(90),
                ..case(
                    "",
                    "j.tmp",
                    Some("borderline-80"),
                    RiskBucket::Safe,
                    Some(90),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "the same rule on a fresh file crosses down out of green",
                signals: idle(0),
                score: Some(65),
                ..case(
                    "",
                    "k.tmp",
                    Some("borderline-80"),
                    RiskBucket::NeedsReview,
                    Some(65),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "cold (30-179 days) adds five",
                signals: idle(60),
                score: Some(85),
                ..case(
                    "",
                    "l.tmp",
                    Some("borderline-80"),
                    RiskBucket::Safe,
                    Some(85),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "warm (7-29 days) adds two and is not enough on its own",
                signals: idle(10),
                score: Some(82),
                ..case(
                    "",
                    "m.tmp",
                    Some("borderline-80"),
                    RiskBucket::NeedsReview,
                    Some(82),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "between fresh and warm nothing fires at all",
                signals: idle(3),
                score: Some(80),
                ..case(
                    "",
                    "n.tmp",
                    Some("borderline-80"),
                    RiskBucket::NeedsReview,
                    Some(80),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "a fresh file can be pushed all the way to red",
                signals: idle(0),
                score: None,
                ..case(
                    "",
                    "o.tmp",
                    Some("edge-40"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::Rule,
                )
            },
            /* --- step 3: known cache directory --------------------------- */
            Case {
                what: "sitting in a cache directory adds five",
                signals: Signals {
                    in_known_cache_dir: true,
                    ..Signals::unknown()
                },
                score: Some(85),
                ..case(
                    "",
                    "Cache\\p.tmp",
                    Some("borderline-80"),
                    RiskBucket::Safe,
                    Some(85),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "age and the cache directory stack",
                signals: Signals {
                    idle_days: Some(400),
                    written_days: Some(400),
                    in_known_cache_dir: true,
                    ..Signals::unknown()
                },
                score: Some(95),
                ..case(
                    "",
                    "Cache\\q.tmp",
                    Some("borderline-80"),
                    RiskBucket::Safe,
                    Some(95),
                    VerdictSource::Rule,
                )
            },
            /* --- step 3: size, review candidates only -------------------- */
            Case {
                what: "a large green candidate gets no size bonus",
                signals: Signals {
                    size_bytes: BIG,
                    ..Signals::unknown()
                },
                score: Some(90),
                ..case(
                    "",
                    "r.tmp",
                    Some("green-90"),
                    RiskBucket::Safe,
                    Some(90),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "a large review candidate does",
                signals: Signals {
                    size_bytes: BIG,
                    ..Signals::unknown()
                },
                score: Some(54),
                ..case(
                    "",
                    "s.tmp",
                    Some("yellow-50"),
                    RiskBucket::NeedsReview,
                    Some(54),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "a small review candidate does not",
                signals: Signals {
                    size_bytes: 1_024,
                    ..Signals::unknown()
                },
                score: Some(50),
                ..case(
                    "",
                    "t.tmp",
                    Some("yellow-50"),
                    RiskBucket::NeedsReview,
                    Some(50),
                    VerdictSource::Rule,
                )
            },
            /* --- step 3: the owning process ------------------------------ */
            case(
                "a browser cache with the browser closed is green",
                "Cache\\u.tmp",
                Some("browser-92"),
                RiskBucket::Safe,
                Some(92),
                VerdictSource::Rule,
            ),
            Case {
                what: "the same rule with the browser open drops to review",
                chrome: true,
                score: Some(67),
                ..case(
                    "",
                    "Cache\\v.tmp",
                    Some("browser-92"),
                    RiskBucket::NeedsReview,
                    Some(67),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "the lowered ceiling, not the score, is what holds it at review",
                chrome: true,
                signals: Signals {
                    idle_days: Some(400),
                    written_days: Some(400),
                    in_known_cache_dir: true,
                    ..Signals::unknown()
                },
                score: Some(90),
                ..case(
                    "",
                    "Cache\\w.tmp",
                    Some("browser-100"),
                    RiskBucket::NeedsReview,
                    Some(90),
                    VerdictSource::Rule,
                )
            },
            Case {
                what: "a rule with no owner_process_absent ignores the process list",
                chrome: true,
                ..case(
                    "",
                    "x.tmp",
                    Some("green-90"),
                    RiskBucket::Safe,
                    Some(90),
                    VerdictSource::Rule,
                )
            },
            /* --- step 3: old downloads are never green ------------------- */
            Case {
                what: "a 400-day-old download still cannot climb past needs_review",
                signals: Signals {
                    idle_days: Some(400),
                    written_days: Some(400),
                    size_bytes: BIG,
                    ..Signals::unknown()
                },
                score: Some(59),
                ..case(
                    "",
                    "y.zip",
                    Some("downloads-45"),
                    RiskBucket::NeedsReview,
                    Some(59),
                    VerdictSource::Rule,
                )
            },
            /* --- step 1: the hard gates beat every score ----------------- */
            Case {
                what: "a protected path with a Safe rule and every bonus is still red",
                path: "C:\\Windows\\System32\\ntoskrnl.exe",
                signals: Signals {
                    idle_days: Some(400),
                    written_days: Some(400),
                    in_known_cache_dir: true,
                    ..Signals::unknown()
                },
                ..case(
                    "",
                    "",
                    Some("green-90"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::ProtectedPath,
                )
            },
            Case {
                what: "an installed-programs directory, same story",
                path: "C:\\Program Files\\App\\app.exe",
                ..case(
                    "",
                    "",
                    Some("capped-100"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::ProtectedPath,
                )
            },
            Case {
                what: "our own quarantine is never scored",
                path: "C:\\app\\quarantine\\batch\\item",
                ..case(
                    "",
                    "",
                    Some("green-90"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::ProtectedPath,
                )
            },
            Case {
                what: "an unjudgeable spelling is refused before scoring runs",
                path: "\\\\localhost\\C$\\Windows\\System32\\x",
                ..case(
                    "",
                    "",
                    Some("green-90"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::ProtectedPath,
                )
            },
            Case {
                what: "a locked file beats a base score of 100",
                locked: true,
                signals: idle(400),
                ..case(
                    "",
                    "z.tmp",
                    Some("capped-100"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::Locked,
                )
            },
            Case {
                what: "a locked file beats a green rule too",
                locked: true,
                ..case(
                    "",
                    "z2.tmp",
                    Some("green-90"),
                    RiskBucket::DoNotDelete,
                    None,
                    VerdictSource::Locked,
                )
            },
            case(
                "no rule at all means no suggestion, whatever the path",
                "unmatched.bin",
                None,
                RiskBucket::DoNotDelete,
                None,
                VerdictSource::Unknown,
            ),
        ];

        assert!(
            cases.len() >= 25,
            "the brief asks for at least 25 cases; this table has {}",
            cases.len()
        );

        println!(
            "{:<62} | {:<7} | {:<12} | source",
            "case", "score", "bucket"
        );
        for case in &cases {
            let path = if case.path.starts_with("C:\\") || case.path.starts_with("\\\\") {
                case.path.to_owned()
            } else {
                format!("{TEMP}\\{}", case.path)
            };
            let matched = case.rule.map(|id| {
                pack.rules
                    .iter()
                    .find(|r| r.spec.id == id)
                    .unwrap_or_else(|| panic!("fixture rule `{id}` exists"))
            });

            let brain = brain(case.locked, case.chrome);
            let verdict = brain.classify_with(Path::new(&path), matched, &case.signals);

            println!(
                "{:<62} | {:<7} | {:<12} | {:?}",
                case.what,
                verdict
                    .scored
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), |s| s.score.to_string()),
                format!("{:?}", verdict.bucket),
                verdict.source,
            );

            assert_eq!(verdict.bucket, case.bucket, "bucket for `{}`", case.what);
            assert_eq!(verdict.source, case.source, "source for `{}`", case.what);
            if let Some(expected) = case.score {
                let scored = verdict
                    .scored
                    .as_ref()
                    .unwrap_or_else(|| panic!("`{}` should have been scored", case.what));
                assert_eq!(scored.score, expected, "score for `{}`", case.what);
                assert_eq!(
                    verdict.confidence, expected,
                    "confidence must be the score for `{}`",
                    case.what
                );
            }
            assert!(
                !verdict.reason.trim().is_empty(),
                "`{}` produced a verdict with no reason",
                case.what
            );
            // The invariant no case may violate.
            if verdict.bucket == RiskBucket::DoNotDelete {
                assert_eq!(verdict.action, Action::InfoOnly, "`{}`", case.what);
                assert!(!verdict.is_actionable(), "`{}`", case.what);
            }
        }
    }

    /// The acceptance criterion, as its own test: **changing a rule's base score
    /// changes the fixture buckets predictably.**
    ///
    /// One path, one set of signals, one field varied — and the bucket walks
    /// down through the thresholds in exactly the order the architecture states.
    #[test]
    fn changing_a_rules_base_score_moves_its_bucket_predictably() {
        let path = Path::new("C:\\Users\\dev\\AppData\\Local\\Temp\\subject.tmp");

        let expectations: &[(u8, RiskBucket)] = &[
            (100, RiskBucket::Safe),
            (90, RiskBucket::Safe),
            (86, RiskBucket::Safe),
            (85, RiskBucket::Safe),
            (84, RiskBucket::NeedsReview),
            (60, RiskBucket::NeedsReview),
            (41, RiskBucket::NeedsReview),
            (40, RiskBucket::NeedsReview),
            (39, RiskBucket::DoNotDelete),
            (10, RiskBucket::DoNotDelete),
            (0, RiskBucket::DoNotDelete),
        ];

        println!("base | bucket       | action");
        let mut seen = Vec::new();
        for (base, expected) in expectations {
            // Only `score.base` differs between iterations.
            let pack = scoring_pack(&scoring_rule("subject", *base, "safe", ""));
            let brain = Brain::builder().packs(vec![pack.clone()]).build();
            let verdict = brain.classify(path, Some(&pack.rules[0]));

            println!("{base:>4} | {:<12?} | {:?}", verdict.bucket, verdict.action);
            assert_eq!(
                verdict.bucket, *expected,
                "base {base} must produce {expected:?}"
            );
            seen.push(verdict.bucket);
        }

        // Monotonic: a lower base score can never produce a *better* bucket.
        assert!(
            seen.windows(2).all(|pair| pair[0] <= pair[1]),
            "lowering the base score must never improve the bucket: {seen:?}"
        );

        // And the same one-field change is visible end to end, through the
        // cleaner's own fixture path rather than only through `classify`.
        for (base, expected) in [(90u8, RiskBucket::Safe), (50, RiskBucket::NeedsReview)] {
            let pack = scoring_pack(&scoring_rule("subject", base, "safe", ""));
            let brain = Brain::builder().packs(vec![pack.clone()]).build();
            let mut finding = Finding::new("cleaner", path, FindingKind::File);
            finding.size_bytes = 4_096;
            let verdict = brain.classify_finding(&finding, Some(&pack.rules[0]));
            assert_eq!(verdict.bucket, expected, "base {base} through a finding");
        }
    }

    /// A downgraded verdict has to say *why*, or the user reads "safe to remove"
    /// next to a yellow row.
    #[test]
    fn a_downgrade_explains_itself_and_an_untouched_rule_stays_verbatim() {
        let pack = scoring_pack(&scoring_rule(
            "browser",
            92,
            "safe",
            r#", "match": { "owner_process_absent": "chrome.exe" }"#,
        ));
        let rule = &pack.rules[0];
        let path = Path::new("C:\\Users\\dev\\AppData\\Local\\Temp\\Cache\\a.tmp");

        let closed = Brain::builder().packs(vec![pack.clone()]).build();
        assert_eq!(
            closed.classify(path, Some(rule)).reason,
            "Fixture rule browser.",
            "an untouched rule ships its reason verbatim"
        );

        let open = Brain::builder()
            .packs(vec![pack.clone()])
            .process_probe(Arc::new(StubProcessProbe::running(["chrome.exe"])))
            .build();
        let verdict = open.classify_with(
            path,
            Some(rule),
            &Signals {
                owner_process_running: true,
                ..Signals::unknown()
            },
        );
        assert_eq!(verdict.bucket, RiskBucket::NeedsReview);
        assert!(
            verdict.reason.starts_with("Fixture rule browser."),
            "the rule's own reason stays the start of the sentence: `{}`",
            verdict.reason
        );
        assert!(
            verdict.reason.contains("chrome.exe"),
            "the downgrade must name the process: `{}`",
            verdict.reason
        );
    }

    /// `owner_process_absent` was parsed and validated in phase 1 and never
    /// acted on. This is the wiring, asked of the brain rather than of `score`.
    #[test]
    fn the_brain_asks_the_process_list_only_when_a_rule_names_a_process() {
        let with_owner = scoring_pack(&scoring_rule(
            "browser",
            92,
            "safe",
            r#", "match": { "owner_process_absent": "chrome.exe" }"#,
        ));
        let without = scoring_pack(&scoring_rule("plain", 92, "safe", ""));

        let brain = Brain::builder()
            .process_probe(Arc::new(StubProcessProbe::running(["chrome.exe"])))
            .build();

        assert!(brain.owner_process_running(&with_owner.rules[0]));
        assert!(
            !brain.owner_process_running(&without.rules[0]),
            "a rule that names no process must never be downgraded by one"
        );

        let quiet = Brain::builder()
            .process_probe(Arc::new(StubProcessProbe::none()))
            .build();
        assert!(!quiet.owner_process_running(&with_owner.rules[0]));
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
