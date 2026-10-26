//! Restore point (module spec 09): one button that asks Windows for a restore
//! point, an extra safety net before system changes.
//!
//! The app side lives here: the description rules, the read-only System
//! Protection check, how an elevated helper's exit is turned into what the user
//! sees, and how a command line is told apart from a helper launch. The Windows
//! calls are behind [`RestorePointPlatform`] in `core/platform/restore_point.rs`.
//!
//! Read only, apart from the restore point itself: nothing here changes System
//! Protection or `SystemRestorePointCreationFrequency`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{
    failure_from_exit_code, Hive, HelperRun, Registry, RestorePointPlatform,
    CREATE_RESTORE_POINT_FLAG, EXIT_CREATED, EXIT_MAYBE_SKIPPED, EXIT_PROTECTION_OFF,
    EXIT_SKIPPED,
};
use crate::LaunchArgs;

/// The description used when the user leaves the field empty.
pub const DEFAULT_DESCRIPTION: &str = "WinKnife — before changes";

/// Windows keeps 64 characters of a restore point's description (`MAX_DESC`).
pub const MAX_DESCRIPTION_CHARS: usize = 64;

/// Where a policy can switch System Restore off for the whole machine.
const POLICY_KEY: &str = r"SOFTWARE\Policies\Microsoft\Windows NT\SystemRestore";
const POLICY_DISABLE_VALUE: &str = "DisableSR";
/// System Restore's own settings; `RPSessionInterval` is 0 when protection is
/// off on every drive and 1 when it is on.
const SETTINGS_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore";
const SETTINGS_INTERVAL_VALUE: &str = "RPSessionInterval";
/// `REG_DWORD`.
const REG_TYPE_DWORD: u32 = 4;

/// Make a user's description safe to pass on the helper's command line and to
/// hand to Windows: printable characters only, no `"` or `\` (so quoting the
/// argument is the whole escaping job), trimmed, at most
/// [`MAX_DESCRIPTION_CHARS`] characters. Empty becomes [`DEFAULT_DESCRIPTION`].
pub fn sanitize_description(raw: &str) -> String {
    let printable: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .collect();
    let truncated: String = printable.trim().chars().take(MAX_DESCRIPTION_CHARS).collect();
    let cleaned = truncated.trim_end();
    if cleaned.is_empty() {
        DEFAULT_DESCRIPTION.to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// Whether System Protection looks enabled. A best guess from the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Protection {
    /// Turned on for at least one drive (normally the system drive).
    On,
    /// Turned off, by the user or by a policy.
    Off,
    /// The registry does not say. A valid answer, not an error.
    Unknown,
}

/// What the page shows above the button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePointStatus {
    /// The best guess.
    pub protection: Protection,
    /// Why WinKnife thinks so, in one sentence.
    pub reason: String,
    /// What an empty description becomes.
    pub default_description: String,
    /// The longest description Windows keeps.
    pub max_description_chars: usize,
}

fn read_dword(registry: &dyn Registry, subkey: &str, name: &str) -> Option<u32> {
    let value = registry.value(Hive::LocalMachine, subkey, name).ok()??;
    if value.kind != REG_TYPE_DWORD {
        return None;
    }
    let bytes: [u8; 4] = value.bytes.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// Read-only check of System Protection. Never errors: an unreadable registry
/// is [`Protection::Unknown`].
pub fn protection_status(registry: &dyn Registry) -> RestorePointStatus {
    let (protection, reason) = if read_dword(registry, POLICY_KEY, POLICY_DISABLE_VALUE) == Some(1)
    {
        (
            Protection::Off,
            "A policy on this computer turns System Restore off.",
        )
    } else {
        match read_dword(registry, SETTINGS_KEY, SETTINGS_INTERVAL_VALUE) {
            Some(0) => (
                Protection::Off,
                "Windows reports System Protection as turned off for every drive.",
            ),
            Some(_) => (
                Protection::On,
                "Windows reports System Protection as turned on.",
            ),
            None => (
                Protection::Unknown,
                "Windows does not say whether System Protection is on; creating a point will tell.",
            ),
        }
    };
    RestorePointStatus {
        protection,
        reason: reason.to_owned(),
        default_description: DEFAULT_DESCRIPTION.to_owned(),
        max_description_chars: MAX_DESCRIPTION_CHARS,
    }
}

/// What happened when the user pressed Create. Every state has a sentence on
/// the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CreateOutcome {
    /// A new restore point exists.
    Created,
    /// Windows made no new point: it allows one per 24 hours by default.
    Skipped,
    /// Windows accepted the request, but WinKnife could not check whether it
    /// made a new point (most likely skipped under the 24-hour limit).
    MaybeSkipped,
    /// System Protection is off; the page offers its settings window.
    ProtectionOff,
    /// The user said No at the UAC prompt.
    Declined,
    /// The helper did not finish in five minutes. It was left running.
    TimedOut,
    /// Anything else, with the Win32 error code or the helper's exit code.
    Failed {
        /// The error the page shows.
        code: u32,
    },
}

