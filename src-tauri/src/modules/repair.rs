//! Repair kit (C6): Windows' own SFC and DISM checks, one click each.
//!
//! Nothing runs until the user clicks a tool, and the page shows the exact
//! command first. The checks (`CheckHealth`, `ScanHealth`,
//! `AnalyzeComponentStore`) only read; `sfc /scannow` and `RestoreHealth`
//! replace damaged system files and go behind a confirm dialog in the UI.
//! WinKnife never elevates itself for this: unelevated, a run is refused with
//! `needs_admin` and the UI offers "Restart as administrator" (K-021).
//!
//! The verdict is read from the tool's English output first, then from the
//! documented DISM exit codes; anything else is "finished, see the log",
//! never a guess.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::core::busy::Operation;
use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::RepairRunner;
use crate::core::scanner::CancellationToken;

/// DISM: the operation succeeded and Windows must restart to finish it.
const DISM_EXIT_RESTART: i32 = 3010;
/// DISM: `CBS_E_SOURCE_MISSING` (0x800F081F), no repair source was found.
const DISM_EXIT_SOURCE_MISSING: i32 = 0x800F_081F_u32 as i32;

/// One of the five tools the page offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepairTool {
    /// `sfc /scannow`: checks and replaces protected system files.
    Sfc,
    /// `DISM ... /CheckHealth`: reads the corruption flag, seconds.
    CheckHealth,
    /// `DISM ... /ScanHealth`: scans the component store, read-only.
    ScanHealth,
    /// `DISM ... /RestoreHealth`: repairs the component store.
    RestoreHealth,
    /// `DISM ... /AnalyzeComponentStore`: reports size and cleanup advice.
    AnalyzeComponentStore,
}

impl RepairTool {
    /// Every tool, in page order.
    pub const ALL: [Self; 5] = [
        Self::CheckHealth,
        Self::ScanHealth,
        Self::RestoreHealth,
        Self::Sfc,
        Self::AnalyzeComponentStore,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Sfc => "sfc",
            Self::CheckHealth => "check-health",
            Self::ScanHealth => "scan-health",
            Self::RestoreHealth => "restore-health",
            Self::AnalyzeComponentStore => "analyze-component-store",
        }
    }

    fn exe(self) -> &'static str {
        match self {
            Self::Sfc => "sfc.exe",
            _ => "dism.exe",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Sfc => &["/scannow"],
            Self::CheckHealth => &["/Online", "/Cleanup-Image", "/CheckHealth"],
            Self::ScanHealth => &["/Online", "/Cleanup-Image", "/ScanHealth"],
            Self::RestoreHealth => &["/Online", "/Cleanup-Image", "/RestoreHealth"],
            Self::AnalyzeComponentStore => &["/Online", "/Cleanup-Image", "/AnalyzeComponentStore"],
        }
    }

    /// The command line exactly as run, for the page and the log.
    pub fn command_line(self) -> String {
        let name = if self == Self::Sfc { "sfc" } else { "DISM" };
        format!("{name} {}", self.args().join(" "))
    }

    /// Whether the tool changes system files and needs a confirm first.
    pub fn needs_confirm(self) -> bool {
        matches!(self, Self::Sfc | Self::RestoreHealth)
    }
}

/// One tool as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairToolInfo {
    /// Which tool.
    pub tool: RepairTool,
    /// The exact command line.
    pub command: String,
    /// Whether the UI must confirm before running it.
    pub needs_confirm: bool,
}

/// What a finished run found, in one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// No integrity violations / no component store corruption.
    NoProblems,
    /// Corruption was found and repaired, or RestoreHealth completed.
    Repaired,
    /// Some damage could not be repaired.
    CouldNotRepair,
    /// A restart is needed to finish (or before SFC can run).
    RestartNeeded,
    /// The component store is damaged and RestoreHealth can fix it.
    StoreRepairable,
    /// AnalyzeComponentStore recommends a cleanup.
    CleanupRecommended,
    /// AnalyzeComponentStore says no cleanup is needed.
    CleanupNotNeeded,
    /// The tool failed with an error code.
    Failed,
    /// The user cancelled the run.
    Cancelled,
    /// Finished, but the output was not recognised: see the log.
    Unknown,
}

