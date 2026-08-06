//! Cleaner: find reclaimable junk, sort it into three risk buckets, clean via
//! quarantine (`docs/modules/02-cleaner-core.md`).
//!
//! **Phase 1 scope: the green packs only** — `windows-temp`, `recycle-bin`,
//! `thumbnail-icon-cache`. The yellow and red buckets are wired end to end and
//! render, but nothing targets yellow yet, and red is *produced, not targeted*:
//! it holds whatever the walk surfaced that the hard gates refused. That is the
//! honesty feature, not an empty placeholder.
//!
//! Two safety rules live here rather than in the rule packs, because a pack is
//! community-editable and these are not negotiable:
//!
//! - Files modified in the last hour are skipped even in a Safe pack. Something
//!   is probably still writing them.
//! - Only files are ever proposed. Directories are walked but never quarantined
//!   in phase 1, so a clean can never take a folder structure with it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::core::brain::{
    self, Brain, CompiledRule, GateVerdict, ProtectedPaths, RuleAction, RulePack,
};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::platform::RecycleBin;
use crate::core::scanner::{self, CancellationToken, WalkEntry};
use crate::core::types::{Action, Finding, FindingKind, Recommendation, RiskBucket};

/// Files touched more recently than this are left alone whatever a rule says:
/// something is probably still writing them.
pub const MIN_QUIET_HOURS: i64 = 1;

/// Pack whose findings come from the shell API rather than a walk.
pub const RECYCLE_BIN_PACK: &str = "recycle-bin";

/// Sentinel path for the recycle bin. Not a real filesystem path — the shell
/// namespace is how Explorer refers to it too.
pub const RECYCLE_BIN_PATH: &str = "shell:RecycleBinFolder";

/// Metadata attached to every finding, so the UI can group by pack and the
/// review screen can show which rule spoke.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingMeta {
    /// Pack that matched.
    pub pack_id: String,
    /// Display name of the pack, for the group header.
    pub pack_name: String,
    /// Rule that matched.
    pub rule_id: String,
}

/// What a scan produced, before the brain has an opinion.
#[derive(Debug, Clone, Default)]
pub struct ScanOutput {
    /// The findings.
    pub findings: Vec<Finding>,
    /// Rules that could not run, as `pack/rule: VARIABLE`.
    pub skipped_rules: Vec<String>,
    /// Directories the walker could not read.
    pub unreadable: Vec<PathBuf>,
    /// Whether the walk was cancelled part-way.
    pub cancelled: bool,
}

/// Everything the cleaner needs that is not a rule pack.
pub struct CleanerContext<'a> {
    /// Packs to run, already compiled and expanded.
    pub packs: &'a [RulePack],
    /// Hard gates, consulted before a walk root is walked rather than only
    /// afterwards: a root that resolves into a protected location is never
    /// entered at all (security review C1d).
    pub protected: &'a ProtectedPaths,
    /// The recycle bin, read-only during a scan.
    pub recycle_bin: &'a dyn RecycleBin,
    /// Reference instant for every age comparison, so a scan is reproducible.
    pub now: DateTime<Utc>,
    /// Cancellation.
    pub cancel: &'a CancellationToken,
}

/// Progress callback: the running item count and the path being looked at.
/// Throttling is the caller's job, so a test can observe every tick.
pub type ScanProgressSink<'a> = &'a mut dyn FnMut(u64, &Path);

/// Walk the enabled packs and collect findings. Read-only.
///
/// `progress` is called with the running item count and the current path; the
/// caller throttles it.
pub fn scan(ctx: &CleanerContext<'_>, progress: Option<ScanProgressSink<'_>>) -> ScanOutput {
    let mut output = ScanOutput::default();
    let mut progress = progress;
    let mut seen = 0u64;

    for pack in ctx.packs {
        for (rule_id, variable) in &pack.skipped {
            output
                .skipped_rules
                .push(format!("{}/{rule_id}: %{variable}%", pack.id()));
        }

        if pack.id() == RECYCLE_BIN_PACK {
            if let Some(finding) = recycle_bin_finding(pack, ctx) {
                output.findings.push(finding);
            }
            continue;
        }

        // Each rule names its own walk roots, so a scan only touches the
        // directories the packs actually point at — and only after every root
        // has been resolved and cleared against the hard gates.
        let (groups, refused) = plan_walks(pack, ctx.protected);
        output.skipped_rules.extend(refused);

        for group in &groups {
            let summary = scanner::walk(&group.root, ctx.cancel, &mut |entry| {
                seen += 1;
                if let Some(sink) = progress.as_deref_mut() {
                    sink(seen, &entry.path);
                }
                // Rules sharing a root are tried in declaration order and the
                // first one to claim an entry wins, which is what the walk-per-
                // rule loop plus `dedupe_by_path` used to add up to.
                for rule in &group.rules {
                    if let Some(finding) = finding_for(entry, rule, pack, ctx.now) {
                        output.findings.push(finding);
                        break;
                    }
                }
            });
            output.unreadable.extend(summary.unreadable);
            output.cancelled |= summary.cancelled;
            if output.cancelled {
                return output;
            }
        }
    }

    // One path can match rules in two packs; the first pack to claim it wins,
    // so a file is never offered twice.
    dedupe_by_path(&mut output.findings);
    output
}

