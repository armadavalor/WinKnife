//! File unlocker (module spec 07): "which program has this file open?"
//!
//! The answer comes from the Windows Restart Manager
//! ([`crate::core::platform::restart_manager`]). Querying is read-only. The one
//! action is a *polite* close request for window and console programs — the
//! same message Windows sends at sign-out, which a program with unsaved work
//! can answer by asking the user. Nothing here forces anything: no
//! `RmForceShutdown`, no `TerminateProcess`, no handle closing.
//!
//! ## Attribution
//!
//! The Restart Manager says which programs hold *any* registered file, not
//! which file each one holds. For up to [`PER_FILE_ATTRIBUTION_LIMIT`] files
//! the module asks once per file, so every row can name its files. Above that,
//! one session covers them all (in batches of 500) and a row says it holds
//! "one or more of these files" — 5 000 sessions would be slow and the names
//! would not fit on screen anyway.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{
    close_policy, AppKind, CloseOutcome, LockInfo, RestartManager, RmError, UniqueProcess,
};
use crate::core::scanner::{walk_with, CancellationToken, WalkOptions};

/// Files taken from inside picked folders, at most.
pub const FOLDER_FILE_CAP: usize = 5_000;

/// Up to this many files, each is queried on its own so rows can name them.
pub const PER_FILE_ATTRIBUTION_LIMIT: usize = 50;

/// Held files listed on a row; the rest are a count.
pub const HELD_FILES_SHOWN: usize = 3;

/// The module: no scan, no execute — its two commands are its whole surface.
#[derive(Debug, Default)]
pub struct FileUnlockerModule;

impl ToolModule for FileUnlockerModule {
    fn id(&self) -> &'static str {
        "file-unlocker"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "File Unlocker".to_owned(),
            description:
                "Find out which program has a file open, and ask it to close politely."
                    .to_owned(),
            icon: "unlock".to_owned(),
            category: ModuleCategory::Files,
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: false,
            },
        }
    }
}

/// A picked path that was not checked, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedPath {
    /// The path as picked.
    pub path: String,
    /// Human-readable reason.
    pub reason: String,
}

/// The files a query covers, after folders were expanded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Targets {
    /// Every file to ask about, de-duplicated, in pick order.
    pub files: Vec<PathBuf>,
    /// Whether a folder held more files than [`FOLDER_FILE_CAP`].
    pub capped: bool,
    /// Picked paths that were not checked.
    pub skipped: Vec<SkippedPath>,
    /// Entries inside folders that could not be read.
    pub unreadable: usize,
}

/// Expand picked paths into files. Folders are walked without following any
/// reparse point, and the walk stops after `cap` files.
pub fn expand_targets(paths: &[PathBuf], cap: usize) -> Targets {
    let mut targets = Targets::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut folder_files = 0usize;

    for path in paths {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            targets.skipped.push(skipped(path, "It is not there any more."));
            continue;
        };
        if metadata.file_type().is_symlink() {
            targets.skipped.push(skipped(
                path,
                "It is a link to somewhere else; WinKnife does not follow links. Pick the target instead.",
            ));
            continue;
        }
        if metadata.is_file() {
            if seen.insert(path.clone()) {
                targets.files.push(path.clone());
            }
            continue;
        }

        let token = CancellationToken::new();
        let options = WalkOptions {
            report_links: false,
            accurate_timestamps: false,
        };
        let summary = walk_with(path, options, &token, &mut |entry| {
            if entry.is_dir || entry.is_link {
                return;
            }
            if folder_files >= cap {
                targets.capped = true;
                token.cancel();
                return;
            }
            if seen.insert(entry.path.clone()) {
                targets.files.push(entry.path.clone());
                folder_files += 1;
            }
        });
        targets.unreadable += summary.unreadable.len();
    }
    targets
}

fn skipped(path: &Path, reason: &str) -> SkippedPath {
    SkippedPath {
        path: path.display().to_string(),
        reason: reason.to_owned(),
    }
}