/// Text patterns, most specific first. Lowercase; matched on the lowercased
/// transcript.
const PATTERNS: &[(&str, Verdict)] = &[
    ("unable to fix some of them", Verdict::CouldNotRepair),
    ("source files could not be found", Verdict::CouldNotRepair),
    ("component store cannot be repaired", Verdict::CouldNotRepair),
    ("system repair pending which requires reboot", Verdict::RestartNeeded),
    ("restart windows to complete this operation", Verdict::RestartNeeded),
    ("successfully repaired them", Verdict::Repaired),
    ("component store corruption was repaired", Verdict::Repaired),
    ("the restore operation completed successfully", Verdict::Repaired),
    ("the component store is repairable", Verdict::StoreRepairable),
    ("did not find any integrity violations", Verdict::NoProblems),
    ("no component store corruption detected", Verdict::NoProblems),
    ("cleanup recommended : yes", Verdict::CleanupRecommended),
    ("cleanup recommended : no", Verdict::CleanupNotNeeded),
    ("could not perform the requested operation", Verdict::Failed),
    ("could not start the repair service", Verdict::Failed),
];

/// Read the verdict from the transcript, then from the exit code.
///
/// Non-English Windows prints localised text, so only the exit code is left
/// there: DISM documents 3010 and 0x800F081F; SFC documents none, so a run
/// whose text is not recognised is `Unknown` rather than a guess.
pub fn verdict(tool: RepairTool, transcript: &str, exit_code: Option<i32>) -> Verdict {
    let Some(code) = exit_code else {
        return Verdict::Cancelled;
    };
    let text = transcript.to_lowercase();
    if let Some((_, found)) = PATTERNS.iter().find(|(pattern, _)| text.contains(pattern)) {
        return *found;
    }
    if tool == RepairTool::Sfc {
        return Verdict::Unknown;
    }
    match code {
        0 => Verdict::Unknown,
        DISM_EXIT_RESTART => Verdict::RestartNeeded,
        DISM_EXIT_SOURCE_MISSING => Verdict::CouldNotRepair,
        _ => Verdict::Failed,
    }
}

/// One line of output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairLine {
    /// Which tool printed it.
    pub tool: RepairTool,
    /// The text, without line ending.
    pub text: String,
    /// A progress line (ended by a bare CR): the next line replaces it.
    pub progress: bool,
}

/// Splits streamed text into lines. A bare CR ends a progress line that the
/// next one overwrites (DISM's bar, SFC's percentage); CR LF and SFC's
/// CR CR LF end an ordinary line.
#[derive(Debug, Default)]
pub struct LineSplitter {
    current: String,
    pending_cr: bool,
}

impl LineSplitter {
    /// Feed text; returns the lines it completed, as `(text, progress)`.
    pub fn push(&mut self, text: &str) -> Vec<(String, bool)> {
        let mut lines = Vec::new();
        for ch in text.chars() {
            match ch {
                '\r' => self.pending_cr = true,
                '\n' => {
                    self.pending_cr = false;
                    lines.push((std::mem::take(&mut self.current), false));
                }
                _ => {
                    if std::mem::take(&mut self.pending_cr) && !self.current.is_empty() {
                        lines.push((std::mem::take(&mut self.current), true));
                    }
                    self.current.push(ch);
                }
            }
        }
        lines
    }

    /// The unterminated last line, if any.
    pub fn finish(&mut self) -> Option<String> {
        self.pending_cr = false;
        Some(std::mem::take(&mut self.current)).filter(|line| !line.is_empty())
    }
}

/// The run's output as the user would see it: progress lines replaced.
#[derive(Debug, Default)]
struct Transcript {
    lines: Vec<String>,
    last_is_progress: bool,
}

impl Transcript {
    fn push(&mut self, text: &str, progress: bool) {
        if self.last_is_progress {
            self.lines.pop();
        }
        self.lines.push(text.to_owned());
        self.last_is_progress = progress;
    }

    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// What a finished run hands the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairOutcome {
    /// Which tool ran.
    pub tool: RepairTool,
    /// The command line run.
    pub command: String,
    /// The one-line verdict.
    pub verdict: Verdict,
    /// The tool's exit code; `None` when cancelled.
    pub exit_code: Option<i32>,
    /// The saved full log, `None` if it could not be written.
    pub log_path: Option<PathBuf>,
}