/// One directory to walk and the rules that asked for it.
struct WalkGroup<'a> {
    /// The root exactly as the rule spelled it. The canonical form decides
    /// *whether* to walk; the spelled form decides *what* to walk, because the
    /// globs were compiled against that spelling and would stop matching if a
    /// junction rewrote it.
    root: PathBuf,
    /// Rules whose roots resolve to this same directory, in declaration order.
    rules: Vec<&'a CompiledRule>,
}

/// What resolving a walk root concluded.
enum RootVerdict {
    /// Walk it. Carries the identity used to fold duplicate roots together.
    Walk(String),
    /// Do not walk it, and say so in `skipped_rules`.
    Refused(String),
}

/// Group a pack's walk roots, dropping the ones that must not be walked and
/// folding the ones that name the same directory twice (security review C1d,
/// L5). `%TEMP%` and `%LOCALAPPDATA%\Temp` are the same tree on a stock
/// install, and walking it twice doubles the scan for nothing.
fn plan_walks<'a>(pack: &'a RulePack, protected: &ProtectedPaths) -> (Vec<WalkGroup<'a>>, Vec<String>) {
    let mut groups: Vec<WalkGroup<'a>> = Vec::new();
    let mut by_identity: HashMap<String, usize> = HashMap::new();
    let mut refused = Vec::new();

    for rule in &pack.rules {
        for root in rule.roots() {
            let identity = match resolve_root(root, protected) {
                RootVerdict::Walk(identity) => identity,
                RootVerdict::Refused(why) => {
                    tracing::warn!(
                        pack = pack.id(),
                        rule = rule.spec.id.as_str(),
                        root = %root.display(),
                        "walk root refused"
                    );
                    refused.push(format!("{}/{}: {why}", pack.id(), rule.spec.id));
                    continue;
                }
            };

            match by_identity.get(&identity) {
                Some(&index) => groups[index].rules.push(rule),
                None => {
                    by_identity.insert(identity, groups.len());
                    groups.push(WalkGroup {
                        root: root.clone(),
                        rules: vec![rule],
                    });
                }
            }
        }
    }

    (groups, refused)
}

/// Decide whether a walk root may be walked, and what identifies it.
fn resolve_root(root: &Path, protected: &ProtectedPaths) -> RootVerdict {
    if let Some(reason) = protected.reason_for(root) {
        return RootVerdict::Refused(format!(
            "`{}` was not walked because it is a protected location ({reason})",
            root.display()
        ));
    }

    // The one chokepoint. Every shape that is not a plain local drive-letter
    // path is refused here rather than walked on the strength of a lexical list
    // that cannot compare it with anything (security review C2b).
    match brain::gate_path(root) {
        GateVerdict::Local(canonical) => {
            // A junction, a reparse point or an 8.3 alias can aim a harmless-
            // looking root straight at `C:\Windows`. Only the resolved form
            // answers that question.
            if let Some(reason) = protected.reason_for(&canonical) {
                return RootVerdict::Refused(format!(
                    "`{}` was not walked because it resolves to `{}`, a protected location ({reason})",
                    root.display(),
                    canonical.display()
                ));
            }
            RootVerdict::Walk(canonical.to_string_lossy().to_lowercase())
        }
        // A rule pointing at a folder this machine does not have is not an
        // error; the walk simply finds nothing.
        GateVerdict::Missing => RootVerdict::Walk(root.to_string_lossy().to_lowercase()),
        GateVerdict::Refused(why) => {
            RootVerdict::Refused(format!("it was not walked because {why}"))
        }
    }
}

/// Decide whether one walked entry becomes a finding for one rule.
fn finding_for(
    entry: &WalkEntry,
    rule: &CompiledRule,
    pack: &RulePack,
    now: DateTime<Utc>,
) -> Option<Finding> {
    // Phase 1 never proposes directories; see the module header.
    if entry.is_dir {
        return None;
    }
    if !rule.matches_path(&entry.path) {
        return None;
    }

    let age_hours = entry.age_hours(now);

    // The non-negotiable quiet period, applied before any rule's own age rule.
    if age_hours < MIN_QUIET_HOURS {
        return None;
    }
    if let Some(min_age) = rule.spec.match_.min_age_hours {
        if age_hours < i64::from(min_age) {
            return None;
        }
    }
    if let Some(min_size) = rule.spec.match_.min_size_bytes {
        if entry.size_bytes < min_size {
            return None;
        }
    }
    if let Some(extensions) = &rule.spec.match_.extensions {
        let actual = entry
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        let allowed = actual.is_some_and(|actual| {
            extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(&actual))
        });
        if !allowed {
            return None;
        }
    }

    let mut finding = Finding::new("cleaner", entry.path.clone(), FindingKind::File);
    finding.size_bytes = entry.size_bytes;
    finding.created = entry.created;
    finding.modified = entry.modified;
    finding.accessed = entry.accessed;
    finding.meta = serde_json::to_value(FindingMeta {
        pack_id: pack.id().to_owned(),
        pack_name: pack.spec.name.clone(),
        rule_id: rule.spec.id.clone(),
    })
    .unwrap_or(serde_json::Value::Null);
    Some(finding)
}