/// One program holding one or more of the files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockRow {
    /// Process id.
    pub pid: u32,
    /// Process start time (`FILETIME` as decimal text — it does not fit a JS
    /// number). Sent back unchanged with a close request.
    pub start_time: String,
    /// Name Windows reports.
    pub app_name: String,
    /// Full executable path, when readable.
    pub exe_path: Option<String>,
    /// What kind of program it is.
    pub kind: AppKind,
    /// Service short name, for a service.
    pub service_name: Option<String>,
    /// Whether the UI may offer "Ask to close".
    pub can_close: bool,
    /// Stable code for the reason, for translation.
    pub reason_code: String,
    /// Human-readable reason: what it holds and what can be done.
    pub reason: String,
    /// The first held files (per-file attribution only).
    pub held_files: Vec<String>,
    /// How many of the files it holds (per-file attribution only).
    pub held_count: usize,
    /// `false` when the query covered too many files to say which ones.
    pub attributed: bool,
}

/// What `unlocker_query` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnlockerReport {
    /// Programs holding the files, by name.
    pub rows: Vec<LockRow>,
    /// Files asked about.
    pub files_checked: usize,
    /// Whether a folder had more files than the cap.
    pub capped: bool,
    /// The cap, for the "checked the first 5 000 files" note.
    pub cap: usize,
    /// Picked paths that were not checked.
    pub skipped: Vec<SkippedPath>,
    /// Entries inside folders that could not be read.
    pub unreadable: usize,
}

/// Who holds `paths`. Read-only.
///
/// # Errors
/// Returns an error when the Restart Manager fails.
pub fn query(rm: &dyn RestartManager, paths: &[PathBuf]) -> Result<UnlockerReport, RmError> {
    let targets = expand_targets(paths, FOLDER_FILE_CAP);
    let attributed = targets.files.len() <= PER_FILE_ATTRIBUTION_LIMIT;

    let mut holders: Vec<(LockInfo, Vec<PathBuf>)> = Vec::new();
    if attributed {
        for file in &targets.files {
            for info in rm.locking_processes(std::slice::from_ref(file))? {
                add_holder(&mut holders, info, Some(file));
            }
        }
    } else {
        for info in rm.locking_processes(&targets.files)? {
            add_holder(&mut holders, info, None);
        }
    }

    let mut rows: Vec<LockRow> = holders
        .into_iter()
        .map(|(info, files)| row(rm, info, &files, attributed))
        .collect();
    rows.sort_by(|a, b| {
        a.app_name
            .to_lowercase()
            .cmp(&b.app_name.to_lowercase())
            .then(a.pid.cmp(&b.pid))
    });

    Ok(UnlockerReport {
        rows,
        files_checked: targets.files.len(),
        capped: targets.capped,
        cap: FOLDER_FILE_CAP,
        skipped: targets.skipped,
        unreadable: targets.unreadable,
    })
}

fn add_holder(holders: &mut Vec<(LockInfo, Vec<PathBuf>)>, info: LockInfo, file: Option<&PathBuf>) {
    let index = match holders.iter().position(|(known, _)| known.process == info.process) {
        Some(index) => index,
        None => {
            holders.push((info, Vec::new()));
            holders.len() - 1
        }
    };
    if let Some(file) = file {
        holders[index].1.push(file.clone());
    }
}

fn row(rm: &dyn RestartManager, info: LockInfo, files: &[PathBuf], attributed: bool) -> LockRow {
    let policy = close_policy(info.process.pid, info.kind);
    let holds = if attributed {
        format!("Has {} of these files open.", files.len())
    } else {
        "Holds one or more of these files.".to_owned()
    };
    LockRow {
        pid: info.process.pid,
        start_time: info.process.start_time.to_string(),
        exe_path: rm.exe_path(info.process.pid).map(|p| p.display().to_string()),
        app_name: info.app_name,
        kind: info.kind,
        service_name: info.service_name,
        can_close: policy.is_closable(),
        reason_code: policy.code().to_owned(),
        reason: format!("{holds} {}", policy.reason()),
        held_files: files
            .iter()
            .take(HELD_FILES_SHOWN)
            .map(|p| p.display().to_string())
            .collect(),
        held_count: files.len(),
        attributed,
    }
}

