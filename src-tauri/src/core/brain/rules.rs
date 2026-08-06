//! Rule-pack loading and validation (`docs/RULES-FORMAT.md`).
//!
//! Rule packs are community-editable JSON, which means the parser is a trust
//! boundary. It rejects loudly and specifically — pack name plus the dotted
//! field path — because "silently skipped one rule" is how a cleaner quietly
//! stops cleaning, and "silently accepted a broken rule" is how it starts
//! deleting the wrong thing.
//!
//! Phase 1 deliberately implements no scoring. A rule declares a bucket
//! (`score.max_bucket`) and that is the bucket, subject to the brain's hard
//! gates. `score.base` is parsed and validated so packs written today stay
//! valid when phase 2 turns it into a real score, but it is not consulted yet.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, Result};
use crate::core::platform::{env, EnvResolver};
use crate::core::types::RiskBucket;

/// Bucket ceiling as written in a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxBucket {
    /// Green.
    Safe,
    /// Yellow.
    NeedsReview,
    /// Red.
    DoNotDelete,
}

impl From<MaxBucket> for RiskBucket {
    fn from(value: MaxBucket) -> Self {
        match value {
            MaxBucket::Safe => Self::Safe,
            MaxBucket::NeedsReview => Self::NeedsReview,
            MaxBucket::DoNotDelete => Self::DoNotDelete,
        }
    }
}

/// What a matching rule proposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    /// Move to quarantine.
    Quarantine,
    /// Report only; never offered as an action.
    InfoOnly,
    /// Empty the recycle bin through the shell API. Only legal in the
    /// `recycle-bin` pack, which the validator enforces.
    EmptyRecycleBin,
}

/// Optional `match` constraints, AND-ed together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct MatchCriteria {
    /// Only files older than this many hours (by modification time).
    pub min_age_hours: Option<u32>,
    /// Only files at least this large.
    pub min_size_bytes: Option<u64>,
    /// Only these extensions, without the dot, compared case-insensitively.
    pub extensions: Option<Vec<String>>,
    /// Skip or downgrade while this process is running. Parsed and validated in
    /// phase 1; the process check itself arrives with the browser packs.
    pub owner_process_absent: Option<String>,
}

/// Bucket and starting confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct RuleScore {
    /// 0-100 starting confidence. Not consulted until phase 2.
    pub base: u8,
    /// Ceiling; heuristics may lower a bucket, never raise it above this.
    pub max_bucket: MaxBucket,
}

/// One rule inside a pack, exactly as written in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct RuleSpec {
    /// Unique within the pack.
    pub id: String,
    /// Glob patterns; `%VARS%` expanded at load time.
    pub paths: Vec<String>,
    /// Patterns that veto a match.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Extra conditions. Named `match_` because `match` is a Rust keyword; the
    /// JSON field is plain `match`.
    #[serde(rename = "match", default)]
    pub match_: MatchCriteria,
    /// Bucket and base score.
    pub score: RuleScore,
    /// Proposed action.
    pub action: RuleAction,
    /// User-facing justification, shipped to the UI verbatim.
    pub reason: String,
    /// Whether the rule needs elevation. Required for absolute paths outside
    /// the user profile (RULES-FORMAT.md).
    #[serde(default)]
    pub requires_admin: bool,
}

/// A pack file as written on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct RulePackSpec {
    /// Editor hint only.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// kebab-case pack id; must match the file stem.
    pub id: String,
    /// English display name.
    pub name: String,
    /// Bumped when rules change meaningfully.
    pub version: u32,
    /// Module the pack belongs to.
    pub module: String,
    /// Whether the pack is on when the user has never touched it.
    pub enabled_by_default: bool,
    /// One-line English description.
    pub description: String,
    /// The rules.
    pub rules: Vec<RuleSpec>,
}

/// A rule with its patterns compiled and expanded.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    /// The rule as written.
    pub spec: RuleSpec,
    /// Pack this rule came from.
    pub pack_id: String,
    /// Expanded, compiled include patterns.
    include: Vec<GlobMatcher>,
    /// Expanded, compiled exclude patterns.
    exclude: Vec<GlobMatcher>,
    /// Longest literal directory prefix of each include pattern — the scanner
    /// walks these instead of the whole disk.
    roots: Vec<PathBuf>,
}