/// The recycle bin as a single finding, sized through the shell API.
fn recycle_bin_finding(pack: &RulePack, ctx: &CleanerContext<'_>) -> Option<Finding> {
    let rule = pack.rules.first()?;
    let stats = match ctx.recycle_bin.query() {
        Ok(stats) => stats,
        Err(err) => {
            tracing::warn!(error = %err, "could not read the recycle bin");
            return None;
        }
    };
    if stats.is_empty() {
        return None;
    }

    let mut finding = Finding::new("cleaner", RECYCLE_BIN_PATH, FindingKind::RecycleBinItem);
    finding.size_bytes = stats.size_bytes;
    finding.meta = serde_json::json!({
        "packId": pack.id(),
        "packName": pack.spec.name,
        "ruleId": rule.spec.id,
        "itemCount": stats.item_count,
    });
    Some(finding)
}

/// Turn findings into scored, explained recommendations. Read-only.
///
/// One recommendation per finding: the review screen selects item by item, and
/// grouping is a UI concern driven by the finding's pack metadata.
pub fn analyze(findings: &[Finding], brain: &Brain) -> Vec<Recommendation> {
    let rules_by_id = index_rules(brain);
    let mut recommendations = Vec::with_capacity(findings.len());

    for finding in findings {
        let matched = rule_key(finding).and_then(|key| rules_by_id.get(&key).copied());
        let verdict = brain.classify_finding(finding, matched);

        // The red bucket carries no action, and `Recommendation::new` refuses
        // any other combination — that invariant is the last line of defence.
        let action = if verdict.bucket.is_actionable() {
            verdict.action
        } else {
            Action::InfoOnly
        };
        let savings = if action.is_mutating() {
            finding.size_bytes
        } else {
            0
        };

        match Recommendation::new(
            vec![finding.id],
            verdict.bucket,
            verdict.confidence,
            action,
            verdict.reason,
            savings,
        ) {
            Ok(rec) => recommendations.push(rec),
            Err(err) => tracing::error!(
                path = %finding.path.display(),
                error = %err,
                "dropped a recommendation that failed its own invariants"
            ),
        }
    }

    recommendations
}

fn index_rules(brain: &Brain) -> BTreeMap<(String, String), &CompiledRule> {
    brain
        .packs()
        .iter()
        .flat_map(|pack| {
            pack.rules
                .iter()
                .map(move |rule| ((pack.id().to_owned(), rule.spec.id.clone()), rule))
        })
        .collect()
}

fn rule_key(finding: &Finding) -> Option<(String, String)> {
    let pack = finding.meta.get("packId")?.as_str()?.to_owned();
    let rule = finding.meta.get("ruleId")?.as_str()?.to_owned();
    Some((pack, rule))
}

/// Keep the first finding for each path.
fn dedupe_by_path(findings: &mut Vec<Finding>) {
    let mut seen = std::collections::HashSet::new();
    findings.retain(|finding| seen.insert(finding.path.to_string_lossy().to_lowercase()));
}

/// Summary numbers for the header line after a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanerSummary {
    /// Bytes in the green bucket.
    pub safe_bytes: u64,
    /// Bytes in the yellow bucket.
    pub review_bytes: u64,
    /// Items in the red bucket, which has no bytes to offer.
    pub blocked_items: u64,
}

/// Compute the summary from a set of recommendations.
pub fn summarise(recommendations: &[Recommendation]) -> CleanerSummary {
    let mut summary = CleanerSummary::default();
    for rec in recommendations {
        match rec.risk {
            RiskBucket::Safe => summary.safe_bytes += rec.savings_bytes,
            RiskBucket::NeedsReview => summary.review_bytes += rec.savings_bytes,
            RiskBucket::DoNotDelete => summary.blocked_items += 1,
        }
    }
    summary
}

/// Registry entry.
///
/// The real work is driven by the commands in `commands.rs`, which own the
/// brain, the recycle-bin wrapper and the quarantine; the trait methods here
/// exist so the module has registry presence and a manifest.
#[derive(Debug, Default)]
pub struct CleanerModule;

impl ToolModule for CleanerModule {
    fn id(&self) -> &'static str {
        "cleaner"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Disk Cleaner".to_owned(),
            description:
                "Find temporary files, caches and recycle-bin leftovers, sorted by how safe they are to remove."
                    .to_owned(),
            icon: "broom".to_owned(),
            capabilities: ModuleCapabilities::FULL,
        }
    }

    fn analyze(&self, findings: &[Finding], brain: &Brain) -> Vec<Recommendation> {
        analyze(findings, brain)
    }
}

/// Which rule action a pack declares for its first rule. Used by the confirm
/// dialog to label the recycle bin separately.
pub fn is_recycle_bin_finding(finding: &Finding) -> bool {
    finding.kind == FindingKind::RecycleBinItem
}

/// Look a finding up by id.
pub fn find(findings: &[Finding], id: Uuid) -> Option<&Finding> {
    findings.iter().find(|finding| finding.id == id)
}

