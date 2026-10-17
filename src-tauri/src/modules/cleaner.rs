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
use rayon::prelude::*;
use uuid::Uuid;

use crate::core::brain::{
    self, Brain, CompiledRule, GateVerdict, ProtectedPaths, RuleAction, RulePack,
};
use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};
use crate::core::path_compare::CanonicalPath;
use crate::core::platform::RecycleBin;
use crate::core::scanner::{self, CancellationToken, WalkEntry};
use crate::core::types::{
    Action, Finding, FindingKind, Justification, Recommendation, RiskBucket,
};

/// Files touched more recently than this are left alone whatever a rule says:
/// something is probably still writing them.
pub const MIN_QUIET_HOURS: i64 = 1;

/// Pack whose findings come from the shell API rather than a walk.
pub const RECYCLE_BIN_PACK: &str = "recycle-bin";

/// Id of this module, which is also the `module` value its packs declare.
pub const CLEANER_MODULE_ID: &str = "cleaner";

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

/// Per-module settings for the cleaner, stored under `module.cleaner`.
///
/// Only *overrides* are stored. A pack the user has never touched is not in the
/// map and answers with its own `enabled_by_default`, so shipping a new pack
/// turns it on (or leaves it off) without needing to touch anybody's database.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CleanerSettings {
    /// Pack id → enabled, for packs the user has decided about.
    pub packs: BTreeMap<String, bool>,
}

impl CleanerSettings {
    /// Whether a pack runs, honouring the user's choice and falling back to the
    /// pack's own declared default.
    pub fn is_enabled(&self, pack: &RulePack) -> bool {
        self.packs
            .get(pack.id())
            .copied()
            .unwrap_or(pack.spec.enabled_by_default)
    }

    /// Record a decision about one pack.
    pub fn set_enabled(&mut self, pack_id: impl Into<String>, enabled: bool) {
        self.packs.insert(pack_id.into(), enabled);
    }
}

/// Everything the cleaner needs that is not a rule pack.
pub struct CleanerContext<'a> {
    /// Packs to run, already compiled and expanded. Disabled packs are filtered
    /// out by [`scan`] rather than by the caller, so a pack that ships off by
    /// default really is off everywhere.
    pub packs: &'a [RulePack],
    /// Which packs the user has turned on or off.
    pub settings: &'a CleanerSettings,
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
    let mut walks: Vec<SharedWalk<'_>> = Vec::new();

    for pack in ctx.packs {
        // Packs written for other tools (the empty-folders pack) share the
        // rules directory but are none of the cleaner's business.
        if pack.spec.module != CLEANER_MODULE_ID {
            continue;
        }
        // A pack the user turned off — or one that ships off, like `dev-caches`
        // and `windows-update-leftovers` — is not walked at all. Silently:
        // "you asked for this not to run" is not a partial scan, so it has no
        // business in `skipped_rules`.
        if !ctx.settings.is_enabled(pack) {
            tracing::debug!(pack = pack.id(), "pack disabled; not scanned");
            continue;
        }

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
        share_walks(&mut walks, pack, groups);
    }

    for walk in &walks {
        let summary = scanner::walk(&walk.root, ctx.cancel, &mut |entry| {
            seen += 1;
            if let Some(sink) = progress.as_deref_mut() {
                sink(seen, &entry.path);
            }
            // Packs in pack order, rules in declaration order, and the first
            // one to claim an entry wins — what one walk per pack plus
            // `dedupe_by_path`'s "first pack wins" used to add up to.
            'claim: for (pack, rules) in &walk.claims {
                for held in rules {
                    let match_path = held.match_path(&entry.path, &walk.root);
                    if let Some(finding) =
                        finding_for(entry, &match_path, held.rule, pack, ctx.now)
                    {
                        output.findings.push(finding);
                        break 'claim;
                    }
                }
            }
        });
        output.unreadable.extend(summary.unreadable);
        output.cancelled |= summary.cancelled;
        if output.cancelled {
            return output;
        }
    }

    // One path can match rules in two packs; the first pack to claim it wins,
    // so a file is never offered twice.
    dedupe_by_path(&mut output.findings);
    output
}

/// One directory walked once for every pack that points at it.
///
/// `%TEMP%` is a root of three shipped packs, and on a full temp folder each
/// extra walk was ten seconds of reading the same listing again. Each pack's
/// own [`plan_walks`] still decides what that pack may walk; this only folds
/// the walks the packs were already allowed.
struct SharedWalk<'a> {
    /// The root as the first pack to claim it spelled it.
    root: PathBuf,
    /// The canonical identity [`plan_walks`] folded on.
    identity: String,
    /// Every pack walking this root with its rules, in pack order.
    claims: Vec<(&'a RulePack, Vec<GroupRule<'a>>)>,
}

/// Merge one pack's walk groups into the shared walks, folding on the same
/// canonical identity [`plan_walks`] folds on inside a pack.
fn share_walks<'a>(walks: &mut Vec<SharedWalk<'a>>, pack: &'a RulePack, groups: Vec<WalkGroup<'a>>) {
    for group in groups {
        let Some(walk) = walks.iter_mut().find(|walk| walk.identity == group.identity) else {
            walks.push(SharedWalk {
                root: group.root,
                identity: group.identity,
                claims: vec![(pack, group.rules)],
            });
            continue;
        };
        // Walked under the first pack's spelling, so a pack spelling it
        // differently keeps its own anchor for its globs (review M14), exactly
        // as a folded rule inside one pack does.
        let spelled_alike = CanonicalPath::lexical(&walk.root) == CanonicalPath::lexical(&group.root);
        let rules = group
            .rules
            .into_iter()
            .map(|held| GroupRule {
                own_root: held
                    .own_root
                    .or_else(|| (!spelled_alike).then(|| group.root.clone())),
                rule: held.rule,
            })
            .collect();
        walk.claims.push((pack, rules));
    }
}

/// One directory to walk and the rules that asked for it.
struct WalkGroup<'a> {
    /// The root exactly as the *first* rule spelled it. The canonical form
    /// decides *whether* to walk; the spelled form decides *what* to walk,
    /// because the globs were compiled against that spelling and would stop
    /// matching if a junction rewrote it.
    root: PathBuf,
    /// The canonical identity the fold keyed on.
    identity: String,
    /// Rules whose roots resolve to this same directory, in declaration order.
    rules: Vec<GroupRule<'a>>,
}