impl CompiledRule {
    /// Directories the scanner needs to walk for this rule.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether `path` matches an include pattern and no exclude pattern.
    pub fn matches_path(&self, path: &Path) -> bool {
        // globset works on `/`-separated text; Windows paths arrive either way.
        let normalised = path.to_string_lossy().replace('\\', "/").to_lowercase();
        let candidate = Path::new(&normalised);
        self.include.iter().any(|glob| glob.is_match(candidate))
            && !self.exclude.iter().any(|glob| glob.is_match(candidate))
    }

    /// The bucket this rule declares.
    pub fn bucket(&self) -> RiskBucket {
        self.spec.score.max_bucket.into()
    }
}

/// A validated, compiled pack.
#[derive(Debug, Clone)]
pub struct RulePack {
    /// The pack as written.
    pub spec: RulePackSpec,
    /// Its rules, compiled.
    pub rules: Vec<CompiledRule>,
    /// Rules whose paths referenced an unset variable, with the variable name.
    /// Reported rather than silently dropped.
    pub skipped: Vec<(String, String)>,
}

impl RulePack {
    /// The pack id.
    pub fn id(&self) -> &str {
        &self.spec.id
    }
}

/// Parse and validate one pack from JSON text.
///
/// `pack_name` is used in errors; pass the file stem so the message points at a
/// file the user can open.
///
/// # Errors
/// Returns [`CoreError::RulePack`] naming the pack and the offending field.
pub fn parse_pack(pack_name: &str, json: &str) -> Result<RulePackSpec> {
    let invalid = |field: &str, message: String| CoreError::RulePack {
        pack: pack_name.to_owned(),
        field: field.to_owned(),
        message,
    };

    let spec: RulePackSpec = serde_json::from_str(json)
        .map_err(|err| invalid("<document>", format!("not valid rule-pack JSON: {err}")))?;

    if spec.id.trim().is_empty() {
        return Err(invalid("id", "must not be empty".to_owned()));
    }
    if !is_kebab_case(&spec.id) {
        return Err(invalid("id", format!("`{}` is not kebab-case", spec.id)));
    }
    if spec.name.trim().is_empty() {
        return Err(invalid("name", "must not be empty".to_owned()));
    }
    if spec.description.trim().is_empty() {
        return Err(invalid("description", "must not be empty".to_owned()));
    }
    if spec.version == 0 {
        return Err(invalid("version", "must be 1 or higher".to_owned()));
    }
    if spec.module.trim().is_empty() {
        return Err(invalid("module", "must not be empty".to_owned()));
    }
    if spec.rules.is_empty() {
        return Err(invalid(
            "rules",
            "a pack with no rules does nothing".to_owned(),
        ));
    }

    let mut seen = BTreeSet::new();
    for (index, rule) in spec.rules.iter().enumerate() {
        let field = |name: &str| format!("rules[{index}].{name}");

        if rule.id.trim().is_empty() {
            return Err(invalid(&field("id"), "must not be empty".to_owned()));
        }
        if !seen.insert(rule.id.clone()) {
            return Err(invalid(
                &field("id"),
                format!("`{}` is declared twice in this pack", rule.id),
            ));
        }
        if rule.paths.is_empty() {
            return Err(invalid(
                &field("paths"),
                "a rule must match at least one path pattern".to_owned(),
            ));
        }
        // `reason` ships to the UI verbatim, so an empty one would be an
        // unexplained suggestion — a CLAUDE.md hard-rule violation.
        if rule.reason.trim().is_empty() {
            return Err(invalid(
                &field("reason"),
                "every rule must explain itself in plain language".to_owned(),
            ));
        }
        if rule.score.base > 100 {
            return Err(invalid(
                &field("score.base"),
                format!("{} is above 100", rule.score.base),
            ));
        }
        if rule.action == RuleAction::EmptyRecycleBin && spec.id != "recycle-bin" {
            return Err(invalid(
                &field("action"),
                "`empty_recycle_bin` is only valid in the `recycle-bin` pack".to_owned(),
            ));
        }
        if rule.action == RuleAction::Quarantine && rule.score.max_bucket == MaxBucket::DoNotDelete
        {
            return Err(invalid(
                &field("action"),
                "a `do_not_delete` rule must not propose an action".to_owned(),
            ));
        }
        if let Some(extensions) = &rule.match_.extensions {
            if extensions.iter().any(|e| e.starts_with('.')) {
                return Err(invalid(
                    &field("match.extensions"),
                    "extensions are written without the leading dot".to_owned(),
                ));
            }
        }

        for (pattern_index, pattern) in rule.paths.iter().chain(&rule.exclude).enumerate() {
            if pattern.trim().is_empty() {
                return Err(invalid(
                    &field(&format!("paths[{pattern_index}]")),
                    "a pattern must not be empty".to_owned(),
                ));
            }
            // A pattern that can name a protected location in a spelling the
            // protected-path list does not recognise is rejected outright: a
            // pack is community-editable, and "walk %TEMP%\..\..\..\Windows"
            // must never be a thing a pull request can say (security review
            // C1c).
            if let Some(message) = unsafe_pattern_shape(pattern) {
                return Err(invalid(&field(&format!("paths[{pattern_index}]")), message));
            }
            // Compile-check every pattern now so a typo fails at load, not
            // halfway through a scan.
            compile_glob(pattern)
                .map_err(|message| invalid(&field(&format!("paths[{pattern_index}]")), message))?;
        }

        // An absolute or network pattern reaches outside the user profile.
        // RULES-FORMAT.md requires the rule to say so. This is the shape-only
        // half of the check; `compile_spec` runs the other half once `%VARS%`
        // are expanded and the profile root is known (security review M8).
        if !rule.requires_admin {
            if let Some(pattern) = rule
                .paths
                .iter()
                .find(|pattern| reaches_outside_profile(pattern))
            {
                return Err(invalid(
                    &field("requires_admin"),
                    format!(
                        "`{pattern}` reaches outside the user profile, so the rule must set `requires_admin`"
                    ),
                ));
            }
        }
    }

    Ok(spec)
}