/// True when a rule proposes emptying the recycle bin.
pub fn empties_recycle_bin(rule: &CompiledRule) -> bool {
    rule.spec.action == RuleAction::EmptyRecycleBin
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::{rules, ProtectedPaths};
    use crate::core::platform::{AlwaysUnlocked, MapEnv, StubRecycleBin};
    use std::fs;
    use std::sync::Arc;
    use std::time::SystemTime;

    /// A miniature dirty Windows profile, built in a tempdir. `%TEMP%` and
    /// `%LOCALAPPDATA%` are pointed at it, so the shipped rule packs are the
    /// ones under test and the real machine is never touched.
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        env: MapEnv,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().to_path_buf();

            let temp = root.join("AppData/Local/Temp");
            let explorer = root.join("AppData/Local/Microsoft/Windows/Explorer");
            let windows = root.join("Windows");
            let downloads = root.join("Downloads");
            for path in [&temp, &explorer, &windows, &downloads] {
                fs::create_dir_all(path).expect("fixture dirs");
            }

            // Green: old temp files.
            aged(&temp.join("old-installer.tmp"), b"0123456789", 48);
            aged(&temp.join("nested/deep.tmp"), b"deep", 72);
            // Excluded by the pack's own `exclude` pattern.
            aged(&temp.join("session.lock"), b"lock", 48);
            // Too new: the quiet-period rule and the pack's 24h rule.
            aged(&temp.join("being-written.tmp"), b"live", 0);
            aged(&temp.join("yesterdayish.tmp"), b"recent", 12);
            // Green: the icon cache.
            aged(&explorer.join("iconcache_32.db"), b"icon-cache", 48);
            aged(&explorer.join("notes.txt"), b"not a db", 48);
            // Red by injection: a `C:\Windows` stand-in inside the fixture.
            aged(&windows.join("System32/critical.sys"), b"do not touch", 200);
            // Untouched by any pack.
            aged(&downloads.join("keep-me.zip"), b"user data", 500);

            let env = MapEnv::new()
                .with("TEMP", temp.to_string_lossy())
                .with("LOCALAPPDATA", root.join("AppData/Local").to_string_lossy());

            Self {
                root,
                env,
                _dir: dir,
            }
        }

        fn packs(&self) -> Vec<RulePack> {
            rules::load_dir(&repo_rules_dir(), &self.env).expect("the shipped packs must load")
        }

        /// The injected protected path: a rule pack cannot see it, and it must
        /// still land red.
        fn protected(&self) -> ProtectedPaths {
            ProtectedPaths::from_trees([self.root.join("Windows")])
        }

        fn brain(&self, packs: Vec<RulePack>) -> Brain {
            Brain::builder()
                .protected_paths(self.protected())
                .packs(packs)
                .lock_probe(Arc::new(AlwaysUnlocked))
                .build()
        }
    }

    /// Write a file and backdate its modification time by `hours`.
    fn aged(path: &Path, bytes: &[u8], hours: u64) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, bytes).expect("write");
        let when = SystemTime::now() - std::time::Duration::from_secs(hours * 3600);
        // `set_times` needs both; keeping access time equal to modification
        // time is what an untouched file looks like anyway.
        let times = fs::FileTimes::new().set_modified(when).set_accessed(when);
        fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_times(times)
            .expect("backdate");
    }

    fn repo_rules_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .join("rules")
    }

    fn names(findings: &[Finding]) -> Vec<String> {
        let mut names: Vec<_> = findings
            .iter()
            .map(|f| {
                f.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| f.path.to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        names
    }

    fn run(fixture: &Fixture, bin: &dyn RecycleBin) -> ScanOutput {
        let packs = fixture.packs();
        let protected = fixture.protected();
        let cancel = CancellationToken::new();
        scan(
            &CleanerContext {
                packs: &packs,
                protected: &protected,
                recycle_bin: bin,
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        )
    }

    /// The validate-all-packs test RULES-FORMAT.md asks CI to run: every pack
    /// that ships must parse, and its id must match its file name.
    #[test]
    fn every_shipped_rule_pack_is_valid() {
        let env = MapEnv::new()
            .with("TEMP", "C:\\Temp")
            .with("LOCALAPPDATA", "C:\\Users\\dev\\AppData\\Local");
        let packs = rules::load_dir(&repo_rules_dir(), &env).expect("all packs must load");

        assert!(!packs.is_empty(), "the app ships with no rule packs");
        for pack in &packs {
            assert_eq!(pack.spec.module, "cleaner", "{} targets no module", pack.id());
            assert!(!pack.rules.is_empty(), "{} compiled to nothing", pack.id());
            for rule in &pack.rules {
                assert!(
                    !rule.spec.reason.trim().is_empty(),
                    "{}/{} has no reason",
                    pack.id(),
                    rule.spec.id
                );
            }
        }
    }

    /// Phase 1 ships exactly the three green packs the brief names.
    #[test]
    fn the_shipped_packs_are_the_phase_one_set() {
        let env = MapEnv::new()
            .with("TEMP", "C:\\Temp")
            .with("LOCALAPPDATA", "C:\\Users\\dev\\AppData\\Local");
        let mut ids: Vec<_> = rules::load_dir(&repo_rules_dir(), &env)
            .expect("load")
            .iter()
            .map(|pack| pack.id().to_owned())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["recycle-bin", "thumbnail-icon-cache", "windows-temp"]
        );
    }

    /// No shipped rule may point at a protected path — RULES-FORMAT.md calls
    /// that "a test failure, not a runtime decision".
    #[test]
    fn no_shipped_rule_targets_a_protected_path() {
        let env = MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("ProgramFiles", "C:\\Program Files")
            .with("ProgramFiles(x86)", "C:\\Program Files (x86)")
            .with("ProgramData", "C:\\ProgramData")
            .with("USERPROFILE", "C:\\Users\\dev")
            .with("TEMP", "C:\\Users\\dev\\AppData\\Local\\Temp")
            .with("LOCALAPPDATA", "C:\\Users\\dev\\AppData\\Local");
        let protected = ProtectedPaths::from_env(
            &env,
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife"),
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine"),
        );

        for pack in rules::load_dir(&repo_rules_dir(), &env).expect("load") {
            for rule in &pack.rules {
                for root in rule.roots() {
                    // The recycle bin is a shell namespace object, not a
                    // directory: `scan` handles that pack through the shell API
                    // and never reaches `plan_walks`, so it has no walk root to
                    // clear. Since C2 the gate reports it as unjudgeable, which
                    // is the correct answer for something that is not a
                    // filesystem path — and it must still never be walked.
                    if root.to_string_lossy().to_lowercase().starts_with("shell:") {
                        assert!(
                            matches!(brain::gate_path(root), GateVerdict::Refused(_)),
                            "{}/{}: a shell-namespace root must be refused, not walked",
                            pack.id(),
                            rule.spec.id
                        );
                        continue;
                    }
                    assert!(
                        !protected.is_protected(root),
                        "{}/{} walks the protected path {}",
                        pack.id(),
                        rule.spec.id,
                        root.display()
                    );
                }
            }
        }
    }

    #[test]
    fn a_fixture_scan_produces_deterministic_findings() {
        let fixture = Fixture::new();
        let bin = StubRecycleBin::default();

        let first = run(&fixture, &bin);
        let second = run(&fixture, &bin);

        assert_eq!(names(&first.findings), names(&second.findings));
        assert!(!first.cancelled);
        assert!(first.skipped_rules.is_empty(), "got {:?}", first.skipped_rules);
    }

    /// The table-driven fixture manifest from the module spec: path -> bucket.
    #[test]
    fn the_fixture_manifest_maps_every_path_to_its_bucket() {
        let fixture = Fixture::new();
        let bin = StubRecycleBin::new(3, 4_096);
        let packs = fixture.packs();
        let brain = fixture.brain(packs.clone());

        let protected = fixture.protected();
        let cancel = CancellationToken::new();
        let mut output = scan(
            &CleanerContext {
                packs: &packs,
                protected: &protected,
                recycle_bin: &bin,
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        );

        // The walk only sees what the packs point at. Add the injected
        // protected path the way a broader pack would surface it, so the red
        // bucket is exercised end to end.
        let planted = fixture.root.join("Windows/System32/critical.sys");
        let mut red = Finding::new("cleaner", &planted, FindingKind::File);
        red.size_bytes = 12;
        red.meta = serde_json::to_value(FindingMeta {
            pack_id: "windows-temp".to_owned(),
            pack_name: "Windows Temp Files".to_owned(),
            rule_id: "user-temp-files".to_owned(),
        })
        .expect("meta");
        output.findings.push(red);

        let recommendations = analyze(&output.findings, &brain);
        let bucket_of = |name: &str| {
            let finding = output
                .findings
                .iter()
                .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with(name))
                .unwrap_or_else(|| panic!("`{name}` was not found by the scan"));
            recommendations
                .iter()
                .find(|rec| rec.finding_ids.contains(&finding.id))
                .map(|rec| rec.risk)
                .unwrap_or_else(|| panic!("`{name}` produced no recommendation"))
        };
        let found = |name: &str| {
            output
                .findings
                .iter()
                .any(|f| f.path.to_string_lossy().replace('\\', "/").ends_with(name))
        };

        // Green.
        assert_eq!(bucket_of("Temp/old-installer.tmp"), RiskBucket::Safe);
        assert_eq!(bucket_of("Temp/nested/deep.tmp"), RiskBucket::Safe);
        assert_eq!(bucket_of("Explorer/iconcache_32.db"), RiskBucket::Safe);
        assert_eq!(bucket_of("shell:RecycleBinFolder"), RiskBucket::Safe);

        // Red, by protected-path injection, despite a Safe rule claiming it.
        assert_eq!(
            bucket_of("Windows/System32/critical.sys"),
            RiskBucket::DoNotDelete
        );

        // Not offered at all.
        assert!(!found("Temp/session.lock"), "excluded by the pack");
        assert!(!found("Temp/being-written.tmp"), "inside the quiet period");
        assert!(!found("Temp/yesterdayish.tmp"), "younger than 24h");
        assert!(!found("Explorer/notes.txt"), "not a *.db");
        assert!(!found("Downloads/keep-me.zip"), "no pack targets Downloads");
    }

    #[test]
    fn the_injected_protected_path_is_never_actionable() {
        let fixture = Fixture::new();
        let brain = fixture.brain(fixture.packs());
        let planted = fixture.root.join("Windows/System32/critical.sys");

        let mut finding = Finding::new("cleaner", &planted, FindingKind::File);
        finding.size_bytes = 12;
        finding.meta = serde_json::to_value(FindingMeta {
            pack_id: "windows-temp".to_owned(),
            pack_name: "Windows Temp Files".to_owned(),
            rule_id: "user-temp-files".to_owned(),
        })
        .expect("meta");

        let recommendations = analyze(&[finding], &brain);
        assert_eq!(recommendations.len(), 1);
        assert_eq!(recommendations[0].risk, RiskBucket::DoNotDelete);
        assert_eq!(recommendations[0].action, Action::InfoOnly);
        assert_eq!(recommendations[0].savings_bytes, 0);
        assert!(recommendations[0].reason.contains("protected"));
    }

    #[test]
    fn every_recommendation_carries_a_reason() {
        let fixture = Fixture::new();
        let bin = StubRecycleBin::new(2, 100);
        let output = run(&fixture, &bin);
        let brain = fixture.brain(fixture.packs());

        let recommendations = analyze(&output.findings, &brain);
        assert!(!recommendations.is_empty());
        for rec in &recommendations {
            assert!(
                !rec.reason.trim().is_empty(),
                "an unexplained suggestion for a finding"
            );
        }
    }

    #[test]
    fn findings_carry_the_pack_and_rule_that_matched() {
        let fixture = Fixture::new();
        let bin = StubRecycleBin::default();
        let output = run(&fixture, &bin);

        let temp_finding = output
            .findings
            .iter()
            .find(|f| f.path.ends_with("old-installer.tmp"))
            .expect("the temp file");
        let meta: FindingMeta =
            serde_json::from_value(temp_finding.meta.clone()).expect("meta shape");
        assert_eq!(meta.pack_id, "windows-temp");
        assert_eq!(meta.rule_id, "user-temp-files");
        assert_eq!(meta.pack_name, "Windows Temp Files");
    }

    #[test]
    fn an_empty_recycle_bin_produces_no_finding() {
        let fixture = Fixture::new();
        let output = run(&fixture, &StubRecycleBin::default());
        assert!(
            !output
                .findings
                .iter()
                .any(|f| f.kind == FindingKind::RecycleBinItem),
            "an empty bin has nothing to offer"
        );
    }

    #[test]
    fn a_full_recycle_bin_is_one_finding_sized_by_the_shell_api() {
        let fixture = Fixture::new();
        let output = run(&fixture, &StubRecycleBin::new(7, 12_345));

        let finding = output
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::RecycleBinItem)
            .expect("the recycle bin finding");
        assert_eq!(finding.size_bytes, 12_345);
        assert_eq!(finding.path, PathBuf::from(RECYCLE_BIN_PATH));
        assert_eq!(finding.meta.get("itemCount").and_then(|v| v.as_u64()), Some(7));
        assert!(is_recycle_bin_finding(finding));
    }

    #[test]
    fn scanning_never_modifies_the_fixture() {
        let fixture = Fixture::new();
        let before = tree_snapshot(&fixture.root);

        run(&fixture, &StubRecycleBin::new(1, 1));

        assert_eq!(
            tree_snapshot(&fixture.root),
            before,
            "scan() must be read-only"
        );
    }

    fn tree_snapshot(root: &Path) -> Vec<(PathBuf, u64)> {
        let cancel = CancellationToken::new();
        let mut entries = Vec::new();
        scanner::walk(root, &cancel, &mut |entry| {
            entries.push((entry.path.clone(), entry.size_bytes));
        });
        entries.sort();
        entries
    }

    #[test]
    fn cancellation_stops_a_scan_and_says_so() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let protected = fixture.protected();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let output = scan(
            &CleanerContext {
                packs: &packs,
                protected: &protected,
                recycle_bin: &StubRecycleBin::default(),
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        );

        assert!(output.cancelled);
        assert!(output.findings.is_empty());
    }

    #[test]
    fn progress_reports_the_running_count() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let protected = fixture.protected();
        let cancel = CancellationToken::new();
        let mut ticks = 0u64;
        let mut last = 0u64;
        let mut sink = |seen: u64, _: &Path| {
            ticks += 1;
            last = seen;
        };

        scan(
            &CleanerContext {
                packs: &packs,
                protected: &protected,
                recycle_bin: &StubRecycleBin::default(),
                now: Utc::now(),
                cancel: &cancel,
            },
            Some(&mut sink),
        );

        assert!(ticks > 0, "progress must be reported");
        assert_eq!(last, ticks, "the count must be monotonic");
    }

    #[test]
    fn a_pack_whose_variable_is_unset_is_reported_not_hidden() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packs = rules::load_dir(&repo_rules_dir(), &MapEnv::new()).expect("load");
        let protected = ProtectedPaths::empty();
        let cancel = CancellationToken::new();

        let output = scan(
            &CleanerContext {
                packs: &packs,
                protected: &protected,
                recycle_bin: &StubRecycleBin::default(),
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        );

        assert!(
            output
                .skipped_rules
                .iter()
                .any(|entry| entry.contains("%TEMP%")),
            "a rule that could not run must be reported, got {:?}",
            output.skipped_rules
        );
        drop(dir);
    }

    /// Security review C1d: a root the hard gates protect is never walked, and
    /// the scan says so rather than quietly finding nothing.
    #[test]
    fn a_protected_walk_root_is_refused_and_reported() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        // Somebody pointed a pack at a place the gates protect.
        let protected = ProtectedPaths::from_trees([fixture.root.join("AppData/Local/Temp")]);
        let cancel = CancellationToken::new();

        let output = scan(
            &CleanerContext {
                packs: &packs,
                protected: &protected,
                recycle_bin: &StubRecycleBin::default(),
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        );

        assert!(
            output
                .skipped_rules
                .iter()
                .any(|entry| entry.contains("protected location")),
            "a refused root must be reported, got {:?}",
            output.skipped_rules
        );
        assert!(
            !output
                .findings
                .iter()
                .any(|f| f.path.to_string_lossy().contains("old-installer")),
            "a protected root must not be walked at all"
        );
    }

    /// The same, one layer deeper: the root itself is innocent and the thing it
    /// resolves to is not.
    #[cfg(windows)]
    #[test]
    fn a_walk_root_that_resolves_into_a_protected_location_is_refused() {
        let fixture = Fixture::new();
        let temp = fixture.root.join("AppData/Local/Temp");
        let real_windows = fixture.root.join("Windows");
        let door = temp.join("innocent");
        if std::os::windows::fs::symlink_dir(&real_windows, &door).is_err() {
            // M11: loud, not silent. This used to `return` and pass.
            crate::core::test_support::privileged_operation_unavailable(
                "a_walk_root_that_resolves_into_a_protected_location_is_refused",
                "creating a directory symlink needs Developer Mode or elevation",
            );
            return;
        }

        let protected = fixture.protected();
        assert!(
            protected.reason_for(&door).is_none(),
            "the premise: the link's own path is not lexically protected"
        );
        match resolve_root(&door, &protected) {
            RootVerdict::Refused(why) => {
                assert!(why.contains("resolves to"), "got `{why}`");
            }
            RootVerdict::Walk(_) => panic!("a root resolving into a protected tree must be refused"),
        }
    }

    /// Security review C2, end to end: a hostile pack that points at
    /// `C:\Windows` through the administrative share must not get anywhere
    /// near a Safe/Quarantine verdict for a file under it.
    ///
    /// Every layer is asserted separately, because the last round fixed one
    /// layer and shipped with the others still open: the pack must not parse,
    /// the root must not be walked, and the verdict must be red even if a
    /// finding is planted by hand as though it had been.
    #[test]
    fn a_hostile_unc_pack_cannot_reach_a_file_under_windows() {
        const HOSTILE: &str = r#"{
            "id": "hostile",
            "name": "Hostile",
            "version": 1,
            "module": "cleaner",
            "enabled_by_default": true,
            "description": "Points at C:\\Windows through the administrative share.",
            "rules": [
                {
                    "id": "unc-windows",
                    "paths": ["\\\\localhost\\C$\\Windows\\**"],
                    "match": { "min_age_hours": 1 },
                    "score": { "base": 95, "max_bucket": "safe" },
                    "action": "quarantine",
                    "reason": "Hostile rule that claims C:\\Windows is safe.",
                    "requires_admin": true
                }
            ]
        }"#;

        // Layer 1: the parser refuses the pattern outright.
        let err = rules::parse_pack("hostile", HOSTILE)
            .expect_err("a `\\\\`-prefixed pattern must not parse");
        assert!(
            err.to_string().contains("UNC"),
            "the refusal must say why, got `{err}`"
        );

        // Layer 2: even if it had parsed, the walk-root gate refuses the root.
        let env = MapEnv::new().with("USERPROFILE", "C:\\Users\\dev");
        let protected = ProtectedPaths::from_env(
            &env,
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife"),
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine"),
        );
        for root in [
            "\\\\localhost\\C$\\Windows",
            "\\\\127.0.0.1\\C$\\Windows",
            "\\\\?\\UNC\\localhost\\c$\\Windows",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows",
            "\\??\\C:\\Windows",
            "C:Windows",
            "\\Windows",
            "\\\\server\\share",
        ] {
            match resolve_root(Path::new(root), &protected) {
                RootVerdict::Refused(why) => assert!(
                    why.len() > 20,
                    "`{root}` must be refused with an explanation, got `{why}`"
                ),
                RootVerdict::Walk(identity) => {
                    panic!("`{root}` must never be walked (identity `{identity}`)")
                }
            }
        }

        // Layer 3: a finding planted at the UNC spelling, with the hostile
        // rule's own metadata, still lands red with no action and no savings.
        let spec = serde_json::from_str::<rules::RulePackSpec>(HOSTILE).expect("shape");
        let pack = rules::compile_spec(spec, &MapEnv::new()).expect("compile");
        let brain = Brain::builder()
            .protected_paths(protected)
            .packs(vec![pack])
            .lock_probe(Arc::new(AlwaysUnlocked))
            .build();

        for victim in [
            "\\\\localhost\\C$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\127.0.0.1\\C$\\Windows\\System32\\ntoskrnl.exe",
            "\\\\?\\UNC\\localhost\\c$\\Windows\\System32\\ntoskrnl.exe",
            "\\??\\C:\\Windows\\System32\\ntoskrnl.exe",
            "\\\\.\\C:\\Windows\\System32\\ntoskrnl.exe",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows\\System32\\x",
            "C:Windows\\System32\\x",
            "\\Windows\\System32\\x",
            "C:\\Windows\\System32\\ntoskrnl.exe",
        ] {
            let mut finding = Finding::new("cleaner", victim, FindingKind::File);
            finding.size_bytes = 4_096;
            finding.meta = serde_json::to_value(FindingMeta {
                pack_id: "hostile".to_owned(),
                pack_name: "Hostile".to_owned(),
                rule_id: "unc-windows".to_owned(),
            })
            .expect("meta");

            let recommendations = analyze(&[finding], &brain);
            assert_eq!(recommendations.len(), 1, "`{victim}`");
            let rec = &recommendations[0];
            assert_eq!(rec.risk, RiskBucket::DoNotDelete, "`{victim}` must be red");
            assert_eq!(rec.action, Action::InfoOnly, "`{victim}` must offer nothing");
            assert_eq!(rec.savings_bytes, 0, "`{victim}` must promise nothing");
        }
    }

    /// The privilege-free companion to the symlink test below: the walk-root
    /// gate refuses a root it cannot judge, on every machine, with no reparse
    /// point required (security review M11).
    #[test]
    fn a_walk_root_the_gate_cannot_judge_is_refused_without_needing_a_link() {
        let protected = ProtectedPaths::empty();
        for root in [
            "\\\\localhost\\C$\\Windows\\Temp",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Temp",
            "\\??\\C:\\Temp",
            "C:Temp",
            "\\Temp",
        ] {
            assert!(
                matches!(
                    resolve_root(Path::new(root), &protected),
                    RootVerdict::Refused(_)
                ),
                "`{root}` must be refused even with an empty protected list"
            );
        }
        // And the shape the gate exists for still walks.
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            resolve_root(dir.path(), &protected),
            RootVerdict::Walk(_)
        ));
    }

    /// Security review L5: `%TEMP%` and `%LOCALAPPDATA%\Temp` are the same
    /// folder on a stock install, and walking it twice doubles the scan. Both
    /// rules still get to see every entry.
    #[test]
    fn two_rules_naming_the_same_folder_share_one_walk() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let protected = fixture.protected();
        let temp_pack = packs
            .iter()
            .find(|pack| pack.id() == "windows-temp")
            .expect("the temp pack");

        let (groups, refused) = plan_walks(temp_pack, &protected);

        assert!(refused.is_empty(), "got {refused:?}");
        assert_eq!(
            groups.len(),
            1,
            "%TEMP% and %LOCALAPPDATA%\\Temp resolve to one folder here"
        );
        assert_eq!(
            groups[0].rules.len(),
            2,
            "both rules must still be tried against every entry"
        );
    }

    #[test]
    fn the_same_path_is_never_offered_twice() {
        let mut findings = vec![
            Finding::new("cleaner", "C:\\Temp\\a.tmp", FindingKind::File),
            Finding::new("cleaner", "c:\\temp\\A.TMP", FindingKind::File),
            Finding::new("cleaner", "C:\\Temp\\b.tmp", FindingKind::File),
        ];
        dedupe_by_path(&mut findings);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn a_finding_with_no_matching_rule_falls_through_to_red() {
        let fixture = Fixture::new();
        let brain = fixture.brain(fixture.packs());
        let finding = Finding::new("cleaner", "C:\\Somewhere\\odd.bin", FindingKind::File);

        let recommendations = analyze(&[finding], &brain);
        assert_eq!(recommendations[0].risk, RiskBucket::DoNotDelete);
        assert_eq!(recommendations[0].action, Action::InfoOnly);
    }

    #[test]
    fn the_summary_totals_each_bucket() {
        let finding = Finding::new("cleaner", "C:\\a", FindingKind::File);
        let id = finding.id;
        let make = |risk, bytes| {
            Recommendation::new(
                vec![id],
                risk,
                50,
                if risk == RiskBucket::DoNotDelete {
                    Action::InfoOnly
                } else {
                    Action::Quarantine
                },
                "because",
                bytes,
            )
            .expect("recommendation")
        };

        let summary = summarise(&[
            make(RiskBucket::Safe, 1_000),
            make(RiskBucket::Safe, 500),
            make(RiskBucket::NeedsReview, 20),
            make(RiskBucket::DoNotDelete, 0),
            make(RiskBucket::DoNotDelete, 0),
        ]);

        assert_eq!(
            summary,
            CleanerSummary {
                safe_bytes: 1_500,
                review_bytes: 20,
                blocked_items: 2,
            }
        );
    }

    #[test]
    fn the_manifest_declares_the_full_capability_set() {
        let module = CleanerModule;
        assert_eq!(module.manifest().id, module.id());
        assert_eq!(module.manifest().capabilities, ModuleCapabilities::FULL);
    }

    #[test]
    fn finding_lookup_by_id_works() {
        let a = Finding::new("cleaner", "C:\\a", FindingKind::File);
        let b = Finding::new("cleaner", "C:\\b", FindingKind::File);
        let id = b.id;
        let findings = vec![a, b];
        assert_eq!(find(&findings, id).map(|f| f.id), Some(id));
        assert!(find(&findings, Uuid::new_v4()).is_none());
    }
}