/// The exit-code → result table. Pure; the mapping the spec asks to test.
pub fn outcome_from_run(run: HelperRun) -> CreateOutcome {
    match run {
        HelperRun::Declined => CreateOutcome::Declined,
        HelperRun::TimedOut => CreateOutcome::TimedOut,
        HelperRun::LaunchFailed(code) => CreateOutcome::Failed { code },
        HelperRun::Exited(EXIT_CREATED) => CreateOutcome::Created,
        HelperRun::Exited(EXIT_SKIPPED) => CreateOutcome::Skipped,
        HelperRun::Exited(EXIT_MAYBE_SKIPPED) => CreateOutcome::MaybeSkipped,
        HelperRun::Exited(EXIT_PROTECTION_OFF) => CreateOutcome::ProtectionOff,
        // A panicking or unknown helper is a failure with its raw exit code.
        HelperRun::Exited(code) => CreateOutcome::Failed {
            code: failure_from_exit_code(code).unwrap_or(code),
        },
    }
}

/// What this process was started to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    /// The elevated helper: create a restore point and exit. Never builds the
    /// app, never reaches the single-instance plugin.
    CreateRestorePoint {
        /// Already sanitised.
        description: String,
    },
    /// The normal app.
    App(LaunchArgs),
}

/// Tell a helper launch from an app launch. Pure. The helper flag wins over
/// everything else on the line, `--module` included, so an elevated helper can
/// never turn into a second app forwarding its argv to the first.
pub fn launch_mode(args: impl IntoIterator<Item = String>) -> LaunchMode {
    let args: Vec<String> = args.into_iter().collect();
    // `argv[0]` is the program path, never the flag.
    let flag = args
        .iter()
        .skip(1)
        .position(|arg| arg == CREATE_RESTORE_POINT_FLAG);
    match flag {
        Some(index) => LaunchMode::CreateRestorePoint {
            description: sanitize_description(
                args.get(index + 2).map(String::as_str).unwrap_or_default(),
            ),
        },
        None => LaunchMode::App(crate::parse_launch_args(args)),
    }
}

/// Clears the busy flag when dropped, so a panic cannot wedge the service.
struct BusyReset<'a>(&'a AtomicBool);

impl Drop for BusyReset<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The app side of the module: status, create, open settings.
pub struct RestorePointService {
    platform: Arc<dyn RestorePointPlatform>,
    registry: Arc<dyn Registry>,
    busy: AtomicBool,
}

impl RestorePointService {
    /// Compose the service.
    pub fn new(platform: Arc<dyn RestorePointPlatform>, registry: Arc<dyn Registry>) -> Self {
        Self {
            platform,
            registry,
            busy: AtomicBool::new(false),
        }
    }

    /// Read-only System Protection check.
    pub fn status(&self) -> RestorePointStatus {
        protection_status(self.registry.as_ref())
    }

    /// Ask the elevated helper for a restore point and wait for its answer.
    /// Blocks for as long as the UAC prompt and Windows take (at most five
    /// minutes); call it off the UI thread.
    ///
    /// # Errors
    /// Returns an error while another request is still running (one UAC prompt
    /// at a time), or when this build may not reach the OS. The busy flag is
    /// cleared on every exit, a panic included.
    pub fn create(&self, description: &str) -> Result<CreateOutcome> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(CoreError::RestorePoint(
                "a restore point is already being created".to_owned(),
            ));
        }
        let _reset = BusyReset(&self.busy);
        let description = sanitize_description(description);
        let run = self.platform.run_helper(&description);
        let outcome = outcome_from_run(run?);
        tracing::info!(?outcome, "restore point request finished");
        Ok(outcome)
    }

    /// Open Windows' System Protection settings (button click only).
    ///
    /// # Errors
    /// Returns an error when the window could not be opened.
    pub fn open_protection_settings(&self) -> Result<()> {
        self.platform.open_protection_settings()
    }
}