/// Parse, validate and compile a pack, expanding `%VARS%` through `env`.
///
/// # Errors
/// See [`parse_pack`], plus pattern-compilation failures.
pub fn compile_pack(pack_name: &str, json: &str, env: &dyn EnvResolver) -> Result<RulePack> {
    let spec = parse_pack(pack_name, json)?;
    compile_spec(spec, env)
}

/// Compile an already-validated spec.
///
/// # Errors
/// Returns [`CoreError::RulePack`] if a pattern fails to compile after
/// expansion.
pub fn compile_spec(spec: RulePackSpec, env: &dyn EnvResolver) -> Result<RulePack> {
    let mut rules = Vec::with_capacity(spec.rules.len());
    let mut skipped = Vec::new();
    // Known only at compile time, and only on a real machine: without it the
    // containment half of the `requires_admin` check cannot be evaluated and
    // the shape-only half in `parse_pack` is all there is.
    let profile = env.var("USERPROFILE").map(PathBuf::from);

    for rule in &spec.rules {
        let mut include = Vec::new();
        let mut roots = Vec::new();
        let mut missing_var = None;

        for pattern in &rule.paths {
            match env::expand(pattern, env) {
                env::Expansion::Expanded(expanded) => {
                    let glob = compile_glob(&expanded).map_err(|message| CoreError::RulePack {
                        pack: spec.id.clone(),
                        field: format!("rules.{}.paths", rule.id),
                        message,
                    })?;
                    let root = literal_prefix(&expanded);
                    // `%TEMP%` is only inside the profile on the machine that
                    // says so. A pack that expands to somewhere else has to
                    // declare `requires_admin`, whatever the pattern looked
                    // like before expansion (security review M8).
                    if !rule.requires_admin && !is_shell_namespace(&expanded) {
                        if let Some(profile) = &profile {
                            if !is_inside(profile, &root) {
                                return Err(CoreError::RulePack {
                                    pack: spec.id.clone(),
                                    field: format!("rules.{}.requires_admin", rule.id),
                                    message: format!(
                                        "`{pattern}` expands to `{}`, which is outside the user profile `{}`; the rule must set `requires_admin`",
                                        root.display(),
                                        profile.display()
                                    ),
                                });
                            }
                        }
                    }
                    roots.push(root);
                    include.push(glob);
                }
                env::Expansion::Missing(name) => {
                    missing_var = Some(name);
                    break;
                }
            }
        }

        // A rule whose `%APPDATA%` is unset on this machine cannot match
        // anything. Record it so the UI can say so instead of pretending the
        // pack ran in full.
        if let Some(name) = missing_var {
            tracing::warn!(
                pack = spec.id.as_str(),
                rule = rule.id.as_str(),
                variable = name.as_str(),
                "rule skipped: environment variable is not set"
            );
            skipped.push((rule.id.clone(), name));
            continue;
        }

        let mut exclude = Vec::new();
        for pattern in &rule.exclude {
            if let env::Expansion::Expanded(expanded) = env::expand(pattern, env) {
                exclude.push(
                    compile_glob(&expanded).map_err(|message| CoreError::RulePack {
                        pack: spec.id.clone(),
                        field: format!("rules.{}.exclude", rule.id),
                        message,
                    })?,
                );
            }
        }

        rules.push(CompiledRule {
            spec: rule.clone(),
            pack_id: spec.id.clone(),
            include,
            exclude,
            roots,
        });
    }

    Ok(RulePack {
        spec,
        rules,
        skipped,
    })
}

