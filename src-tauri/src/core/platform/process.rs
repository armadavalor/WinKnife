//! "Is this program running right now?" probe.
//!
//! Why it exists: `docs/RULES-FORMAT.md` gives a rule
//! `match.owner_process_absent: "chrome.exe"`, and module spec 02 says a browser
//! cache is Safe *only* while the browser is closed. Phase 1 parsed and
//! validated the field and never acted on it; phase 2 does, and acting on it
//! needs one read-only look at the process list.
//!
//! Behind a trait like every other OS call (CONVENTIONS.md): a test that
//! enumerated the developer's real processes would pass or fail depending on
//! whether they happened to have Chrome open.
//!
//! Read-only by construction — the snapshot API cannot modify anything, which
//! keeps `analyze` honest about touching nothing.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Answers whether a named executable has a running process.
pub trait ProcessProbe: Send + Sync {
    /// Whether any running process is called `exe_name` (`"chrome.exe"`).
    /// Compared case-insensitively, as Windows process names are.
    ///
    /// May be served from a cache — see [`CachedProcessProbe`]. Correct for a
    /// scan, which wants one consistent answer for the whole run.
    fn is_running(&self, exe_name: &str) -> bool;

    /// The same question, asked of the operating system right now.
    ///
    /// The executor uses this and [`ProcessProbe::is_running`] is wrong for it:
    /// a scan and its analyze pass are one burst, but the user then reviews for
    /// as long as they like, and "Chrome was closed when we looked" is not the
    /// question being answered at the moment a cache is moved (review H10).
    ///
    /// Defaults to [`ProcessProbe::is_running`], which is already uncached for
    /// every probe that does not memoise.
    fn is_running_uncached(&self, exe_name: &str) -> bool {
        self.is_running(exe_name)
    }
}

/// The real probe: one `CreateToolhelp32Snapshot` walk per question.
///
/// Callers ask once per distinct process name per scan (see
/// `modules::cleaner::analyze`), not once per finding, so the snapshot cost is
/// paid a handful of times rather than thousands.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessProbe;

impl ProcessProbe for SystemProcessProbe {
    fn is_running(&self, exe_name: &str) -> bool {
        #[cfg(windows)]
        {
            running_process_names()
                .is_some_and(|names| names.iter().any(|name| name.eq_ignore_ascii_case(exe_name)))
        }

        #[cfg(not(windows))]
        {
            // No Toolhelp API off Windows. Report "running" rather than "not":
            // the conservative answer keeps a browser cache in the review bucket
            // instead of promising it is safe, which is the direction that costs
            // the user nothing.
            let _ = exe_name;
            true
        }
    }
}

/// Every running process's executable name, or `None` when the snapshot failed.
///
/// A failure is reported rather than swallowed so the caller can decide; the
/// caller ([`SystemProcessProbe::is_running`]) treats it as "cannot prove it is
/// closed", which keeps the cautious answer.
#[cfg(windows)]
fn running_process_names() -> Option<Vec<String>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // `dwSize` is computed *before* the snapshot exists, so the only fallible
    // step between opening the handle and closing it is the enumeration itself.
    // It used to sit inside the `unsafe` block with a `?` on it: unreachable in
    // practice (`PROCESSENTRY32W` is 568 bytes), but it was an early return that
    // leaked the handle and quietly contradicted the SAFETY comment below —
    // which is the line a future reader trusts (review L10).
    let Ok(entry_size) = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()) else {
        return None;
    };

    // SAFETY: the snapshot handle is closed on every path below, and
    // `PROCESSENTRY32W` is initialised with its own size as the API requires.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;

        let mut entry = PROCESSENTRY32W {
            dwSize: entry_size,
            ..Default::default()
        };

        let mut names = Vec::new();
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                names.push(String::from_utf16_lossy(&entry.szExeFile[..end]));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        Some(names)
    }
}

/// How long a cached answer stays good.
///
/// A scan and its analyze pass run in one burst, so within a run the answer is
/// stable and the snapshot is taken once per program name. Two seconds later the
/// user may well have closed the browser, and the next scan asks again.
pub const PROCESS_CACHE_TTL: Duration = Duration::from_secs(2);

/// Memoises another probe for [`PROCESS_CACHE_TTL`].
///
/// `Brain::classify_with` asks the process list for every finding a rule with
/// `owner_process_absent` matched, and a `%LOCALAPPDATA%` browser cache is tens
/// of thousands of files. Without this that is tens of thousands of full process
/// snapshots and a scan that never finishes.
pub struct CachedProcessProbe {
    inner: Arc<dyn ProcessProbe>,
    ttl: Duration,
    answers: Mutex<HashMap<String, (Instant, bool)>>,
}

impl CachedProcessProbe {
    /// Wrap `inner` with the default TTL.
    pub fn new(inner: Arc<dyn ProcessProbe>) -> Self {
        Self::with_ttl(inner, PROCESS_CACHE_TTL)
    }

