//! Task Scheduler access through COM `ITaskService`, behind a trait.
//!
//! Module spec 05 lists logon-triggered scheduled tasks as the third autostart
//! source and prefers `ITaskService` over parsing `schtasks /query /xml`. The
//! preference is worth stating: `schtasks` output is localised, so a parser
//! written against an English machine silently finds nothing on a German one,
//! and the XML it prints has to be re-parsed to answer questions the COM object
//! answers directly.
//!
//! # Disabling is a flag, never a deletion
//!
//! [`TaskScheduler::set_enabled`] maps onto `IRegisteredTask::put_Enabled`.
//! There is deliberately no delete: module spec 05 says "scheduled tasks
//! `Enabled=false` via API, never delete", and an API this app does not have
//! cannot be called by accident.
//!
//! # Tests never touch the real scheduler
//!
//! [`SystemTaskScheduler`] refuses to change anything unless the process armed
//! real OS effects, which only `lib.rs::run()` can do — the same guard the power
//! controller, the recycle bin and the registry use. Enumeration is read-only
//! and unguarded.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::core::error::{CoreError, Result};

/// A scheduled task that runs at logon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogonTask {
    /// Full path in the task tree, e.g. `\Vendor\Updater`. This is the id.
    pub path: String,
    /// Leaf name, for display.
    pub name: String,
    /// Whether it is currently enabled.
    pub enabled: bool,
    /// Program the task's first `Exec` action runs, when it has one.
    pub command: Option<String>,
    /// Arguments of that action.
    pub arguments: Option<String>,
    /// The task's declared author, which is often the vendor name.
    pub author: Option<String>,
}

/// Enumerate logon-triggered tasks and toggle them.
pub trait TaskScheduler: Send + Sync {
    /// Every task with at least one logon trigger.
    ///
    /// # Errors
    /// Returns an error when the scheduler cannot be reached at all. A single
    /// unreadable task is skipped and logged, not fatal — a machine with one
    /// corrupt task definition must still show the other thirty.
    fn logon_tasks(&self) -> Result<Vec<LogonTask>>;

    /// Set a task's enabled flag. Never deletes.
    ///
    /// # Errors
    /// Returns an error when the task is unknown or the change is refused
    /// (a task under `\Microsoft\Windows` usually needs elevation).
    fn set_enabled(&self, task_path: &str, enabled: bool) -> Result<()>;
}

/// A fake scheduler for tests.
#[derive(Debug, Default)]
pub struct MapTaskScheduler {
    tasks: Mutex<BTreeMap<String, LogonTask>>,
    refuse: Mutex<Option<String>>,
}

impl MapTaskScheduler {
    /// An empty scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a task.
    #[must_use]
    pub fn with_task(self, task: LogonTask) -> Self {
        self.lock().insert(task.path.clone(), task);
        self
    }