/// Load every `*.json` pack in `dir` (ignoring the schema file).
///
/// # Errors
/// Fails on the first invalid pack: a broken pack is a bug to fix, not a thing
/// to route around.
pub fn load_dir(dir: &Path, env: &dyn EnvResolver) -> Result<Vec<RulePack>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|err| CoreError::RulePack {
            pack: dir.display().to_string(),
            field: "<directory>".to_owned(),
            message: format!("cannot read the rules directory: {err}"),
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|e| e.to_str()) == Some("json")
                && path.file_name().and_then(|n| n.to_str()) != Some("rule-pack.schema.json")
        })
        .collect();
    // Deterministic order: the same disk produces the same findings order.
    paths.sort();

    let mut packs = Vec::with_capacity(paths.len());
    for path in paths {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let json = std::fs::read_to_string(&path).map_err(|err| CoreError::RulePack {
            pack: stem.clone(),
            field: "<file>".to_owned(),
            message: format!("cannot read `{}`: {err}", path.display()),
        })?;
        let pack = compile_pack(&stem, &json, env)?;
        if pack.spec.id != stem {
            return Err(CoreError::RulePack {
                pack: stem.clone(),
                field: "id".to_owned(),
                message: format!(
                    "the pack id `{}` does not match the file name `{stem}.json`",
                    pack.spec.id
                ),
            });
        }
        packs.push(pack);
    }
    Ok(packs)
}

/// Whether a pattern, as written, already reaches outside the user profile.
///
/// Character-based, not byte-indexed: a pattern starting with a multi-byte
/// character used to make `pattern.as_bytes()[1] == b':'` read the middle of a
/// codepoint and answer no (security review M8).
fn reaches_outside_profile(pattern: &str) -> bool {
    let normalised = pattern.replace('/', "\\");

    // A network share and the verbatim namespace are outside every profile.
    // (The verbatim form is rejected outright elsewhere; this keeps the answer
    // correct if that ever relaxes.)
    if normalised.starts_with("\\\\") {
        return true;
    }
    // `%VAR%\...` is judged after expansion, in `compile_spec`.
    if normalised.starts_with('%') {
        return false;
    }
    // `C:\...` — a drive-absolute path written out longhand.
    let mut chars = normalised.chars();
    matches!((chars.next(), chars.next()), (Some(letter), Some(':')) if letter.is_alphabetic())
}

/// Whether a pattern names a shell namespace object rather than a directory.
/// `shell:RecycleBinFolder` has no filesystem location to contain, so profile
/// containment is not a question that can be asked about it.
fn is_shell_namespace(pattern: &str) -> bool {
    pattern.len() >= 6 && pattern[..6].eq_ignore_ascii_case("shell:")
}

/// Whether `path` sits inside `profile`, comparing whole components so
/// `C:\Users\dev2` is not read as inside `C:\Users\dev`.
fn is_inside(profile: &Path, path: &Path) -> bool {
    let parts = |value: &Path| -> Vec<String> {
        value
            .to_string_lossy()
            .replace('/', "\\")
            .split('\\')
            .filter(|part| !part.is_empty() && *part != ".")
            .map(|part| part.trim_end_matches(['.', ' ']).to_lowercase())
            .filter(|part| !part.is_empty())
            .collect()
    };
    let (profile, path) = (parts(profile), parts(path));
    !profile.is_empty() && path.len() >= profile.len() && path[..profile.len()] == profile[..]
}