/// Runs one tool at a time and saves each run's log.
pub struct RepairService {
    runner: Arc<dyn RepairRunner>,
    log_dir: PathBuf,
    active: Mutex<Option<CancellationToken>>,
}

/// Clears the running slot when a run ends, error or panic included.
struct ActiveRun<'a>(&'a Mutex<Option<CancellationToken>>);

impl Drop for ActiveRun<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

impl RepairService {
    /// A service writing its logs under `log_dir`.
    pub fn new(runner: Arc<dyn RepairRunner>, log_dir: PathBuf) -> Self {
        Self {
            runner,
            log_dir,
            active: Mutex::new(None),
        }
    }

    /// The tools, with their command lines.
    pub fn tools() -> Vec<RepairToolInfo> {
        RepairTool::ALL
            .iter()
            .map(|&tool| RepairToolInfo {
                tool,
                command: tool.command_line(),
                needs_confirm: tool.needs_confirm(),
            })
            .collect()
    }

    /// Run `tool`, handing each output line to `on_line`.
    ///
    /// # Errors
    /// [`CoreError::NeedsAdmin`] when unelevated, [`CoreError::Busy`] while
    /// another run is going, or the runner's error when the tool cannot start.
    pub fn run(
        &self,
        tool: RepairTool,
        on_line: &mut dyn FnMut(RepairLine),
    ) -> Result<RepairOutcome> {
        if !self.runner.is_elevated() {
            return Err(CoreError::NeedsAdmin(format!(
                "`{}` only runs as administrator",
                tool.command_line()
            )));
        }
        let cancel = CancellationToken::new();
        let _active = self.begin(&cancel)?;
        let started = Local::now();
        let mut splitter = LineSplitter::default();
        let mut transcript = Transcript::default();
        let mut deliver = |text: String, progress: bool| {
            transcript.push(&text, progress);
            on_line(RepairLine {
                tool,
                text,
                progress,
            });
        };
        let result = self.runner.run(tool.exe(), tool.args(), &cancel, &mut |text| {
            for (line, progress) in splitter.push(text) {
                deliver(line, progress);
            }
        });
        if let Some(line) = splitter.finish() {
            deliver(line, false);
        }
        let exit_code = result?.filter(|_| !cancel.is_cancelled());
        let verdict = verdict(tool, &transcript.text(), exit_code);
        let log_path = self
            .write_log(tool, started, &transcript, exit_code, verdict)
            .map_err(|err| tracing::warn!(error = %err, "repair log not written"))
            .ok();
        tracing::info!(tool = tool.id(), ?verdict, ?exit_code, "repair run finished");
        Ok(RepairOutcome {
            tool,
            command: tool.command_line(),
            verdict,
            exit_code,
            log_path,
        })
    }

    /// Ask the running tool to stop. Returns whether one was running.
    pub fn cancel(&self) -> bool {
        let active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        active.as_ref().map(CancellationToken::cancel).is_some()
    }

    fn begin(&self, cancel: &CancellationToken) -> Result<ActiveRun<'_>> {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if active.is_some() {
            return Err(CoreError::Busy(Operation::Repair));
        }
        *active = Some(cancel.clone());
        Ok(ActiveRun(&self.active))
    }

    fn write_log(
        &self,
        tool: RepairTool,
        started: DateTime<Local>,
        transcript: &Transcript,
        exit_code: Option<i32>,
        verdict: Verdict,
    ) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.log_dir)?;
        let path = self.log_dir.join(format!(
            "repair-{}-{}.log",
            tool.id(),
            started.format("%Y%m%d-%H%M%S")
        ));
        let mut body = String::new();
        let exit = exit_code.map_or_else(|| "none (cancelled)".to_owned(), |c| format!("{c} (0x{c:08X})"));
        let _ = writeln!(body, "WinKnife repair kit");
        let _ = writeln!(body, "Command:  {}", tool.command_line());
        let _ = writeln!(body, "Started:  {}", started.to_rfc3339());
        let _ = writeln!(body, "Finished: {}", Local::now().to_rfc3339());
        let _ = writeln!(body, "Exit:     {exit}");
        let _ = writeln!(body, "Verdict:  {verdict:?}");
        let _ = writeln!(body, "----------------------------------------");
        body.push_str(&transcript.text());
        body.push('\n');
        std::fs::write(&path, body)?;
        Ok(path)
    }
}