/// What `unlocker_request_close` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloseReport {
    /// What happened.
    pub outcome: CloseOutcome,
    /// The executable that was (or would have been) asked.
    pub exe_path: Option<String>,
}

/// Ask one program to close, politely, and log the request and its outcome.
///
/// # Errors
/// Returns an error when the Restart Manager fails, or when this build is not
/// allowed to reach the OS.
pub fn request_close(rm: &dyn RestartManager, process: UniqueProcess) -> Result<CloseReport, RmError> {
    let exe_path = rm.exe_path(process.pid).map(|p| p.display().to_string());
    let exe = exe_path.as_deref().unwrap_or("<unknown>");
    tracing::info!(pid = process.pid, exe, "file unlocker: asking a program to close");
    let result = rm.request_close(process);
    match &result {
        Ok(outcome) => {
            tracing::info!(pid = process.pid, exe, ?outcome, "file unlocker: close request finished");
        }
        Err(err) => {
            tracing::warn!(pid = process.pid, exe, %err, "file unlocker: close request failed");
        }
    }
    Ok(CloseReport {
        outcome: result?,
        exe_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::restart_manager::fake::FakeRmApi;
    use crate::core::platform::restart_manager::RmSessions;

    fn holder(pid: u32, kind: AppKind) -> LockInfo {
        LockInfo {
            process: UniqueProcess {
                pid,
                start_time: 7_000_000_000_000_000_000 + u64::from(pid),
            },
            app_name: format!("App {pid}"),
            service_name: None,
            kind,
        }
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"x").expect("write");
        path
    }

    #[test]
    fn the_manifest_is_a_non_scan_module() {
        let manifest = FileUnlockerModule.manifest();
        assert_eq!(manifest.id, "file-unlocker");
        assert!(!manifest.capabilities.has_scan);
        assert!(!manifest.capabilities.has_execute);
    }

    #[test]
    fn a_folder_is_capped_and_says_so() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..7 {
            touch(dir.path(), &format!("f{i}.txt"));
        }
        let targets = expand_targets(&[dir.path().to_path_buf()], 5);
        assert_eq!(targets.files.len(), 5);
        assert!(targets.capped);

        let exact = expand_targets(&[dir.path().to_path_buf()], 7);
        assert_eq!(exact.files.len(), 7);
        assert!(!exact.capped, "exactly at the cap is not over it");
    }

    #[test]
    fn the_report_carries_the_cap_note_inputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "a.txt");
        let rm = RmSessions::new(FakeRmApi::default());
        let report = query(&rm, &[dir.path().to_path_buf()]).expect("query");
        assert_eq!(report.cap, FOLDER_FILE_CAP);
        assert!(!report.capped);
        assert_eq!(report.files_checked, 1);
        assert!(report.rows.is_empty());
    }

    #[test]
    fn missing_paths_are_skipped_with_a_reason() {
        let dir = tempfile::tempdir().expect("tempdir");
        let targets = expand_targets(&[dir.path().join("nope")], FOLDER_FILE_CAP);
        assert!(targets.files.is_empty());
        assert_eq!(targets.skipped.len(), 1);
        assert!(!targets.skipped[0].reason.is_empty());
    }

    #[test]
    fn a_picked_file_is_not_counted_twice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = touch(dir.path(), "a.txt");
        let targets = expand_targets(&[a.clone(), dir.path().to_path_buf()], FOLDER_FILE_CAP);
        assert_eq!(targets.files, vec![a]);
    }

    #[test]
    fn small_queries_name_the_files_each_program_holds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = touch(dir.path(), "a.txt");
        let b = touch(dir.path(), "b.txt");
        let word = holder(10, AppKind::WindowApp);
        let svc = holder(11, AppKind::Service);
        let rm = RmSessions::new(FakeRmApi::with_holders(vec![
            (word.clone(), vec![a.clone(), b.clone()]),
            (svc.clone(), vec![b.clone()]),
        ]));

        let report = query(&rm, &[a.clone(), b.clone()]).expect("query");
        assert_eq!(report.rows.len(), 2);
        let word_row = report.rows.iter().find(|r| r.pid == 10).expect("word");
        assert!(word_row.attributed);
        assert_eq!(word_row.held_count, 2);
        assert!(word_row.can_close);
        assert_eq!(word_row.start_time, word.process.start_time.to_string());
        let svc_row = report.rows.iter().find(|r| r.pid == 11).expect("service");
        assert!(!svc_row.can_close);
        assert_eq!(svc_row.reason_code, "service");
        assert!(svc_row.reason.contains("stop it from Services"));
        assert_eq!(svc_row.held_files, vec![b.display().to_string()]);
        // One session per file.
        assert_eq!(rm.api().calls().started, 2);
    }

    #[test]
    fn large_queries_say_one_or_more() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files: Vec<PathBuf> = (0..=PER_FILE_ATTRIBUTION_LIMIT)
            .map(|i| touch(dir.path(), &format!("f{i}.txt")))
            .collect();
        let word = holder(10, AppKind::WindowApp);
        let rm = RmSessions::new(FakeRmApi::with_holders(vec![(word, vec![files[3].clone()])]));

        let report = query(&rm, &files).expect("query");
        assert_eq!(rm.api().calls().started, 1, "one session for them all");
        let row = &report.rows[0];
        assert!(!row.attributed);
        assert!(row.held_files.is_empty());
        assert!(row.reason.contains("one or more of these files"));
    }

    #[test]
    fn held_files_are_trimmed_to_the_first_three() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files: Vec<PathBuf> = (0..5).map(|i| touch(dir.path(), &format!("f{i}.txt"))).collect();
        let rm = RmSessions::new(FakeRmApi::with_holders(vec![(
            holder(10, AppKind::Console),
            files.clone(),
        )]));
        let report = query(&rm, &files).expect("query");
        assert_eq!(report.rows[0].held_files.len(), HELD_FILES_SHOWN);
        assert_eq!(report.rows[0].held_count, 5);
    }

    #[test]
    fn every_row_explains_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = touch(dir.path(), "a.txt");
        let kinds = [
            AppKind::WindowApp,
            AppKind::Console,
            AppKind::Service,
            AppKind::Explorer,
            AppKind::Critical,
            AppKind::Unknown,
        ];
        let holders = kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| (holder(100 + i as u32, *kind), vec![a.clone()]))
            .collect();
        let rm = RmSessions::new(FakeRmApi::with_holders(holders));
        let report = query(&rm, &[a]).expect("query");
        assert_eq!(report.rows.len(), kinds.len());
        for row in &report.rows {
            assert!(row.reason.len() > 20, "{row:?}");
            let closable = matches!(row.kind, AppKind::WindowApp | AppKind::Console);
            assert_eq!(row.can_close, closable, "{row:?}");
        }
    }

    #[test]
    fn a_close_request_reports_the_outcome() {
        let word = holder(10, AppKind::WindowApp);
        let mut api = FakeRmApi::with_holders(vec![(word.clone(), vec![PathBuf::from("C:\\a")])]);
        api.shutdown_closes = true;
        api.exe_paths.insert(10, PathBuf::from("C:\\Office\\WINWORD.EXE"));
        let rm = RmSessions::new(api);
        let report = request_close(&rm, word.process).expect("close");
        assert_eq!(report.outcome, CloseOutcome::Closed);
        assert_eq!(report.exe_path.as_deref(), Some("C:\\Office\\WINWORD.EXE"));
    }

    #[test]
    fn a_stale_start_time_is_refused() {
        let word = holder(10, AppKind::WindowApp);
        let rm = RmSessions::new(FakeRmApi::with_holders(vec![(word.clone(), Vec::new())]));
        let report = request_close(
            &rm,
            UniqueProcess {
                pid: 10,
                start_time: 1,
            },
        )
        .expect("answered");
        assert!(matches!(report.outcome, CloseOutcome::Refused { .. }));
        assert_eq!(rm.api().calls().shutdowns, 0);
    }
}