/// One rule inside a [`WalkGroup`], plus how to spell a walked path so that
/// rule's globs recognise it.
struct GroupRule<'a> {
    /// The rule.
    rule: &'a CompiledRule,
    /// This rule's own spelling of the walk root, when it differs from the
    /// group's.
    ///
    /// The fold in [`plan_walks`] keys on canonical identity but keeps the first
    /// rule's spelling, while [`CompiledRule::matches_path`] is a text glob
    /// anchored to *each rule's own* spelling. So two rules in one pack sharing
    /// a canonical root spelled differently — which is `%TEMP%` (8.3 on any
    /// account whose name does not fit) against `%LOCALAPPDATA%\Temp` (never
    /// 8.3) on a stock machine — left every rule but the first unable to claim a
    /// single entry, silently (review M14).
    ///
    /// Rather than give up the fold and walk the tree twice, the walked path is
    /// re-spelled into this rule's anchor before its globs are asked.
    own_root: Option<PathBuf>,
}

impl GroupRule<'_> {
    /// The path to test this rule's globs against, given an entry the walk
    /// produced under `walked_root`.
    ///
    /// Falls back to the walked path when there is nothing to re-spell, which
    /// is both the common case and the safe one: a rule that cannot be
    /// re-anchored is no worse off than it was.
    fn match_path<'p>(&self, entry: &'p Path, walked_root: &Path) -> std::borrow::Cow<'p, Path> {
        let Some(own_root) = &self.own_root else {
            return std::borrow::Cow::Borrowed(entry);
        };
        // Lexical on purpose, and it is the same reason `matches_path` is
        // lexical: this is string surgery on a glob anchor, not a question about
        // where two paths point. `scanner::walk` builds every entry by joining
        // `walked_root`, so the prefix is there verbatim — and where it somehow
        // is not, the rule keeps the behaviour it had before this fix.
        match entry.strip_prefix(walked_root) {
            Ok(tail) => std::borrow::Cow::Owned(own_root.join(tail)),
            Err(_) => std::borrow::Cow::Borrowed(entry),
        }
    }
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
                Some(&index) => {
                    // The surviving group is walked under the *first* rule's
                    // spelling, so a rule joining it with a different spelling
                    // needs its anchor carried along or its globs will never
                    // see a path they recognise (review M14).
                    //
                    // `CanonicalPath::lexical` and not `resolved`: the question
                    // is whether the two are the same *text* once case and
                    // separators are normalised, because that is all a glob can
                    // tell apart. Whether they are the same *place* was already
                    // answered — it is the fold key that put them here.
                    let spelled_alike = CanonicalPath::lexical(&groups[index].root)
                        == CanonicalPath::lexical(root);
                    let own_root = (!spelled_alike).then(|| root.clone());
                    if let Some(own) = &own_root {
                        tracing::debug!(
                            pack = pack.id(),
                            rule = rule.spec.id.as_str(),
                            walked = %groups[index].root.display(),
                            anchored_at = %own.display(),
                            "walk root folded; re-anchoring this rule's globs"
                        );
                    }
                    groups[index].rules.push(GroupRule { rule, own_root });
                }
                None => {
                    by_identity.insert(identity.clone(), groups.len());
                    groups.push(WalkGroup {
                        root: root.clone(),
                        identity,
                        rules: vec![GroupRule {
                            rule,
                            own_root: None,
                        }],
                    });
                }
            }
        }
    }

    // A rule that reached no group at all claims nothing, and a cleaner that
    // silently stops cleaning is the failure mode this whole file is written
    // against. Every rule that got here with roots must appear in one.
    for rule in &pack.rules {
        if rule.roots().is_empty() {
            continue;
        }
        let placed = groups
            .iter()
            .any(|group| group.rules.iter().any(|held| held.rule.spec.id == rule.spec.id));
        let already_reported = refused
            .iter()
            .any(|line| line.starts_with(&format!("{}/{}: ", pack.id(), rule.spec.id)));
        if !placed && !already_reported {
            refused.push(format!(
                "{}/{}: it was not walked because none of its locations could be reached",
                pack.id(),
                rule.spec.id
            ));
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
            // The identity that folds duplicate roots together. Through
            // `path_compare`, so `%TEMP%` and `%LOCALAPPDATA%\Temp` still fold
            // into one walk when the two variables disagree about how to spell
            // the tree they share — lower-casing the text alone did not.
            RootVerdict::Walk(CanonicalPath::resolved(&canonical).key())
        }
        // A rule pointing at a folder this machine does not have is not an
        // error; the walk simply finds nothing.
        GateVerdict::Missing => RootVerdict::Walk(CanonicalPath::resolved(root).key()),
        GateVerdict::Refused(why) => {
            RootVerdict::Refused(format!("it was not walked because {why}"))
        }
    }
}