/// Registry entry. No scan, no findings: a page of buttons.
#[derive(Debug, Default)]
pub struct RepairModule;

impl ToolModule for RepairModule {
    fn id(&self) -> &'static str {
        "repair"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Repair Kit".to_owned(),
            description: "Runs Windows' own SFC and DISM checks when you ask, and says what they found."
                .to_owned(),
            icon: "wrench".to_owned(),
            category: ModuleCategory::System,
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::FakeRepairRunner;

    const SFC_CLEAN: &str = "Beginning system scan.  This process will take some time.\r\r\n\r\r\nBeginning verification phase of system scan.\r\r\nVerification 100% complete.\r\r\n\r\r\nWindows Resource Protection did not find any integrity violations.\r\r\n";
    const SFC_REPAIRED: &str = "Windows Resource Protection found corrupt files and successfully repaired them.\r\nFor online repairs, details are included in the CBS log file";
    const SFC_UNFIXED: &str = "Windows Resource Protection found corrupt files but was unable to fix some of them.";
    const SFC_PENDING: &str = "There is a system repair pending which requires reboot to complete.  Restart Windows and run sfc again.";
    const DISM_CHECK_CLEAN: &str = "Deployment Image Servicing and Management tool\r\nVersion: 10.0.19041.3636\r\n\r\nImage Version: 10.0.19045.4529\r\n\r\nNo component store corruption detected.\r\nThe operation completed successfully.\r\n";
    const DISM_SCAN_REPAIRABLE: &str = "[==========================100.0%==========================] The component store is repairable.\r\nThe operation completed successfully.";
    const DISM_RESTORE_OK: &str = "\r\n[==                         4.0%                           ]\r[===========================100.0%==========================] The restore operation completed successfully.\r\nThe operation completed successfully.\r\n";
    const DISM_SOURCE_MISSING: &str = "Error: 0x800f081f\r\n\r\nThe source files could not be found.";
    const DISM_ANALYZE_YES: &str = "Number of Reclaimable Packages : 2\r\nComponent Store Cleanup Recommended : Yes\r\n";
    const DISM_ANALYZE_NO: &str = "Component Store Cleanup Recommended : No\r\n";

    #[test]
    fn sfc_outputs_map_to_their_verdicts() {
        let sfc = RepairTool::Sfc;
        assert_eq!(verdict(sfc, SFC_CLEAN, Some(0)), Verdict::NoProblems);
        assert_eq!(verdict(sfc, SFC_REPAIRED, Some(0)), Verdict::Repaired);
        assert_eq!(verdict(sfc, SFC_UNFIXED, Some(0)), Verdict::CouldNotRepair);
        assert_eq!(verdict(sfc, SFC_PENDING, Some(0)), Verdict::RestartNeeded);
    }

    #[test]
    fn dism_outputs_map_to_their_verdicts() {
        use RepairTool::*;
        assert_eq!(verdict(CheckHealth, DISM_CHECK_CLEAN, Some(0)), Verdict::NoProblems);
        assert_eq!(verdict(ScanHealth, DISM_SCAN_REPAIRABLE, Some(0)), Verdict::StoreRepairable);
        assert_eq!(verdict(RestoreHealth, DISM_RESTORE_OK, Some(0)), Verdict::Repaired);
        assert_eq!(
            verdict(RestoreHealth, DISM_SOURCE_MISSING, Some(DISM_EXIT_SOURCE_MISSING)),
            Verdict::CouldNotRepair
        );
        assert_eq!(verdict(AnalyzeComponentStore, DISM_ANALYZE_YES, Some(0)), Verdict::CleanupRecommended);
        assert_eq!(verdict(AnalyzeComponentStore, DISM_ANALYZE_NO, Some(0)), Verdict::CleanupNotNeeded);
    }