/// Why a pattern is refused whatever else it says, or `None` when its shape is
/// fine.
///
/// Every one of these is a way to name a place while spelling it as somewhere
/// else, which is exactly what the protected-path list cannot see through
/// without touching the disk. A pack has no legitimate need for any of them:
/// every location a rule wants is reachable by its ordinary long name on a
/// local drive.
fn unsafe_pattern_shape(pattern: &str) -> Option<String> {
    let normalised = pattern.replace('/', "\\");

    // Anything starting with `\\` at all: the verbatim and device prefixes, and
    // UNC. `\\localhost\C$\Windows\**` is `C:\Windows\**` spelled as a network
    // share, it resolves from a non-elevated process, and both the lexical list
    // and its canonical form `\\?\UNC\localhost\c$\...` used to miss it
    // (security review C2c). Phase 1 rules are local-drive only.
    if normalised.starts_with("\\\\") {
        return Some(format!(
            "`{pattern}` starts with `\\\\` (a UNC share, or a `\\\\?\\` or `\\\\.\\` prefix); \
             write the plain local path instead"
        ));
    }
    if normalised.starts_with("\\??\\") {
        return Some(format!(
            "`{pattern}` uses the `\\??\\` NT object namespace; write the plain local path instead"
        ));
    }

    for part in normalised.split('\\') {
        if part == ".." {
            return Some(format!(
                "`{pattern}` climbs with `..`; write the folder it means directly"
            ));
        }
        if is_short_name(part) {
            return Some(format!(
                "`{pattern}` uses the 8.3 short name `{part}`; write the long name instead"
            ));
        }
    }
    None
}

/// Whether a path component is an 8.3 short name such as `PROGRA~1`.
fn is_short_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part);
    stem.rsplit_once('~')
        .is_some_and(|(_, tail)| !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()))
}

fn compile_glob(pattern: &str) -> std::result::Result<GlobMatcher, String> {
    // Windows paths are case-insensitive and use backslashes; globset wants
    // `/`. Lower-casing both sides is simpler and faster than a case-insensitive
    // matcher, and rule patterns are ASCII by construction.
    let normalised = pattern.replace('\\', "/").to_lowercase();
    Glob::new(&normalised)
        .map(|glob| glob.compile_matcher())
        .map_err(|err| format!("`{pattern}` is not a valid pattern: {err}"))
}

/// The longest leading directory of a pattern that contains no glob syntax.
/// `C:\Temp\**\*.log` gives `C:\Temp`, which is what the scanner walks.
fn literal_prefix(pattern: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for part in pattern.split(['\\', '/']) {
        if part.contains(['*', '?', '[', '{']) {
            break;
        }
        if prefix.as_os_str().is_empty() {
            // Keep the drive's separator: `C:` alone means "current dir on C:".
            prefix.push(format!("{part}\\"));
        } else {
            prefix.push(part);
        }
    }
    prefix
}