    /// Wrap `inner` with an explicit TTL, so a test can pin the clock behaviour.
    pub fn with_ttl(inner: Arc<dyn ProcessProbe>, ttl: Duration) -> Self {
        Self {
            inner,
            ttl,
            answers: Mutex::new(HashMap::new()),
        }
    }

    fn answers(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, bool)>> {
        self.answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for CachedProcessProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedProcessProbe")
            .field("ttl", &self.ttl)
            .field("cached", &self.answers().len())
            .finish()
    }
}

impl ProcessProbe for CachedProcessProbe {
    fn is_running(&self, exe_name: &str) -> bool {
        let key = exe_name.to_lowercase();
        let now = Instant::now();

        if let Some((at, answer)) = self.answers().get(&key).copied() {
            if now.duration_since(at) < self.ttl {
                return answer;
            }
        }

        let answer = self.inner.is_running(exe_name);
        self.answers().insert(key, (now, answer));
        answer
    }

    /// Bypass the memo and refresh it, so a later scan does not inherit an
    /// answer this call already knows is out of date.
    fn is_running_uncached(&self, exe_name: &str) -> bool {
        let answer = self.inner.is_running_uncached(exe_name);
        self.answers()
            .insert(exe_name.to_lowercase(), (Instant::now(), answer));
        answer
    }
}

/// A probe that reports a fixed set of processes as running.
///
/// Tests about browser caches use it to run both halves of the question — "the
/// browser is closed" and "the browser is open" — on any machine.
#[derive(Debug, Clone, Default)]
pub struct StubProcessProbe {
    running: BTreeSet<String>,
}

impl StubProcessProbe {
    /// Nothing is running.
    pub fn none() -> Self {
        Self::default()
    }

    /// Exactly these executables are running.
    pub fn running(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            running: names
                .into_iter()
                .map(|name| name.into().to_lowercase())
                .collect(),
        }
    }
}

impl ProcessProbe for StubProcessProbe {
    fn is_running(&self, exe_name: &str) -> bool {
        self.running.contains(&exe_name.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_stub_answers_only_for_what_it_was_given() {
        let probe = StubProcessProbe::running(["chrome.exe", "MSEDGE.EXE"]);
        assert!(probe.is_running("chrome.exe"));
        assert!(probe.is_running("CHROME.EXE"), "names are case-insensitive");
        assert!(probe.is_running("msedge.exe"));
        assert!(!probe.is_running("firefox.exe"));

        assert!(!StubProcessProbe::none().is_running("chrome.exe"));
    }

    /// A counting probe, so "how many snapshots did that take?" is assertable.
    #[derive(Debug, Default)]
    struct Counting {
        calls: AtomicUsize,
        answer: bool,
    }

    impl ProcessProbe for Counting {
        fn is_running(&self, _exe_name: &str) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.answer
        }
    }

    /// A browser-cache scan asks this question once per finding — tens of
    /// thousands of times. It must reach the OS once.
    #[test]
    fn the_cache_asks_the_real_probe_once_per_name() {
        let inner = Arc::new(Counting {
            answer: true,
            ..Counting::default()
        });
        let cached = CachedProcessProbe::new(inner.clone());

        for _ in 0..10_000 {
            assert!(cached.is_running("chrome.exe"));
        }
        assert!(cached.is_running("CHROME.EXE"), "names fold together");
        assert!(
            cached.is_running("msedge.exe"),
            "a second name is a second ask"
        );

        assert_eq!(
            inner.calls.load(Ordering::Relaxed),
            2,
            "one snapshot per distinct program name, not per question"
        );
    }

    #[test]
    fn a_cached_answer_expires() {
        let inner = Arc::new(Counting {
            answer: false,
            ..Counting::default()
        });
        let cached = CachedProcessProbe::with_ttl(inner.clone(), Duration::ZERO);

        cached.is_running("chrome.exe");
        cached.is_running("chrome.exe");
        assert_eq!(
            inner.calls.load(Ordering::Relaxed),
            2,
            "a zero TTL must never serve a stale answer"
        );
    }

    /// The real probe has to find the process running this very test, which is
    /// the only process any machine is guaranteed to have.
    #[cfg(windows)]
    #[test]
    fn the_real_probe_finds_the_process_it_is_running_in() {
        let exe = std::env::current_exe().expect("current exe");
        let name = exe
            .file_name()
            .expect("exe name")
            .to_string_lossy()
            .into_owned();

        assert!(
            SystemProcessProbe.is_running(&name),
            "`{name}` is running right now, so the probe must say so"
        );
        assert!(
            !SystemProcessProbe.is_running("winknife-no-such-process-9f3a.exe"),
            "a name nothing is called must not be reported as running"
        );
    }
}