    #[test]
    fn localised_output_falls_back_to_dism_exit_codes_and_never_guesses() {
        let turkish = "Bileşen deposunda bozulma algılanmadı.";
        use RepairTool::*;
        assert_eq!(verdict(CheckHealth, turkish, Some(0)), Verdict::Unknown);
        assert_eq!(verdict(RestoreHealth, turkish, Some(DISM_EXIT_RESTART)), Verdict::RestartNeeded);
        assert_eq!(verdict(RestoreHealth, turkish, Some(DISM_EXIT_SOURCE_MISSING)), Verdict::CouldNotRepair);
        assert_eq!(verdict(ScanHealth, turkish, Some(87)), Verdict::Failed);
        assert_eq!(verdict(Sfc, turkish, Some(1)), Verdict::Unknown);
        assert_eq!(verdict(Sfc, SFC_CLEAN, None), Verdict::Cancelled);
    }

    #[test]
    fn the_splitter_marks_cr_progress_and_collapses_sfc_double_cr() {
        let mut splitter = LineSplitter::default();
        let mut lines = splitter.push("\r\n[==  4.0%  ]\r[== 50");
        lines.extend(splitter.push(".0% ]\r[100.0%] done.\r"));
        lines.extend(splitter.push("\r\nok\r\r\n\r\r\ntail"));
        let tail = splitter.finish();
        assert_eq!(
            lines,
            vec![
                (String::new(), false),
                ("[==  4.0%  ]".to_owned(), true),
                ("[== 50.0% ]".to_owned(), true),
                ("[100.0%] done.".to_owned(), false),
                ("ok".to_owned(), false),
                (String::new(), false),
            ]
        );
        assert_eq!(tail.as_deref(), Some("tail"));
    }

    #[test]
    fn the_transcript_keeps_only_the_last_progress_line() {
        let mut transcript = Transcript::default();
        let mut splitter = LineSplitter::default();
        for (line, progress) in splitter.push(DISM_RESTORE_OK) {
            transcript.push(&line, progress);
        }
        assert_eq!(
            transcript.text(),
            "\n[===========================100.0%==========================] The restore operation completed successfully.\nThe operation completed successfully."
        );
    }

    fn service(elevated: bool, dir: &std::path::Path) -> RepairService {
        RepairService::new(Arc::new(FakeRepairRunner { elevated }), dir.to_path_buf())
    }

    #[test]
    fn an_unelevated_run_is_refused_with_needs_admin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = service(false, dir.path())
            .run(RepairTool::CheckHealth, &mut |_| {})
            .expect_err("unelevated");
        assert_eq!(err.code(), "needs_admin");
    }

    #[test]
    fn a_run_streams_lines_writes_a_log_and_says_see_log_for_unknown_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lines = Vec::new();
        let outcome = service(true, dir.path())
            .run(RepairTool::ScanHealth, &mut |line| lines.push(line))
            .expect("fake run");
        assert_eq!(outcome.verdict, Verdict::Unknown);
        assert_eq!(outcome.command, "DISM /Online /Cleanup-Image /ScanHealth");
        assert!(lines.iter().any(|l| l.text.contains("was not started")));
        let log = std::fs::read_to_string(outcome.log_path.expect("log")).expect("read log");
        assert!(log.contains("Command:  DISM /Online /Cleanup-Image /ScanHealth"));
        assert!(log.contains("was not started"));
    }

    #[test]
    fn a_second_run_is_busy_and_cancel_reaches_the_running_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let svc = service(true, dir.path());
        assert!(!svc.cancel(), "nothing is running");
        let token = CancellationToken::new();
        let guard = svc.begin(&token).expect("first");
        assert_eq!(svc.begin(&CancellationToken::new()).err().map(|e| e.code()), Some("busy"));
        assert!(svc.cancel());
        assert!(token.is_cancelled());
        drop(guard);
        assert!(svc.begin(&CancellationToken::new()).is_ok(), "slot freed");
    }

    #[test]
    fn only_the_two_repairing_tools_need_a_confirm() {
        let confirmed: Vec<_> = RepairService::tools()
            .into_iter()
            .filter(|t| t.needs_confirm)
            .map(|t| t.command)
            .collect();
        assert_eq!(confirmed, ["DISM /Online /Cleanup-Image /RestoreHealth", "sfc /scannow"]);
    }
}