fn is_kebab_case(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::MapEnv;

    const VALID: &str = r#"{
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
                "exclude": ["%TEMP%\\**\\*.lock"],
                "match": { "min_age_hours": 24 },
                "score": { "base": 90, "max_bucket": "safe" },
                "action": "quarantine",
                "reason": "Temporary file older than 24h."
            }
        ]
    }"#;

    fn env() -> MapEnv {
        MapEnv::new().with("TEMP", "C:\\Temp")
    }

    fn with_rule_field(field: &str, value: &str) -> String {
        VALID.replace(
            r#""score": { "base": 90, "max_bucket": "safe" }"#,
            &format!(r#""score": {{ "base": 90, "max_bucket": "safe" }}, "{field}": {value}"#),
        )
    }

    #[test]
    fn a_valid_pack_parses() {
        let spec = parse_pack("windows-temp", VALID).expect("valid pack");
        assert_eq!(spec.id, "windows-temp");
        assert_eq!(spec.rules.len(), 1);
        assert_eq!(spec.rules[0].match_.min_age_hours, Some(24));
        assert_eq!(spec.rules[0].score.max_bucket, MaxBucket::Safe);
        assert_eq!(spec.rules[0].action, RuleAction::Quarantine);
    }

    /// Every failure names the pack and the field. Table-driven so a new
    /// validation rule is one line here.
    #[test]
    fn malformed_packs_are_rejected_loudly() {
        let cases: &[(&str, String, &str)] = &[
            ("not json at all", "{".to_owned(), "<document>"),
            (
                "unknown top-level field",
                VALID.replace(r#""version": 1"#, r#""version": 1, "surprise": true"#),
                "<document>",
            ),
            (
                "unknown rule field",
                with_rule_field("surprise", "true"),
                "<document>",
            ),
            ("blank id", VALID.replace("windows-temp", " "), "id"),
            (
                "non-kebab id",
                VALID.replace(r#""id": "windows-temp""#, r#""id": "Windows_Temp""#),
                "id",
            ),
            (
                "version zero",
                VALID.replace(r#""version": 1"#, r#""version": 0"#),
                "version",
            ),
            (
                "no rules",
                VALID.replace(
                    VALID.split_once("\"rules\":").expect("rules").1,
                    " []\n    }",
                ),
                "rules",
            ),
            (
                "blank reason",
                VALID.replace("Temporary file older than 24h.", "  "),
                "rules[0].reason",
            ),
            (
                "score above 100",
                VALID.replace(r#""base": 90"#, r#""base": 101"#),
                "rules[0].score.base",
            ),
            (
                "unknown bucket",
                VALID.replace(r#""max_bucket": "safe""#, r#""max_bucket": "green""#),
                "<document>",
            ),
            (
                "unknown action",
                VALID.replace(r#""action": "quarantine""#, r#""action": "delete""#),
                "<document>",
            ),
            (
                "empty path list",
                VALID.replace(r#"["%TEMP%\\**"]"#, "[]"),
                "rules[0].paths",
            ),
            (
                "broken glob",
                VALID.replace(r#"%TEMP%\\**"#, r#"%TEMP%\\[a-"#),
                "rules[0].paths[0]",
            ),
            (
                "extension with a dot",
                VALID.replace(
                    r#""match": { "min_age_hours": 24 }"#,
                    r#""match": { "extensions": [".log"] }"#,
                ),
                "rules[0].match.extensions",
            ),
            (
                "action on a red rule",
                VALID.replace(
                    r#""max_bucket": "safe""#,
                    r#""max_bucket": "do_not_delete""#,
                ),
                "rules[0].action",
            ),
            (
                "recycle-bin action in the wrong pack",
                VALID.replace(
                    r#""action": "quarantine""#,
                    r#""action": "empty_recycle_bin""#,
                ),
                "rules[0].action",
            ),
            (
                "absolute path without requires_admin",
                VALID.replace(r#"%TEMP%\\**"#, r#"C:\\Windows\\Temp\\**"#),
                "rules[0].requires_admin",
            ),
            (
                // Since C2 a UNC pattern never gets as far as the
                // `requires_admin` question: the shape check refuses it first,
                // and `requires_admin: true` would not have saved it.
                "UNC path, refused by shape before requires_admin is asked",
                VALID.replace(r#"%TEMP%\\**"#, r#"\\\\server\\share\\**"#),
                "rules[0].paths[0]",
            ),
        ];

        for (label, json, expected_field) in cases {
            let err = parse_pack("windows-temp", json)
                .err()
                .unwrap_or_else(|| panic!("`{label}` should have been rejected"));
            match err {
                CoreError::RulePack { pack, field, .. } => {
                    assert_eq!(pack, "windows-temp", "`{label}` must name the pack");
                    assert_eq!(&field, expected_field, "`{label}` must name the field");
                }
                other => panic!("`{label}` produced the wrong error: {other}"),
            }
        }
    }

    /// Security review C1c: a community pack must not be able to name a place
    /// in a spelling the protected-path list cannot see through.
    #[test]
    fn patterns_that_disguise_their_destination_are_rejected() {
        let cases: &[(&str, &str)] = &[
            (
                "parent traversal",
                r#"%TEMP%\\..\\..\\..\\..\\Windows\\System32\\**"#,
            ),
            (
                "parent traversal, forward slashes",
                r#"%TEMP%/../../Windows/**"#,
            ),
            ("bare parent traversal", r#"%TEMP%\\..\\**"#),
            ("verbatim prefix", r#"\\\\?\\C:\\Windows\\**"#),
            ("device prefix", r#"\\\\.\\C:\\Windows\\**"#),
            // Security review C2c. `\\localhost\C$\Windows` *is* `C:\Windows`,
            // it resolves from a non-elevated process, and neither the lexical
            // list nor its canonical form `\\?\UNC\localhost\c$\...` saw it.
            (
                "administrative share by host name",
                r#"\\\\localhost\\C$\\Windows\\**"#,
            ),
            (
                "administrative share by loopback address",
                r#"\\\\127.0.0.1\\C$\\Windows\\**"#,
            ),
            ("verbatim UNC", r#"\\\\?\\UNC\\localhost\\c$\\Windows\\**"#),
            (
                "device namespace by volume",
                r#"\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\Windows\\**"#,
            ),
            ("an ordinary remote share", r#"\\\\server\\share\\**"#),
            ("NT object namespace", r#"\\??\\C:\\Windows\\**"#),
            ("8.3 short name", r#"%TEMP%\\PROGRA~1\\**"#),
            (
                "8.3 short name with an extension",
                r#"%TEMP%\\DOCUME~1.TXT"#,
            ),
        ];

        for (label, pattern) in cases {
            let json = VALID.replace(r#"%TEMP%\\**"#, pattern);
            let err = parse_pack("windows-temp", &json)
                .err()
                .unwrap_or_else(|| panic!("`{label}` should have been rejected"));
            match err {
                CoreError::RulePack { pack, field, .. } => {
                    assert_eq!(pack, "windows-temp", "`{label}` must name the pack");
                    assert_eq!(field, "rules[0].paths[0]", "`{label}` must name the field");
                }
                other => panic!("`{label}` produced the wrong error: {other}"),
            }
        }
    }

    /// The same rejection applies to `exclude`, which is walked by the same
    /// loop and would otherwise be a second way in.
    #[test]
    fn an_exclude_pattern_is_held_to_the_same_shape_rules() {
        let json = VALID.replace(r#"%TEMP%\\**\\*.lock"#, r#"%TEMP%\\..\\**"#);
        assert!(matches!(
            parse_pack("windows-temp", &json),
            Err(CoreError::RulePack { .. })
        ));
    }

    #[test]
    fn ordinary_patterns_that_merely_contain_a_tilde_or_dots_still_parse() {
        for pattern in [
            r#"%TEMP%\\my~file\\**"#,
            r#"%TEMP%\\a..b\\**"#,
            r#"%TEMP%\\backup~\\**"#,
        ] {
            let json = VALID.replace(r#"%TEMP%\\**"#, pattern);
            parse_pack("windows-temp", &json)
                .unwrap_or_else(|err| panic!("`{pattern}` should have parsed: {err}"));
        }
    }

    /// Security review M8: `%VAR%` is not a promise about where a rule lands.
    #[test]
    fn a_variable_that_expands_outside_the_profile_needs_requires_admin() {
        let env = MapEnv::new()
            .with("USERPROFILE", "C:\\Users\\dev")
            .with("TEMP", "D:\\SomewhereElse");

        let err = compile_pack("windows-temp", VALID, &env)
            .expect_err("an expansion outside the profile must be refused");
        match err {
            CoreError::RulePack { field, message, .. } => {
                assert_eq!(field, "rules.user-temp-files.requires_admin");
                assert!(message.contains("D:\\SomewhereElse"), "got `{message}`");
            }
            other => panic!("wrong error: {other}"),
        }

        // Inside the profile, the same pack compiles.
        let inside = MapEnv::new()
            .with("USERPROFILE", "C:\\Users\\dev")
            .with("TEMP", "C:\\Users\\dev\\AppData\\Local\\Temp");
        assert!(compile_pack("windows-temp", VALID, &inside).is_ok());
    }

    #[test]
    fn a_sibling_profile_is_not_inside_the_profile() {
        assert!(is_inside(
            Path::new("C:\\Users\\dev"),
            Path::new("C:\\Users\\dev\\AppData\\Local\\Temp")
        ));
        assert!(is_inside(
            Path::new("C:\\Users\\dev"),
            Path::new("c:/users/DEV")
        ));
        assert!(!is_inside(
            Path::new("C:\\Users\\dev"),
            Path::new("C:\\Users\\dev2\\Temp")
        ));
        assert!(!is_inside(
            Path::new("C:\\Users\\dev"),
            Path::new("C:\\Windows")
        ));
    }

    #[test]
    fn the_recycle_bin_pack_is_not_asked_where_it_lives() {
        assert!(is_shell_namespace("shell:RecycleBinFolder"));
        assert!(is_shell_namespace("SHELL:recyclebinfolder"));
        assert!(!is_shell_namespace("C:\\shell"));
        assert!(!reaches_outside_profile("shell:RecycleBinFolder"));
    }

    #[test]
    fn duplicate_rule_ids_are_rejected() {
        let two_rules = VALID.replace(
            "\"rules\": [",
            "\"rules\": [
            {
                \"id\": \"user-temp-files\",
                \"paths\": [\"%TEMP%\\\\**\"],
                \"score\": { \"base\": 90, \"max_bucket\": \"safe\" },
                \"action\": \"quarantine\",
                \"reason\": \"duplicate id\"
            },",
        );
        let err = parse_pack("windows-temp", &two_rules).expect_err("duplicate ids");
        assert!(matches!(err, CoreError::RulePack { field, .. } if field == "rules[1].id"));
    }

    #[test]
    fn the_error_message_reads_like_a_bug_report() {
        let err = parse_pack(
            "windows-temp",
            &VALID.replace(r#""base": 90"#, r#""base": 200"#),
        )
        .expect_err("invalid");
        let message = err.to_string();
        assert!(message.contains("windows-temp"), "got `{message}`");
        assert!(message.contains("score.base"), "got `{message}`");
        assert!(message.contains("200"), "got `{message}`");
    }

    #[test]
    fn compiling_expands_variables_and_finds_the_walk_roots() {
        let pack = compile_pack("windows-temp", VALID, &env()).expect("compile");
        assert_eq!(pack.rules.len(), 1);
        assert!(pack.skipped.is_empty());
        assert_eq!(pack.rules[0].roots(), [PathBuf::from("C:\\Temp")]);
    }

    #[test]
    fn a_rule_with_an_unset_variable_is_skipped_and_reported() {
        let pack = compile_pack("windows-temp", VALID, &MapEnv::new()).expect("compile");
        assert!(pack.rules.is_empty());
        assert_eq!(
            pack.skipped,
            vec![("user-temp-files".to_owned(), "TEMP".to_owned())]
        );
    }

    #[test]
    fn matching_is_case_insensitive_and_separator_agnostic() {
        let pack = compile_pack("windows-temp", VALID, &env()).expect("compile");
        let rule = &pack.rules[0];

        assert!(rule.matches_path(Path::new("C:\\Temp\\a.log")));
        assert!(rule.matches_path(Path::new("c:\\temp\\NESTED\\B.TMP")));
        assert!(rule.matches_path(Path::new("C:/Temp/forward/slash.txt")));
        assert!(!rule.matches_path(Path::new("C:\\Other\\a.log")));
    }

    #[test]
    fn exclude_patterns_veto_a_match() {
        let pack = compile_pack("windows-temp", VALID, &env()).expect("compile");
        let rule = &pack.rules[0];
        assert!(rule.matches_path(Path::new("C:\\Temp\\sub\\a.txt")));
        assert!(!rule.matches_path(Path::new("C:\\Temp\\sub\\a.lock")));
    }

    #[test]
    fn literal_prefixes_stop_at_the_first_wildcard() {
        assert_eq!(literal_prefix("C:\\Temp\\**"), PathBuf::from("C:\\Temp"));
        assert_eq!(
            literal_prefix("C:\\Temp\\logs\\*.log"),
            PathBuf::from("C:\\Temp\\logs")
        );
        assert_eq!(literal_prefix("C:\\Temp"), PathBuf::from("C:\\Temp"));
        assert_eq!(literal_prefix("C:\\**"), PathBuf::from("C:\\"));
    }

    #[test]
    fn max_bucket_maps_onto_the_shared_risk_bucket() {
        assert_eq!(RiskBucket::from(MaxBucket::Safe), RiskBucket::Safe);
        assert_eq!(
            RiskBucket::from(MaxBucket::NeedsReview),
            RiskBucket::NeedsReview
        );
        assert_eq!(
            RiskBucket::from(MaxBucket::DoNotDelete),
            RiskBucket::DoNotDelete
        );
    }

    #[test]
    fn loading_a_directory_rejects_a_mismatched_file_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("wrong-name.json"), VALID).expect("write");
        let err = load_dir(dir.path(), &env()).expect_err("id must match the file name");
        assert!(matches!(err, CoreError::RulePack { field, .. } if field == "id"));
    }

    #[test]
    fn loading_a_directory_ignores_the_schema_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("windows-temp.json"), VALID).expect("write");
        std::fs::write(dir.path().join("rule-pack.schema.json"), "{ not json").expect("write");

        let packs = load_dir(dir.path(), &env()).expect("load");
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id(), "windows-temp");
    }

    #[test]
    fn one_broken_pack_fails_the_whole_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("windows-temp.json"), VALID).expect("write");
        std::fs::write(dir.path().join("broken.json"), "{").expect("write");
        assert!(load_dir(dir.path(), &env()).is_err());
    }
}