/// Decide whether one walked entry becomes a finding for one rule.
///
/// `match_path` is the spelling the rule's globs are tested against, which is
/// `entry.path` unless the walk-root fold re-anchored this rule (see
/// [`GroupRule::match_path`]). Everything else — the finding's path, its size,
/// its timestamps — comes from the entry as the walk really found it, because
/// that is the file the executor will be asked to move.
fn finding_for(
    entry: &WalkEntry,
    match_path: &Path,
    rule: &CompiledRule,
    pack: &RulePack,
    now: DateTime<Utc>,
) -> Option<Finding> {
    // Phase 1 never proposes directories; see the module header.
    if entry.is_dir {
        return None;
    }
    if !rule.matches_path(match_path) {
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
    analyze_at(findings, brain, Utc::now())
}

/// [`analyze`] with the reference instant injected, so an age-sensitive test is
/// not at the mercy of when it runs.
pub fn analyze_at(findings: &[Finding], brain: &Brain, now: DateTime<Utc>) -> Vec<Recommendation> {
    analyze_with(findings, brain, now, &CancellationToken::new(), None).recommendations
}

/// Progress callback for [`analyze_with`]: findings analysed so far, how many
/// there are in all, and the path just analysed. Throttling is the caller's job.
pub type AnalyzeProgressSink<'a> = &'a mut dyn FnMut(u64, u64, &Path);

/// What [`analyze_with`] produced.
#[derive(Debug, Default)]
pub struct AnalyzeOutput {
    /// One per analysed finding that passed its own invariants.
    pub recommendations: Vec<Recommendation>,
    /// How many findings, from the front of the slice, were analysed. Equal to
    /// the slice length unless the run was cancelled.
    pub analyzed: usize,
    /// Whether the run stopped early.
    pub cancelled: bool,
}

/// Findings analysed between two cancellation checks and progress ticks.
///
/// Each finding costs a canonicalisation and a lock probe — a few hundred
/// microseconds on a real disk with an antivirus filter in the way — so a
/// chunk this size answers a cancel within a fraction of a second while
/// keeping the per-chunk overhead negligible.
const ANALYZE_CHUNK: usize = 256;

/// [`analyze_at`], cancellable and reporting progress.
///
/// The gates are the same per-finding gates as ever (path refusal, then the
/// lock probe, then the rule): the work is spread over a thread pool in
/// chunks, not reduced. On cancel, the findings *before* `analyzed` all have
/// their recommendation and the rest were never looked at; the caller keeps
/// only the analysed prefix, so nothing reaches the review screen unjudged.
pub fn analyze_with(
    findings: &[Finding],
    brain: &Brain,
    now: DateTime<Utc>,
    cancel: &CancellationToken,
    progress: Option<AnalyzeProgressSink<'_>>,
) -> AnalyzeOutput {
    let rules_by_id = index_rules(brain);
    let total = findings.len() as u64;
    let mut progress = progress;
    let mut output = AnalyzeOutput {
        recommendations: Vec::with_capacity(findings.len()),
        ..AnalyzeOutput::default()
    };

    for chunk in findings.chunks(ANALYZE_CHUNK) {
        if cancel.is_cancelled() {
            output.cancelled = true;
            break;
        }
        let judged: Vec<Option<Recommendation>> = chunk
            .par_iter()
            .map(|finding| recommend(finding, brain, &rules_by_id, now))
            .collect();
        output.recommendations.extend(judged.into_iter().flatten());
        output.analyzed += chunk.len();

        if let (Some(sink), Some(last)) = (progress.as_deref_mut(), chunk.last()) {
            sink(output.analyzed as u64, total, &last.path);
        }
    }

    output
}

/// The one recommendation for one finding, or `None` if it failed its own
/// invariants (logged, never silently).
fn recommend(
    finding: &Finding,
    brain: &Brain,
    rules_by_id: &BTreeMap<(String, String), &CompiledRule>,
    now: DateTime<Utc>,
) -> Option<Recommendation> {
    let key = rule_key(finding);
    let matched = key.as_ref().and_then(|key| rules_by_id.get(key).copied());
    let verdict = brain.classify_finding_with(finding, matched, &signals_for(finding, now));

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

    // The machine-checkable half of the reason. A finding the walk produced
    // always names the rule that matched it; a finding that somehow does not
    // can only ever be informational, which is what the executor's gate 1
    // then enforces (architect addition A, phase 3).
    let justification = match key {
        Some((pack_id, rule_id)) => Justification::Rule { pack_id, rule_id },
        None => Justification::Informational,
    };

    match Recommendation::new(
        vec![finding.id],
        verdict.bucket,
        verdict.confidence,
        action,
        verdict.reason,
        savings,
        justification,
    ) {
        Ok(rec) => Some(rec),
        Err(err) => {
            tracing::error!(
                path = %finding.path.display(),
                error = %err,
                "dropped a recommendation that failed its own invariants"
            );
            None
        }
    }
}

/// What the filesystem says about one finding, for the brain's step 3.
///
/// `owner_process_running` is deliberately left `false` here: the brain asks the
/// process list itself, through its own cached probe, so a module cannot forget
/// to (see `Brain::classify_by_rule`).
fn signals_for(finding: &Finding, now: DateTime<Utc>) -> brain::Signals {
    let days_since = |when: Option<DateTime<Utc>>| when.map(|when| (now - when).num_days());

    brain::Signals {
        // The *later* of the two clocks: "nothing has been near this in six
        // months" is only true if neither reading nor writing has happened.
        // `Option::max` is what we want here — `None` sorts below `Some`, so a
        // filesystem that reports only one of them still answers.
        idle_days: days_since(finding.accessed.max(finding.modified)),
        written_days: days_since(finding.modified),
        size_bytes: finding.size_bytes,
        in_known_cache_dir: brain::score::in_known_cache_dir(&finding.path),
        owner_process_running: false,
    }
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
///
/// Keyed through [`CanonicalPath`], because "the same path" has to mean the same
/// place and not the same string: two packs whose roots came from variables
/// spelled differently would otherwise both offer the same file, and the user
/// would be asked to approve it twice.
fn dedupe_by_path(findings: &mut Vec<Finding>) {
    // Resolving is a filesystem round trip per finding — 13 s for 100 000 on
    // one thread — so the keys are computed in parallel. The keep-or-drop pass
    // stays sequential: "first one wins" is an ordering promise.
    let keys: Vec<String> = findings
        .par_iter()
        .map(|finding| CanonicalPath::resolved(&finding.path).key())
        .collect();
    let mut seen = std::collections::HashSet::with_capacity(keys.len());
    let mut keys = keys.into_iter();
    findings.retain(|_| keys.next().is_some_and(|key| seen.insert(key)));
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
    use crate::core::platform::{AlwaysUnlocked, MapEnv, StubProcessProbe, StubRecycleBin};
    use std::fs;
    use std::sync::Arc;
    use std::time::SystemTime;

    /// A miniature dirty Windows profile, built in a tempdir. Every variable the
    /// shipped packs expand is pointed at it, so the packs under test are the
    /// real ones and the developer's machine is never touched.
    ///
    /// One fixture entry exists for every shipped pack, which is what
    /// `docs/RULES-FORMAT.md` asks a new pack to bring with it.
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        env: MapEnv,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().to_path_buf();

            let local = root.join("AppData/Local");
            let temp = local.join("Temp");
            let explorer = local.join("Microsoft/Windows/Explorer");
            let windows = root.join("Windows");
            let downloads = root.join("Downloads");
            for path in [&temp, &explorer, &windows, &downloads] {
                fs::create_dir_all(path).expect("fixture dirs");
            }

            // windows-temp — green: old temp files.
            aged(&temp.join("old-installer.tmp"), b"0123456789", 48);
            aged(&temp.join("nested/deep.tmp"), b"deep", 72);
            // Excluded by the pack's own `exclude` pattern.
            aged(&temp.join("session.lock"), b"lock", 48);
            // Too new: the quiet-period rule and the pack's 24h rule.
            aged(&temp.join("being-written.tmp"), b"live", 0);
            aged(&temp.join("yesterdayish.tmp"), b"recent", 12);

            // thumbnail-icon-cache — green.
            aged(&explorer.join("iconcache_32.db"), b"icon-cache", 48);
            aged(&explorer.join("notes.txt"), b"not a db", 48);

            // browser-cache — green while the browser is closed, yellow while it
            // is open. One entry per browser the pack knows.
            aged(
                &local.join("Google/Chrome/User Data/Default/Cache/f_000001"),
                b"chrome cache entry",
                72,
            );
            aged(
                &local.join("Microsoft/Edge/User Data/Default/Cache/f_000001"),
                b"edge cache entry",
                72,
            );
            aged(
                &local.join("Mozilla/Firefox/Profiles/abc.default/cache2/entries/E1"),
                b"firefox cache entry",
                72,
            );
            // Not a cache: the pack must not reach a profile file.
            aged(
                &local.join("Google/Chrome/User Data/Default/Cookies"),
                b"do not touch my logins",
                72,
            );

            // crash-dumps-logs — green dump, yellow stray dump in temp.
            aged(&local.join("CrashDumps/app.exe.1234.dmp"), b"crash", 48);
            aged(&temp.join("stray.dmp"), b"a stray memory dump", 400);

            // old-downloads — yellow, and never green whatever its age.
            aged(&downloads.join("forgotten.zip"), b"downloaded once", 5_000);
            // Recent enough that the 180-day rule leaves it alone.
            aged(&downloads.join("keep-me.zip"), b"user data", 500);

            // installer-leftovers — yellow.
            aged(&temp.join("setup-something.msi"), b"an installer", 1_000);

            // dev-caches — off by default, so it must produce nothing unless the
            // settings say otherwise.
            aged(&local.join("npm-cache/_cacache/index-v5/aa/bb"), b"npm", 900);

            // windows-update-leftovers — off by default and informational.
            aged(
                &windows.join("SoftwareDistribution/Download/abc/payload.cab"),
                b"update payload",
                900,
            );

            // Red by injection: a `C:\Windows` stand-in inside the fixture.
            aged(&windows.join("System32/critical.sys"), b"do not touch", 200);

            let env = MapEnv::new()
                .with("USERPROFILE", root.to_string_lossy())
                .with("TEMP", temp.to_string_lossy())
                .with("LOCALAPPDATA", local.to_string_lossy())
                .with("WINDIR", windows.to_string_lossy());

            Self {
                root,
                env,
                _dir: dir,
            }
        }

        fn packs(&self) -> Vec<RulePack> {
            rules::load_dir(&repo_rules_dir(), &self.env)
                .expect("the shipped packs must load")
                .into_iter()
                .filter(|pack| pack.spec.module == CLEANER_MODULE_ID)
                .collect()
        }

        /// The injected protected path: a rule pack cannot see it, and it must
        /// still land red.
        fn protected(&self) -> ProtectedPaths {
            ProtectedPaths::from_trees([self.root.join("Windows")])
        }

        fn brain(&self, packs: Vec<RulePack>) -> Brain {
            self.brain_with(packs, StubProcessProbe::none())
        }

        fn brain_with(&self, packs: Vec<RulePack>, processes: StubProcessProbe) -> Brain {
            Brain::builder()
                .protected_paths(self.protected())
                .packs(packs)
                .lock_probe(Arc::new(AlwaysUnlocked))
                .process_probe(Arc::new(processes))
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
        let settings = CleanerSettings::default();
        scan(
            &CleanerContext {
                packs: &packs,
                settings: &settings,
                protected: &protected,
                recycle_bin: bin,
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        )
    }

    /// A stock Windows layout, for the pack-level tests that are not about a
    /// fixture tree.
    fn stock_env() -> MapEnv {
        MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("ProgramFiles", "C:\\Program Files")
            .with("ProgramFiles(x86)", "C:\\Program Files (x86)")
            .with("ProgramData", "C:\\ProgramData")
            .with("USERPROFILE", "C:\\Users\\dev")
            .with("TEMP", "C:\\Users\\dev\\AppData\\Local\\Temp")
            .with("LOCALAPPDATA", "C:\\Users\\dev\\AppData\\Local")
            .with("APPDATA", "C:\\Users\\dev\\AppData\\Roaming")
    }

    /// The validate-all-packs test RULES-FORMAT.md asks CI to run: every pack
    /// that ships must parse, and its id must match its file name.
    #[test]
    fn every_shipped_rule_pack_is_valid() {
        let packs = rules::load_dir(&repo_rules_dir(), &stock_env()).expect("all packs must load");

        assert!(!packs.is_empty(), "the app ships with no rule packs");
        for pack in &packs {
            assert!(
                ["cleaner", "empty-folders"].contains(&pack.spec.module.as_str()),
                "{} targets no known module",
                pack.id()
            );
            assert!(!pack.rules.is_empty(), "{} compiled to nothing", pack.id());
            assert!(pack.skipped.is_empty(), "{} skipped {:?}", pack.id(), pack.skipped);
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

    /// The packs phase 2 ships, named so adding or losing one is a deliberate
    /// edit rather than a surprise.
    #[test]
    fn the_shipped_packs_are_the_phase_two_set() {
        let mut ids: Vec<_> = rules::load_dir(&repo_rules_dir(), &stock_env())
            .expect("load")
            .iter()
            .map(|pack| pack.id().to_owned())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "browser-cache",
                "crash-dumps-logs",
                "dev-caches",
                "empty-folders",
                "installer-leftovers",
                "old-downloads",
                "recycle-bin",
                "thumbnail-icon-cache",
                "windows-temp",
                "windows-update-leftovers",
            ]
        );
    }

    /// Which packs run when the user has said nothing.
    ///
    /// `dev-caches` is off because module spec 02 says so. `windows-update-leftovers`
    /// is off because its only location is inside `%WINDIR%`, which is protected
    /// in code: the pack can report but never act, so leaving it on by default
    /// would put a permanent "not walked, it is protected" line in front of every
    /// user (`docs/DECISIONS-NEEDED.md`, phase 2).
    #[test]
    fn the_default_pack_selection_is_the_one_the_module_spec_states() {
        let packs: Vec<_> = rules::load_dir(&repo_rules_dir(), &stock_env())
            .expect("load")
            .into_iter()
            .filter(|pack| pack.spec.module == CLEANER_MODULE_ID)
            .collect();
        let settings = CleanerSettings::default();

        let mut on: Vec<_> = packs
            .iter()
            .filter(|pack| settings.is_enabled(pack))
            .map(|pack| pack.id().to_owned())
            .collect();
        let mut off: Vec<_> = packs
            .iter()
            .filter(|pack| !settings.is_enabled(pack))
            .map(|pack| pack.id().to_owned())
            .collect();
        on.sort();
        off.sort();

        assert_eq!(off, vec!["dev-caches", "windows-update-leftovers"]);
        assert_eq!(
            on,
            vec![
                "browser-cache",
                "crash-dumps-logs",
                "installer-leftovers",
                "old-downloads",
                "recycle-bin",
                "thumbnail-icon-cache",
                "windows-temp",
            ]
        );

        // And a user override wins over the pack's own default, both ways.
        let mut settings = CleanerSettings::default();
        settings.set_enabled("dev-caches", true);
        settings.set_enabled("windows-temp", false);
        let by_id = |id: &str| packs.iter().find(|p| p.id() == id).expect("pack");
        assert!(settings.is_enabled(by_id("dev-caches")));
        assert!(!settings.is_enabled(by_id("windows-temp")));
    }

    /// No shipped rule may point at a protected path **with an action** —
    /// RULES-FORMAT.md calls that "a test failure, not a runtime decision".
    ///
    /// One pack does name a protected location: `windows-update-leftovers`, whose
    /// only location is `%WINDIR%\SoftwareDistribution\Download`. It is allowed
    /// to *say so* and nothing else — `action: info_only` plus
    /// `max_bucket: do_not_delete`, a combination the parser can enforce and the
    /// brain's gates enforce again. Reporting a protected location is the red
    /// bucket's "produced, not targeted" honesty feature (module spec 02);
    /// proposing to remove one is the thing the rule exists to forbid, and the
    /// second half of this test proves the brain refuses it either way.
    #[test]
    fn no_shipped_rule_targets_a_protected_path() {
        let env = stock_env();
        let protected = ProtectedPaths::from_env(
            &env,
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife"),
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine"),
        );
        let brain = Brain::builder()
            .protected_paths(protected.clone())
            .packs(rules::load_dir(&repo_rules_dir(), &env).expect("load"))
            .build();

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
                    // A catch-all glob (`**`) has no concrete root, so it names
                    // no location; the Brain's gate still judges every real path.
                    if root.as_os_str().is_empty() || !protected.is_protected(root) {
                        continue;
                    }

                    // The narrow, structural exception: a rule may name a
                    // protected location only if it can never propose anything
                    // about it.
                    assert_eq!(
                        rule.spec.action,
                        RuleAction::InfoOnly,
                        "{}/{} names the protected path {} and proposes an action",
                        pack.id(),
                        rule.spec.id,
                        root.display()
                    );
                    assert_eq!(
                        rule.spec.score.max_bucket,
                        brain::MaxBucket::DoNotDelete,
                        "{}/{} names the protected path {} without capping itself at do_not_delete",
                        pack.id(),
                        rule.spec.id,
                        root.display()
                    );

                    // And asserted rather than assumed: the real brain, asked
                    // about a real file under that root, refuses it.
                    let victim = root.join("something.bin");
                    let verdict = brain.classify(&victim, Some(rule));
                    assert_eq!(verdict.bucket, RiskBucket::DoNotDelete, "{}", victim.display());
                    assert_eq!(verdict.action, Action::InfoOnly, "{}", victim.display());
                    assert!(!verdict.is_actionable(), "{}", victim.display());
                }
            }
        }
    }

    /// A rule pack that names a protected location can report and nothing else.
    /// The one that ships is `windows-update-leftovers`; this pins the whole
    /// chain for it — parse, plan, rate — rather than trusting the JSON.
    #[test]
    fn the_informational_windows_pack_can_report_but_never_act() {
        let env = stock_env();
        let packs = rules::load_dir(&repo_rules_dir(), &env).expect("load");
        let pack = packs
            .iter()
            .find(|pack| pack.id() == "windows-update-leftovers")
            .expect("the pack ships");

        assert!(
            !pack.spec.enabled_by_default,
            "an inert pack must not put a refusal line in front of every user"
        );
        for rule in &pack.rules {
            assert_eq!(rule.spec.action, RuleAction::InfoOnly);
            assert_eq!(rule.spec.score.max_bucket, brain::MaxBucket::DoNotDelete);
        }

        // Even enabled, its walk root is refused before anything is walked.
        let protected = ProtectedPaths::from_env(
            &env,
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife"),
            Path::new("C:\\Users\\dev\\AppData\\Roaming\\WinKnife\\quarantine"),
        );
        let (groups, refused) = plan_walks(pack, &protected);
        assert!(groups.is_empty(), "nothing under %WINDIR% is ever walked");
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0].contains("protected location"),
            "the refusal must say why: {}",
            refused[0]
        );
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
        let settings = CleanerSettings::default();
        let mut output = scan(
            &CleanerContext {
                packs: &packs,
                settings: &settings,
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

        let pack_of = |name: &str| {
            output
                .findings
                .iter()
                .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with(name))
                .and_then(|f| f.meta.get("packId").and_then(|v| v.as_str()))
                .map(str::to_owned)
                .unwrap_or_else(|| panic!("`{name}` was not found by the scan"))
        };

        println!("{:<58} | {:<24} | bucket", "fixture path", "pack");
        for name in [
            "Temp/old-installer.tmp",
            "Temp/nested/deep.tmp",
            "Explorer/iconcache_32.db",
            "Chrome/User Data/Default/Cache/f_000001",
            "Edge/User Data/Default/Cache/f_000001",
            "Firefox/Profiles/abc.default/cache2/entries/E1",
            "CrashDumps/app.exe.1234.dmp",
            "Temp/stray.dmp",
            "Temp/setup-something.msi",
            "Downloads/forgotten.zip",
            "shell:RecycleBinFolder",
        ] {
            println!(
                "{name:<58} | {:<24} | {:?}",
                pack_of(name),
                bucket_of(name)
            );
        }

        // Green.
        assert_eq!(bucket_of("Temp/old-installer.tmp"), RiskBucket::Safe);
        assert_eq!(bucket_of("Temp/nested/deep.tmp"), RiskBucket::Safe);
        assert_eq!(bucket_of("Explorer/iconcache_32.db"), RiskBucket::Safe);
        assert_eq!(bucket_of("shell:RecycleBinFolder"), RiskBucket::Safe);
        // browser-cache, with every browser closed.
        assert_eq!(
            bucket_of("Chrome/User Data/Default/Cache/f_000001"),
            RiskBucket::Safe
        );
        assert_eq!(
            bucket_of("Edge/User Data/Default/Cache/f_000001"),
            RiskBucket::Safe
        );
        assert_eq!(
            bucket_of("Firefox/Profiles/abc.default/cache2/entries/E1"),
            RiskBucket::Safe
        );
        // crash-dumps-logs, the green half.
        assert_eq!(bucket_of("CrashDumps/app.exe.1234.dmp"), RiskBucket::Safe);

        // Yellow.
        assert_eq!(bucket_of("Temp/stray.dmp"), RiskBucket::NeedsReview);
        assert_eq!(bucket_of("Temp/setup-something.msi"), RiskBucket::NeedsReview);
        assert_eq!(bucket_of("Downloads/forgotten.zip"), RiskBucket::NeedsReview);

        // The pack that claimed each of them, so a rule moving between packs is
        // a visible change rather than a silent one.
        assert_eq!(pack_of("Temp/stray.dmp"), "crash-dumps-logs");
        assert_eq!(pack_of("Temp/setup-something.msi"), "installer-leftovers");
        assert_eq!(pack_of("Downloads/forgotten.zip"), "old-downloads");
        assert_eq!(pack_of("Temp/old-installer.tmp"), "windows-temp");

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
        assert!(
            !found("Downloads/keep-me.zip"),
            "younger than the 180-day rule"
        );
        assert!(
            !found("User Data/Default/Cookies"),
            "the browser pack must never reach a profile file"
        );
        assert!(
            !found("npm-cache/_cacache/index-v5/aa/bb"),
            "dev-caches ships off by default"
        );
        assert!(
            !found("SoftwareDistribution/Download/abc/payload.cab"),
            "windows-update-leftovers ships off by default"
        );
    }

    /// `owner_process_absent`, end to end through the shipped `browser-cache`
    /// pack: **browser closed → Safe, browser running → NeedsReview.**
    ///
    /// The field has been parsed and validated since phase 1 and acted on by
    /// nobody. This is the acceptance criterion for making it work, and it runs
    /// against the real pack file rather than a fixture rule.
    #[test]
    fn a_browser_cache_drops_to_review_while_that_browser_is_running() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let bin = StubRecycleBin::default();
        let output = run(&fixture, &bin);

        let entry = |name: &str| {
            output
                .findings
                .iter()
                .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with(name))
                .unwrap_or_else(|| panic!("`{name}` was not found by the scan"))
                .clone()
        };
        let bucket = |brain: &Brain, finding: &Finding| {
            analyze(std::slice::from_ref(finding), brain)[0].risk
        };

        let chrome = entry("Chrome/User Data/Default/Cache/f_000001");
        let edge = entry("Edge/User Data/Default/Cache/f_000001");

        let closed = fixture.brain_with(packs.clone(), StubProcessProbe::none());
        assert_eq!(bucket(&closed, &chrome), RiskBucket::Safe);
        assert_eq!(bucket(&closed, &edge), RiskBucket::Safe);

        let chrome_open =
            fixture.brain_with(packs.clone(), StubProcessProbe::running(["chrome.exe"]));
        assert_eq!(
            bucket(&chrome_open, &chrome),
            RiskBucket::NeedsReview,
            "Chrome is open, so its cache is not safe to take"
        );
        assert_eq!(
            bucket(&chrome_open, &edge),
            RiskBucket::Safe,
            "one browser being open says nothing about another"
        );

        // And the row explains itself rather than still claiming to be safe.
        let recs = analyze(std::slice::from_ref(&chrome), &chrome_open);
        assert!(
            recs[0].reason.contains("chrome.exe"),
            "the downgrade must name the process: `{}`",
            recs[0].reason
        );
        assert_eq!(recs[0].action, Action::Quarantine, "yellow is still offered");
    }

    /// The pack toggle from the settings page, exercised through `scan` rather
    /// than through the struct: turning `dev-caches` on has to make it walk.
    #[test]
    fn a_pack_toggle_decides_whether_a_pack_is_walked_at_all() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let protected = fixture.protected();
        let bin = StubRecycleBin::default();

        let scan_with = |settings: &CleanerSettings| {
            let cancel = CancellationToken::new();
            scan(
                &CleanerContext {
                    packs: &packs,
                    settings,
                    protected: &protected,
                    recycle_bin: &bin,
                    now: Utc::now(),
                    cancel: &cancel,
                },
                None,
            )
        };
        let found = |output: &ScanOutput, name: &str| {
            output
                .findings
                .iter()
                .any(|f| f.path.to_string_lossy().replace('\\', "/").ends_with(name))
        };

        let default = scan_with(&CleanerSettings::default());
        assert!(!found(&default, "npm-cache/_cacache/index-v5/aa/bb"));
        assert!(found(&default, "Temp/old-installer.tmp"));

        let mut on = CleanerSettings::default();
        on.set_enabled("dev-caches", true);
        let enabled = scan_with(&on);
        assert!(
            found(&enabled, "npm-cache/_cacache/index-v5/aa/bb"),
            "turning a pack on must make it walk"
        );

        let mut off = CleanerSettings::default();
        off.set_enabled("windows-temp", false);
        let disabled = scan_with(&off);
        assert!(
            !found(&disabled, "Temp/old-installer.tmp"),
            "turning a pack off must stop it walking"
        );
        assert!(
            disabled.skipped_rules.is_empty(),
            "a pack the user turned off is not a partial scan: {:?}",
            disabled.skipped_rules
        );
    }

    /// Module spec 02: old downloads are **never Safe**, whatever the heuristics
    /// add. The ceiling in the pack is what guarantees it, so this asserts on the
    /// shipped pack rather than on the code.
    #[test]
    fn old_downloads_can_never_be_rated_safe() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let brain = fixture.brain(packs.clone());
        let rule = packs
            .iter()
            .find(|pack| pack.id() == "old-downloads")
            .expect("the pack ships")
            .rules
            .first()
            .expect("its rule");

        assert_eq!(rule.spec.score.max_bucket, brain::MaxBucket::NeedsReview);

        // Every combination of every signal `brain::score` reads, on a path
        // nothing protects. The `owner_process_running` axis used to be pinned
        // to `false` while the comment claimed "every combination" (review L11);
        // it is folded in rather than the claim being narrowed, because the
        // property does hold across it — the ceiling is what guarantees this,
        // and a running process can only lower a ceiling further.
        let path = fixture.root.join("Downloads/forgotten.zip");
        let mut cases = 0usize;
        for idle in [None, Some(0), Some(10), Some(400), Some(10_000)] {
            for size in [0, 4_096, 8 * 1024 * 1024 * 1024] {
                for cache in [false, true] {
                    for owner_running in [false, true] {
                        let verdict = brain.classify_with(
                            &path,
                            Some(rule),
                            &brain::Signals {
                                idle_days: idle,
                                written_days: idle,
                                size_bytes: size,
                                in_known_cache_dir: cache,
                                owner_process_running: owner_running,
                            },
                        );
                        cases += 1;
                        assert_ne!(
                            verdict.bucket,
                            RiskBucket::Safe,
                            "idle={idle:?} size={size} cache={cache} \
                             owner_running={owner_running} made a download green"
                        );
                    }
                }
            }
        }
        assert_eq!(cases, 5 * 3 * 2 * 2, "the combination count is the claim");
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
        let settings = CleanerSettings::default();
        cancel.cancel();

        let output = scan(
            &CleanerContext {
                packs: &packs,
                settings: &settings,
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
        let settings = CleanerSettings::default();
        let mut ticks = 0u64;
        let mut last = 0u64;
        let mut sink = |seen: u64, _: &Path| {
            ticks += 1;
            last = seen;
        };

        scan(
            &CleanerContext {
                packs: &packs,
                settings: &settings,
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

    /// The fixture's findings repeated until they span several analysis
    /// chunks, so a cancel can land between two of them.
    fn many_findings(fixture: &Fixture) -> Vec<Finding> {
        let bin = StubRecycleBin::default();
        let one_pass = run(fixture, &bin).findings;
        assert!(!one_pass.is_empty());
        one_pass
            .iter()
            .cycle()
            .take(ANALYZE_CHUNK * 2 + 1)
            .cloned()
            .collect()
    }

    #[test]
    fn analysis_reports_progress_up_to_the_total_and_matches_the_plain_run() {
        let fixture = Fixture::new();
        let findings = many_findings(&fixture);
        let brain = fixture.brain(fixture.packs());
        let now = Utc::now();
        let mut ticks: Vec<(u64, u64)> = Vec::new();
        let mut sink = |done: u64, total: u64, _: &Path| ticks.push((done, total));

        let output = analyze_with(&findings, &brain, now, &CancellationToken::new(), Some(&mut sink));

        let total = findings.len() as u64;
        assert!(!output.cancelled);
        assert_eq!(output.analyzed, findings.len());
        assert_eq!(ticks.len(), 3, "one tick per chunk");
        assert!(ticks.windows(2).all(|pair| pair[0].0 < pair[1].0), "monotonic");
        assert_eq!(ticks.last(), Some(&(total, total)));
        // Parallel, but not reordered: the same verdicts in the same order.
        let plain = analyze_at(&findings, &brain, now);
        let ids = |recs: &[Recommendation]| {
            recs.iter()
                .map(|rec| (rec.finding_ids.clone(), rec.risk))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&output.recommendations), ids(&plain));
    }

    #[test]
    fn cancelling_analysis_keeps_only_the_judged_prefix() {
        let fixture = Fixture::new();
        let findings = many_findings(&fixture);
        let brain = fixture.brain(fixture.packs());
        let cancel = CancellationToken::new();
        let mut sink = |_: u64, _: u64, _: &Path| cancel.cancel();

        let output = analyze_with(&findings, &brain, Utc::now(), &cancel, Some(&mut sink));

        assert!(output.cancelled);
        assert_eq!(output.analyzed, ANALYZE_CHUNK, "stops at the next chunk boundary");
        let judged: std::collections::HashSet<Uuid> =
            findings[..output.analyzed].iter().map(|f| f.id).collect();
        assert!(
            output
                .recommendations
                .iter()
                .all(|rec| rec.finding_ids.iter().all(|id| judged.contains(id))),
            "nothing past the cancel point may carry a verdict"
        );
    }

    #[test]
    fn a_pack_whose_variable_is_unset_is_reported_not_hidden() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packs = rules::load_dir(&repo_rules_dir(), &MapEnv::new()).expect("load");
        let protected = ProtectedPaths::empty();
        let cancel = CancellationToken::new();
        let settings = CleanerSettings::default();

        let output = scan(
            &CleanerContext {
                packs: &packs,
                settings: &settings,
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
        let settings = CleanerSettings::default();

        let output = scan(
            &CleanerContext {
                packs: &packs,
                settings: &settings,
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

    /// The same thing through a **junction** rather than a symlink, so it
    /// asserts without Developer Mode. A junction is a reparse point too and
    /// `canonicalize` resolves it identically, which is exactly the property the
    /// walk-root gate depends on (`docs/DECISIONS-NEEDED.md`, architect
    /// resolution before phase 2).
    #[cfg(windows)]
    #[test]
    fn a_walk_root_that_is_a_junction_into_a_protected_location_is_refused() {
        let fixture = Fixture::new();
        let temp = fixture.root.join("AppData/Local/Temp");
        let real_windows = fixture.root.join("Windows");
        let door = temp.join("innocent-junction");

        assert!(
            crate::core::test_support::make_junction(&door, &real_windows),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );

        let protected = fixture.protected();
        assert!(
            protected.reason_for(&door).is_none(),
            "the premise: the junction's own path is not lexically protected"
        );
        match resolve_root(&door, &protected) {
            RootVerdict::Refused(why) => assert!(why.contains("resolves to"), "got `{why}`"),
            RootVerdict::Walk(_) => {
                panic!("a root resolving into a protected tree must be refused")
            }
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
    ///
    /// The last clause is the one review M14 caught this test not checking: it
    /// asserted both rules were *in* the group, which is not the same as either
    /// of them being able to claim anything once they are. See
    /// `a_folded_rule_can_still_claim_entries_under_the_surviving_spelling`.
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

        // …and "tried" has to mean a glob that can actually match. Every rule in
        // the group must recognise a path the walk of this group will produce.
        let entry = groups[0].root.join("probe.tmp");
        for held in &groups[0].rules {
            let candidate = held.match_path(&entry, &groups[0].root);
            assert!(
                held.rule.matches_path(&candidate),
                "`{}` cannot claim `{}`, which is the only kind of path this walk \
                 will ever hand it (it is anchored at `{:?}`)",
                held.rule.spec.id,
                entry.display(),
                held.own_root,
            );
        }
    }

    /// Three shipped packs point at `%TEMP%`. Each walked it separately, which
    /// on a 100 000-file temp folder was ten seconds per extra walk; they now
    /// share one walk, in pack order, so "first pack wins" still holds.
    #[test]
    fn packs_naming_the_same_folder_share_one_walk_in_pack_order() {
        let fixture = Fixture::new();
        let packs = fixture.packs();
        let protected = fixture.protected();
        let mut walks = Vec::new();
        for pack in packs.iter().filter(|pack| pack.spec.module == CLEANER_MODULE_ID) {
            let (groups, _) = plan_walks(pack, &protected);
            share_walks(&mut walks, pack, groups);
        }

        let mut identities: Vec<&str> = walks.iter().map(|walk| walk.identity.as_str()).collect();
        let walked = identities.len();
        identities.sort_unstable();
        identities.dedup();
        assert_eq!(identities.len(), walked, "no folder may be walked twice");

        let shared = walks
            .iter()
            .find(|walk| walk.claims.len() > 1)
            .expect("the fixture's temp folder is a root of more than one pack");
        let order: Vec<usize> = shared
            .claims
            .iter()
            .map(|(pack, _)| packs.iter().position(|p| p.id() == pack.id()).expect("known pack"))
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "claims follow pack order");
        for (_, rules) in &shared.claims {
            let entry = shared.root.join("probe.tmp");
            for held in rules {
                let candidate = held.match_path(&entry, &shared.root);
                assert!(
                    CanonicalPath::lexical(&candidate).is_within(&CanonicalPath::lexical(
                        held.own_root.as_deref().unwrap_or(&shared.root)
                    )),
                    "`{}` must see the walked path in its own spelling",
                    held.rule.spec.id
                );
            }
        }
    }

    /// **Review M14.** `plan_walks` folds walk roots on canonical identity but
    /// keeps the *first* rule's spelling, while `matches_path` is a text glob
    /// anchored to each rule's *own* spelling. Two rules sharing a canonical
    /// root spelled differently therefore left every rule but the first unable
    /// to claim a single entry — silently, because nothing reported it.
    ///
    /// On a stock machine the two spellings are `%TEMP%` (8.3 on any account
    /// whose name does not fit) against `%LOCALAPPDATA%\Temp` (never 8.3). A
    /// junction reproduces the same shape on any machine and needs no privilege.
    ///
    /// The rules are separated by extension rather than by glob shape, so what
    /// is under test is the *path anchor* and nothing else.
    #[cfg(windows)]
    #[test]
    fn a_folded_rule_can_still_claim_entries_under_the_surviving_spelling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let profile = dir.path().to_path_buf();
        let real_temp = profile.join("real/Temp");
        fs::create_dir_all(&real_temp).expect("fixture dirs");

        // The second spelling of the same directory.
        let alias = profile.join("alias");
        assert!(
            crate::core::test_support::make_junction(&alias, &profile.join("real")),
            "`mklink /J` needs no privilege, so a failure here is a real one"
        );
        let alias_temp = alias.join("Temp");

        aged(&real_temp.join("first.aaa"), b"claimed by rule a", 48);
        aged(&real_temp.join("second.bbb"), b"claimed by rule b", 48);

        let pack_json = r#"{
            "id": "folded",
            "name": "Folded Roots",
            "version": 1,
            "module": "cleaner",
            "enabled_by_default": true,
            "description": "Two rules, one directory, two spellings.",
            "rules": [
                {
                    "id": "spelled-directly",
                    "paths": ["%REALTEMP%\\**"],
                    "match": { "min_age_hours": 24, "extensions": ["aaa"] },
                    "score": { "base": 90, "max_bucket": "safe" },
                    "action": "quarantine",
                    "reason": "Anchored at the spelling that wins the fold."
                },
                {
                    "id": "spelled-through-the-junction",
                    "paths": ["%ALIASTEMP%\\**"],
                    "match": { "min_age_hours": 24, "extensions": ["bbb"] },
                    "score": { "base": 90, "max_bucket": "safe" },
                    "action": "quarantine",
                    "reason": "Anchored at the spelling the fold discards."
                }
            ]
        }"#;

        let env = MapEnv::new()
            .with("USERPROFILE", profile.to_string_lossy())
            .with("REALTEMP", real_temp.to_string_lossy())
            .with("ALIASTEMP", alias_temp.to_string_lossy());
        let pack = rules::compile_pack("folded", pack_json, &env).expect("the fixture pack");
        let protected = ProtectedPaths::empty();

        let (groups, refused) = plan_walks(&pack, &protected);
        assert!(refused.is_empty(), "nothing here is refused: {refused:?}");
        assert_eq!(
            groups.len(),
            1,
            "the two spellings name one directory, so one walk: {:?}",
            groups.iter().map(|g| g.root.display().to_string()).collect::<Vec<_>>()
        );
        println!("plan_walks walks: {}", groups[0].root.display());
        for held in &groups[0].rules {
            println!(
                "  rule {:<28} anchored at {}",
                held.rule.spec.id,
                held.own_root
                    .as_deref()
                    .unwrap_or(&groups[0].root)
                    .display()
            );
        }

        let bin = StubRecycleBin::default();
        let cancel = CancellationToken::new();
        let settings = CleanerSettings::default();
        let output = scan(
            &CleanerContext {
                packs: std::slice::from_ref(&pack),
                settings: &settings,
                protected: &protected,
                recycle_bin: &bin,
                now: Utc::now(),
                cancel: &cancel,
            },
            None,
        );

        let claimed_by = |rule_id: &str| {
            output
                .findings
                .iter()
                .filter(|f| {
                    f.meta.get("ruleId").and_then(|v| v.as_str()) == Some(rule_id)
                })
                .map(|f| f.path.file_name().unwrap_or_default().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            claimed_by("spelled-directly"),
            vec!["first.aaa"],
            "the surviving spelling still works: {:?}",
            names(&output.findings)
        );
        assert_eq!(
            claimed_by("spelled-through-the-junction"),
            vec!["second.bbb"],
            "the absorbed rule was made unreachable by the fold: {:?}",
            names(&output.findings)
        );
        assert!(
            output.skipped_rules.is_empty(),
            "no rule was skipped, so nothing should be reported: {:?}",
            output.skipped_rules
        );
    }

    /// The other half of M14: a rule that ends up unable to claim anything must
    /// **say so**. The dead rule in the original report was not in
    /// `skipped_rules`, which is how it stayed invisible.
    ///
    /// Asserted as an invariant over every shipped pack rather than over one
    /// fixture: any rule with walk roots is either in a group or named in
    /// `skipped_rules`, never neither.
    #[test]
    fn a_rule_that_cannot_be_walked_is_always_reported() {
        let fixture = Fixture::new();

        // A protected walk root is the reachable way to make a rule unwalkable:
        // `%WINDIR%` is a protected tree, so `windows-update-leftovers` cannot
        // be walked at all.
        let protected = fixture.protected();
        for pack in fixture.packs() {
            let (groups, refused) = plan_walks(&pack, &protected);
            for rule in &pack.rules {
                if rule.roots().is_empty() {
                    continue;
                }
                let placed = groups
                    .iter()
                    .any(|g| g.rules.iter().any(|h| h.rule.spec.id == rule.spec.id));
                let reported = refused
                    .iter()
                    .any(|line| line.contains(&format!("{}/{}", pack.id(), rule.spec.id)));
                assert!(
                    placed || reported,
                    "`{}/{}` is neither walked nor reported — exactly the silent \
                     disappearance review M14 is about",
                    pack.id(),
                    rule.spec.id
                );
            }
        }

        // And the reporting really does fire for the pack whose root is refused.
        let update_pack = fixture
            .packs()
            .into_iter()
            .find(|pack| pack.id() == "windows-update-leftovers")
            .expect("the pack ships");
        let (groups, refused) = plan_walks(&update_pack, &protected);
        assert!(groups.is_empty(), "%WINDIR% is protected, so nothing is walked");
        assert!(
            refused
                .iter()
                .any(|line| line.contains("software-distribution-download")),
            "got {refused:?}"
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
                Justification::Rule {
                    pack_id: "windows-temp".to_owned(),
                    rule_id: "temp-old".to_owned(),
                },
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