    /// Make every change fail, as an unelevated change to a system task does.
    #[must_use]
    pub fn refusing(self, message: impl Into<String>) -> Self {
        *self
            .refuse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.into());
        self
    }

    /// The task as it stands now.
    pub fn get(&self, path: &str) -> Option<LogonTask> {
        self.lock().get(path).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, LogonTask>> {
        self.tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl TaskScheduler for MapTaskScheduler {
    fn logon_tasks(&self) -> Result<Vec<LogonTask>> {
        Ok(self.lock().values().cloned().collect())
    }

    fn set_enabled(&self, task_path: &str, enabled: bool) -> Result<()> {
        if let Some(message) = self
            .refuse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Err(CoreError::TaskScheduler(message));
        }
        match self.lock().get_mut(task_path) {
            Some(task) => {
                task.enabled = enabled;
                Ok(())
            }
            None => Err(CoreError::TaskScheduler(format!(
                "no scheduled task at `{task_path}`"
            ))),
        }
    }
}

/// The scheduler of the machine this is running on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTaskScheduler;

impl TaskScheduler for SystemTaskScheduler {
    fn logon_tasks(&self) -> Result<Vec<LogonTask>> {
        #[cfg(windows)]
        {
            sys::logon_tasks()
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    fn set_enabled(&self, task_path: &str, enabled: bool) -> Result<()> {
        if !crate::core::platform::real_os_effects_armed() {
            return Err(CoreError::TaskSchedulerBlocked);
        }
        #[cfg(windows)]
        {
            sys::set_enabled(task_path, enabled)
        }
        #[cfg(not(windows))]
        {
            let _ = (task_path, enabled);
            Err(CoreError::TaskScheduler(
                "there is no Task Scheduler on this platform".to_owned(),
            ))
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::LogonTask;
    use crate::core::error::{CoreError, Result};
    use windows::core::{Interface, BSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::System::TaskScheduler::{
        IExecAction, IRegisteredTask, ITaskFolder, ITaskService, TaskScheduler, TASK_ENUM_HIDDEN,
        TASK_TRIGGER_LOGON, TASK_TRIGGER_TYPE2,
    };
    use windows::Win32::System::Variant::VARIANT;

    /// Per-thread COM initialisation that undoes itself.
    ///
    /// `spawn_blocking` hands out a fresh thread, so this cannot be done once at
    /// startup. `RPC_E_CHANGED_MODE` means somebody already initialised this
    /// thread with a different model, which is fine to work under — but then the
    /// balancing `CoUninitialize` must not run, or their apartment is torn down.
    struct Com {
        owned: bool,
    }

    impl Com {
        fn init() -> Result<Self> {
            // SAFETY: initialising the calling thread's COM apartment.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                return Ok(Self { owned: true });
            }
            // RPC_E_CHANGED_MODE: already initialised differently; usable.
            if hr.0 == 0x8001_0106_u32 as i32 {
                return Ok(Self { owned: false });
            }
            Err(CoreError::TaskScheduler(format!(
                "COM could not be initialised: 0x{:08X}",
                hr.0 as u32
            )))
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.owned {
                // SAFETY: balancing the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }

    fn service() -> Result<ITaskService> {
        // SAFETY: creating the documented in-process Task Scheduler object.
        let service: ITaskService =
            unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }.map_err(
                |err| CoreError::TaskScheduler(format!("Task Scheduler is unavailable: {err}")),
            )?;
        // SAFETY: connecting to the local scheduler as the current user; the four
        // empty variants mean "this machine, this user, no explicit credentials".
        unsafe {
            service.Connect(
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
            )
        }
        .map_err(|err| {
            CoreError::TaskScheduler(format!("could not connect to the Task Scheduler: {err}"))
        })?;
        Ok(service)
    }

    pub(super) fn logon_tasks() -> Result<Vec<LogonTask>> {
        let _com = Com::init()?;
        let service = service()?;
        // SAFETY: `\` is the root folder and always exists.
        let root = unsafe { service.GetFolder(&BSTR::from("\\")) }.map_err(|err| {
            CoreError::TaskScheduler(format!("could not open the root task folder: {err}"))
        })?;

        let mut out = Vec::new();
        collect(&root, &mut out, 0);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// How deep the task tree is walked.
    ///
    /// `\Microsoft\Windows\<component>` is three levels, and nothing on a real
    /// machine goes much past that; the bound is here so a pathological tree
    /// cannot make an autostart list take minutes.
    const MAX_DEPTH: u32 = 6;

    /// `ITriggerCollection` and friends report their count through an
    /// out-parameter rather than a return value.
    unsafe fn out_i32(read: impl FnOnce(*mut i32) -> windows::core::Result<()>) -> i32 {
        let mut count = 0i32;
        match read(&mut count) {
            Ok(()) => count,
            Err(_) => 0,
        }
    }

    /// Same shape for the `BSTR` properties.
    unsafe fn out_bstr(
        read: impl FnOnce(*mut BSTR) -> windows::core::Result<()>,
    ) -> Option<String> {
        let mut value = BSTR::default();
        read(&mut value).ok()?;
        let text = value.to_string();
        (!text.is_empty()).then_some(text)
    }

    fn collect(folder: &ITaskFolder, out: &mut Vec<LogonTask>, depth: u32) {
        // SAFETY: enumerating an open folder; hidden tasks are included because
        // an autostart entry the user cannot see is exactly the interesting case.
        if let Ok(tasks) = unsafe { folder.GetTasks(TASK_ENUM_HIDDEN.0) } {
            // COM collections are 1-based.
            let count = unsafe { tasks.Count() }.unwrap_or(0);
            for index in 1..=count {
                // SAFETY: `index` is inside the reported count.
                let Ok(task) = (unsafe { tasks.get_Item(&VARIANT::from(index)) }) else {
                    continue;
                };
                if let Some(entry) = read_task(&task) {
                    out.push(entry);
                }
            }
        }

        if depth >= MAX_DEPTH {
            return;
        }
        // SAFETY: enumerating subfolders of an open folder.
        if let Ok(folders) = unsafe { folder.GetFolders(0) } {
            let count = unsafe { folders.Count() }.unwrap_or(0);
            for index in 1..=count {
                if let Ok(child) = unsafe { folders.get_Item(&VARIANT::from(index)) } {
                    collect(&child, out, depth + 1);
                }
            }
        }
    }

    /// Read one task, returning `None` when it has no logon trigger.
    fn read_task(task: &IRegisteredTask) -> Option<LogonTask> {
        // SAFETY: reading properties of a live registered task.
        let definition = unsafe { task.Definition() }.ok()?;
        let triggers = unsafe { definition.Triggers() }.ok()?;
        let trigger_count = unsafe { out_i32(|out| triggers.Count(out)) };

        let mut at_logon = false;
        for index in 1..=trigger_count {
            // SAFETY: `index` is inside the reported count.
            if let Ok(trigger) = unsafe { triggers.get_Item(index) } {
                let mut kind = TASK_TRIGGER_TYPE2::default();
                // SAFETY: `kind` is a live local out-parameter.
                if unsafe { trigger.Type(&mut kind) }.is_ok() && kind == TASK_TRIGGER_LOGON {
                    at_logon = true;
                    break;
                }
            }
        }
        if !at_logon {
            return None;
        }

        // SAFETY: reading properties of a live registered task.
        let path = unsafe { task.Path() }.ok()?.to_string();
        let name = unsafe { task.Name() }.ok()?.to_string();
        let enabled = unsafe { task.Enabled() }
            .map(|b| b.as_bool())
            .unwrap_or(true);

        let mut command = None;
        let mut arguments = None;
        // SAFETY: reading the action list of a live definition.
        if let Ok(actions) = unsafe { definition.Actions() } {
            let count = unsafe { out_i32(|out| actions.Count(out)) };
            for index in 1..=count {
                let Ok(action) = (unsafe { actions.get_Item(index) }) else {
                    continue;
                };
                // Only `Exec` actions name a program; the others (send an email,
                // show a message) are legacy and have nothing to sign-check.
                if let Ok(exec) = action.cast::<IExecAction>() {
                    // SAFETY: `exec` is a live action; both properties are BSTR
                    // out-parameters.
                    command = unsafe { out_bstr(|out| exec.Path(out)) };
                    arguments = unsafe { out_bstr(|out| exec.Arguments(out)) };
                    break;
                }
            }
        }

        // SAFETY: reading the registration block of a live definition.
        let author = unsafe { definition.RegistrationInfo() }
            .ok()
            .and_then(|info| unsafe { out_bstr(|out| info.Author(out)) });

        Some(LogonTask {
            path,
            name,
            enabled,
            command,
            arguments,
            author,
        })
    }

    pub(super) fn set_enabled(task_path: &str, enabled: bool) -> Result<()> {
        let _com = Com::init()?;
        let service = service()?;

        let (folder_path, _) = task_path.rsplit_once('\\').unwrap_or(("\\", task_path));
        let folder_path = if folder_path.is_empty() {
            "\\"
        } else {
            folder_path
        };
        // SAFETY: opening the folder the task lives in, then the task itself.
        let folder = unsafe { service.GetFolder(&BSTR::from(folder_path)) }.map_err(|err| {
            CoreError::TaskScheduler(format!("could not open `{folder_path}`: {err}"))
        })?;
        let leaf = task_path.rsplit('\\').next().unwrap_or(task_path);
        let task = unsafe { folder.GetTask(&BSTR::from(leaf)) }.map_err(|err| {
            CoreError::TaskScheduler(format!("no scheduled task at `{task_path}`: {err}"))
        })?;

        // SAFETY: setting a property on a live registered task. This is the only
        // mutation this module performs, and there is no delete anywhere in it.
        unsafe { task.SetEnabled(enabled.into()) }.map_err(|err| {
            CoreError::TaskScheduler(format!(
                "could not change `{task_path}` (a task under \\Microsoft\\Windows \
                 needs an elevated process): {err}"
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(path: &str, enabled: bool) -> LogonTask {
        LogonTask {
            path: path.to_owned(),
            name: path.rsplit('\\').next().unwrap_or(path).to_owned(),
            enabled,
            command: Some("C:\\Vendor\\updater.exe".to_owned()),
            arguments: None,
            author: Some("Vendor Ltd".to_owned()),
        }
    }

    #[test]
    fn the_fake_lists_and_toggles_without_deleting() {
        let scheduler = MapTaskScheduler::new().with_task(task("\\Vendor\\Updater", true));
        assert_eq!(scheduler.logon_tasks().expect("list").len(), 1);

        scheduler
            .set_enabled("\\Vendor\\Updater", false)
            .expect("disable");
        assert!(
            !scheduler
                .get("\\Vendor\\Updater")
                .expect("still there")
                .enabled
        );
        assert_eq!(
            scheduler.logon_tasks().expect("list").len(),
            1,
            "disabling must never remove the task"
        );

        scheduler
            .set_enabled("\\Vendor\\Updater", true)
            .expect("re-enable");
        assert!(
            scheduler
                .get("\\Vendor\\Updater")
                .expect("still there")
                .enabled
        );
    }

    #[test]
    fn an_unknown_task_is_an_error_rather_than_a_silent_no_op() {
        let scheduler = MapTaskScheduler::new();
        let err = scheduler
            .set_enabled("\\Nope", false)
            .expect_err("an unknown task must not report success");
        assert!(err.to_string().contains("\\Nope"), "got {err}");
    }

    #[test]
    fn a_refusing_scheduler_surfaces_the_reason() {
        let scheduler = MapTaskScheduler::new()
            .with_task(task("\\Microsoft\\Windows\\Thing", true))
            .refusing("access is denied");
        let err = scheduler
            .set_enabled("\\Microsoft\\Windows\\Thing", false)
            .expect_err("a refused change must surface");
        assert!(err.to_string().contains("access is denied"), "got {err}");
        assert!(
            scheduler
                .get("\\Microsoft\\Windows\\Thing")
                .expect("task")
                .enabled,
            "a refused change must not be applied locally"
        );
    }

    /// The structural guarantee: a test process cannot change a real task.
    #[test]
    fn the_real_scheduler_refuses_to_change_anything_from_a_test_process() {
        assert!(!crate::core::platform::real_os_effects_armed());
        let err = SystemTaskScheduler
            .set_enabled("\\WinKnife\\NeverExists", false)
            .expect_err("a test build must not be able to change scheduled tasks");
        assert!(matches!(err, CoreError::TaskSchedulerBlocked));
    }

    /// Enumeration is read-only, so it is allowed — and must not panic on a
    /// machine where the scheduler is unreachable.
    #[test]
    fn listing_tasks_never_panics() {
        let listed = SystemTaskScheduler.logon_tasks();
        assert!(
            listed.is_ok() || listed.is_err(),
            "the call must return rather than panic"
        );
    }
}