impl std::fmt::Debug for RestorePointService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestorePointService")
            .field("busy", &self.busy.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

/// Registry entry. No scan, no findings: one button.
#[derive(Debug, Default)]
pub struct RestorePointModule;

impl ToolModule for RestorePointModule {
    fn id(&self) -> &'static str {
        "restore-point"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Restore Point".to_owned(),
            description: "Create a Windows restore point before you change system settings."
                .to_owned(),
            icon: "restore".to_owned(),
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
    use crate::core::platform::{
        failed_exit_code, FakeRestorePointPlatform, MapRegistry, RegValue, WIN32_CANCELLED,
    };

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("winknife.exe")
            .chain(list.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    fn dword(name: &str, value: u32) -> RegValue {
        RegValue {
            name: name.to_owned(),
            kind: REG_TYPE_DWORD,
            bytes: value.to_le_bytes().to_vec(),
        }
    }

    #[test]
    fn an_empty_description_becomes_the_default() {
        assert_eq!(sanitize_description(""), DEFAULT_DESCRIPTION);
        assert_eq!(sanitize_description("   \t\r\n "), DEFAULT_DESCRIPTION);
        assert_eq!(sanitize_description("\"\\\u{7}"), DEFAULT_DESCRIPTION);
    }

    #[test]
    fn a_description_keeps_only_printable_safe_characters() {
        assert_eq!(sanitize_description("  before  driver  "), "before  driver");
        assert_eq!(sanitize_description("a\"b\\c\u{0}d\u{1b}e"), "abcde");
        assert_eq!(sanitize_description("line\nbreak\ttab"), "line break tab");
        assert_eq!(sanitize_description("Sürücü öncesi — ğış"), "Sürücü öncesi — ğış");
    }

    #[test]
    fn a_description_is_cut_at_64_characters_not_bytes() {
        let long = "ş".repeat(100);
        let cleaned = sanitize_description(&long);
        assert_eq!(cleaned.chars().count(), MAX_DESCRIPTION_CHARS);
        assert_eq!(sanitize_description(&"x".repeat(64)), "x".repeat(64));
        // No trailing space left behind by the cut.
        let spaced = format!("{} tail", "y".repeat(63));
        assert_eq!(sanitize_description(&spaced), "y".repeat(63));
    }

    #[test]
    fn the_default_description_fits() {
        assert!(DEFAULT_DESCRIPTION.chars().count() <= MAX_DESCRIPTION_CHARS);
        assert_eq!(sanitize_description(DEFAULT_DESCRIPTION), DEFAULT_DESCRIPTION);
    }

    #[test]
    fn every_exit_code_maps_to_one_result() {
        let table = [
            (HelperRun::Exited(EXIT_CREATED), CreateOutcome::Created),
            (HelperRun::Exited(EXIT_SKIPPED), CreateOutcome::Skipped),
            (HelperRun::Exited(EXIT_MAYBE_SKIPPED), CreateOutcome::MaybeSkipped),
            (HelperRun::Exited(EXIT_PROTECTION_OFF), CreateOutcome::ProtectionOff),
            (HelperRun::Declined, CreateOutcome::Declined),
            (HelperRun::TimedOut, CreateOutcome::TimedOut),
            (
                HelperRun::Exited(failed_exit_code(1058)),
                CreateOutcome::Failed { code: 1058 },
            ),
            (HelperRun::Exited(101), CreateOutcome::Failed { code: 101 }),
            (HelperRun::LaunchFailed(2), CreateOutcome::Failed { code: 2 }),
        ];
        for (run, expected) in table {
            assert_eq!(outcome_from_run(run), expected, "{run:?}");
        }
    }

    #[test]
    fn outcomes_serialise_as_tagged_states() {
        let json = serde_json::to_string(&CreateOutcome::Failed { code: 5 }).expect("json");
        assert_eq!(json, r#"{"state":"failed","code":5}"#);
        let json = serde_json::to_string(&CreateOutcome::MaybeSkipped).expect("json");
        assert_eq!(json, r#"{"state":"maybeSkipped"}"#);
    }

    #[test]
    fn the_helper_flag_is_parsed_and_sanitised() {
        assert_eq!(
            launch_mode(args(&[CREATE_RESTORE_POINT_FLAG, "before \"drivers\""])),
            LaunchMode::CreateRestorePoint {
                description: "before drivers".to_owned()
            }
        );
        assert_eq!(
            launch_mode(args(&[CREATE_RESTORE_POINT_FLAG])),
            LaunchMode::CreateRestorePoint {
                description: DEFAULT_DESCRIPTION.to_owned()
            }
        );
    }

    /// The helper must never reach the single-instance plugin: if it were an
    /// `App` launch it would forward its argv to the running app and exit with
    /// no restore point made.
    #[test]
    fn the_helper_flag_bypasses_the_app_and_single_instance() {
        for line in [
            args(&[CREATE_RESTORE_POINT_FLAG, "x"]),
            args(&["--module", "startup", CREATE_RESTORE_POINT_FLAG, "x"]),
            args(&[CREATE_RESTORE_POINT_FLAG, "x", "--module=startup"]),
            args(&["--dry-run", CREATE_RESTORE_POINT_FLAG, "x"]),
        ] {
            assert!(
                matches!(launch_mode(line.clone()), LaunchMode::CreateRestorePoint { .. }),
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_normal_launch_is_the_app() {
        assert_eq!(launch_mode(args(&[])), LaunchMode::App(LaunchArgs::default()));
        assert_eq!(
            launch_mode(args(&["--module", "restore-point"])),
            LaunchMode::App(LaunchArgs {
                module: Some("restore-point".to_owned())
            })
        );
        // The program path is never mistaken for the flag.
        assert!(matches!(
            launch_mode(vec![CREATE_RESTORE_POINT_FLAG.to_owned()]),
            LaunchMode::App(_)
        ));
    }

    #[test]
    fn protection_reads_policy_then_settings_then_unknown() {
        let unknown = protection_status(&MapRegistry::new());
        assert_eq!(unknown.protection, Protection::Unknown);
        assert!(!unknown.reason.is_empty());

        let on = MapRegistry::new().with_key(
            Hive::LocalMachine,
            SETTINGS_KEY,
            vec![dword(SETTINGS_INTERVAL_VALUE, 1)],
        );
        assert_eq!(protection_status(&on).protection, Protection::On);

        let off = MapRegistry::new().with_key(
            Hive::LocalMachine,
            SETTINGS_KEY,
            vec![dword(SETTINGS_INTERVAL_VALUE, 0)],
        );
        assert_eq!(protection_status(&off).protection, Protection::Off);

        let policy = MapRegistry::new()
            .with_key(
                Hive::LocalMachine,
                SETTINGS_KEY,
                vec![dword(SETTINGS_INTERVAL_VALUE, 1)],
            )
            .with_key(
                Hive::LocalMachine,
                POLICY_KEY,
                vec![dword(POLICY_DISABLE_VALUE, 1)],
            );
        assert_eq!(protection_status(&policy).protection, Protection::Off);

        let wrong_type = MapRegistry::new().with_key(
            Hive::LocalMachine,
            SETTINGS_KEY,
            vec![RegValue::binary(SETTINGS_INTERVAL_VALUE, vec![1])],
        );
        assert_eq!(protection_status(&wrong_type).protection, Protection::Unknown);
    }

    #[test]
    fn the_service_sanitises_before_the_helper_sees_it() {
        let fake = Arc::new(FakeRestorePointPlatform::new(HelperRun::Exited(EXIT_CREATED)));
        let service = RestorePointService::new(fake.clone(), Arc::new(MapRegistry::new()));
        assert_eq!(service.create("  a\"b  ").expect("create"), CreateOutcome::Created);
        assert_eq!(service.create("").expect("create"), CreateOutcome::Created);
        assert_eq!(
            fake.descriptions(),
            vec!["ab".to_owned(), DEFAULT_DESCRIPTION.to_owned()]
        );
    }

    /// A helper that panics, standing in for any bug below `create`.
    struct PanickingPlatform;

    impl RestorePointPlatform for PanickingPlatform {
        fn run_helper(&self, _description: &str) -> Result<HelperRun> {
            panic!("helper blew up");
        }
        fn open_protection_settings(&self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_panic_mid_request_does_not_leave_the_service_busy() {
        let service = RestorePointService::new(Arc::new(PanickingPlatform), Arc::new(MapRegistry::new()));
        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.create("x")));
        assert!(first.is_err(), "the fake must panic");

        let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.create("x")));
        // Panicking again (not "already being created") proves the flag was reset.
        assert!(second.is_err(), "a stuck busy flag would return an error instead");
    }

    #[test]
    fn a_declined_prompt_is_reported_as_declined() {
        let fake = Arc::new(FakeRestorePointPlatform::new(HelperRun::Declined));
        let service = RestorePointService::new(fake, Arc::new(MapRegistry::new()));
        assert_eq!(service.create("x").expect("create"), CreateOutcome::Declined);
        // ShellExecuteExW's cancel code is the one the real side matches.
        assert_eq!(WIN32_CANCELLED, 1223);
    }

    #[test]
    fn settings_open_only_when_asked() {
        let fake = Arc::new(FakeRestorePointPlatform::new(HelperRun::Exited(EXIT_PROTECTION_OFF)));
        let service = RestorePointService::new(fake.clone(), Arc::new(MapRegistry::new()));
        assert_eq!(service.create("x").expect("create"), CreateOutcome::ProtectionOff);
        assert_eq!(fake.settings_opened(), 0, "a result never opens a window");
        service.open_protection_settings().expect("open");
        assert_eq!(fake.settings_opened(), 1);
    }

    #[test]
    fn the_module_has_no_scan() {
        let manifest = RestorePointModule.manifest();
        assert_eq!(manifest.id, "restore-point");
        assert!(!manifest.capabilities.has_scan);
    }
}
