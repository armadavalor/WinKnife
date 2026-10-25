//! Shutdown timer: schedule shutdown / restart / sleep with a countdown.
//!
//! The first end-to-end module. It touches no files, so it exercises the
//! registry, settings persistence, events and the tray without any risk to the
//! user's data (`docs/modules/01-shutdown-timer.md`).
//!
//! Two design points worth stating, because both are edge cases that bite:
//!
//! - **The target is a wall-clock instant, not a countdown.** Ticks are for
//!   display only. If the machine sleeps for an hour, or the user changes the
//!   system clock, the timer recomputes from the target and does the right
//!   thing instead of drifting.
//! - **We do not pre-arm Windows' own `shutdown /t <big>`.** It cannot show our
//!   countdown and its cancel story is worse. At T-0 we call `shutdown /s /t 0`
//!   through [`PowerController`] — which, in a test or a `--dry-run` build, is
//!   a log line and nothing more.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{DryRunPower, PowerAction, PowerController};

/// Longest timer we accept. Anything longer is almost certainly a typo, and an
/// app that must stay running for days to honour it is not being honest.
pub const MAX_DURATION_HOURS: i64 = 24;

/// Minutes before the target at which a warning notification fires.
pub const WARNING_MINUTES: [i64; 2] = [5, 1];

/// How the user expressed the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ScheduleMode {
    /// "in 2h 30m".
    In {
        /// Whole minutes from now; must be at least 1 and at most 24h.
        minutes: u32,
    },
    /// "at 23:30" — the next occurrence of that local clock time.
    At {
        /// 0-23.
        hour: u32,
        /// 0-59.
        minute: u32,
    },
}

impl ScheduleMode {
    /// Resolve to an absolute UTC instant, using `now` as the reference.
    ///
    /// # Errors
    /// Returns [`CoreError::InvalidTimer`] for a zero-length timer, anything
    /// over [`MAX_DURATION_HOURS`], or an impossible clock time.
    pub fn resolve(self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let target = match self {
            Self::In { minutes } => {
                if minutes == 0 {
                    return Err(CoreError::InvalidTimer(
                        "pick at least one minute — an immediate shutdown is what the Start menu is for".to_owned(),
                    ));
                }
                now + Duration::minutes(i64::from(minutes))
            }
            Self::At { hour, minute } => {
                let clock = NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| {
                    CoreError::InvalidTimer(format!("{hour:02}:{minute:02} is not a valid time"))
                })?;
                next_local_occurrence(now, clock)?
            }
        };

        let span = target - now;
        if span > Duration::hours(MAX_DURATION_HOURS) {
            return Err(CoreError::InvalidTimer(format!(
                "that is more than {MAX_DURATION_HOURS} hours away; pick something sooner"
            )));
        }
        Ok(target)
    }
}

/// The next time the local clock reads `clock`, today or tomorrow.
fn next_local_occurrence(now: DateTime<Utc>, clock: NaiveTime) -> Result<DateTime<Utc>> {
    let local_now = now.with_timezone(&chrono::Local);
    let today = local_now.date_naive();

    for day_offset in 0..=1 {
        let candidate_date = today + Duration::days(day_offset);
        let naive = candidate_date.and_time(clock);
        // A local time can be ambiguous or nonexistent across a DST boundary;
        // take the earliest valid interpretation and move on rather than
        // refusing to set a timer.
        if let Some(local) = chrono::Local.from_local_datetime(&naive).earliest() {
            let utc = local.with_timezone(&Utc);
            if utc > now {
                return Ok(utc);
            }
        }
    }

    Err(CoreError::InvalidTimer(
        "that clock time does not occur in the next 24 hours".to_owned(),
    ))
}

/// A timer as stored in settings, so it survives an app restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedTimer {
    /// Timer id.
    pub id: Uuid,
    /// What happens at T-0.
    pub action: PowerAction,
    /// Absolute target.
    pub target: DateTime<Utc>,
    /// When it was scheduled, shown in the UI as "set at".
    pub scheduled_at: DateTime<Utc>,
    /// How the user expressed it, so an `At` target can be resolved again.
    ///
    /// "At 23:30" is a *wall-clock* request, and storing only the UTC instant
    /// froze it against the offset in force when it was set: a DST change or a
    /// travelled-with laptop between two runs of the app made it fire an hour
    /// out (security review L3). Optional so a timer written by an earlier
    /// version still re-arms, at its stored instant.
    #[serde(default)]
    pub mode: Option<ScheduleMode>,
}

/// The live timer.
#[derive(Debug, Clone)]
struct ActiveTimer {
    persisted: PersistedTimer,
    /// Warning thresholds already fired, so a warning never repeats.
    fired_warnings: Vec<i64>,
}

/// What one tick concluded. The Tauri layer turns this into events,
/// notifications and a tray tooltip; the engine itself knows nothing about UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tick {
    /// No timer is set.
    Idle,
    /// Counting down.
    Counting {
        /// Seconds left, never negative.
        remaining_secs: i64,
        /// A warning threshold crossed on this tick, in minutes.
        warning: Option<i64>,
    },
    /// The timer reached zero and the action was applied.
    Fired {
        /// What was applied.
        action: PowerAction,
    },
    /// The timer reached zero but the action failed. The timer is cleared
    /// either way; leaving it armed would retry forever.
    Failed {
        /// What was attempted.
        action: PowerAction,
        /// Why it did not happen.
        error: String,
    },
}

/// A timer, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerStatus {
    /// The active timer, if any.
    pub active: Option<ActiveTimerView>,
    /// Whether this build only pretends to shut down. The UI shows a banner.
    pub dry_run: bool,
}

/// Public view of an active timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTimerView {
    /// Timer id.
    pub id: Uuid,
    /// What happens at T-0.
    pub action: PowerAction,
    /// Absolute target, so the UI can render a local time.
    pub target: DateTime<Utc>,
    /// When it was set.
    pub scheduled_at: DateTime<Utc>,
    /// Seconds left at the moment the status was taken.
    pub remaining_secs: i64,
}

/// Owns the one timer the app can have.
///
/// The engine is deliberately clock-injectable: every method that cares about
/// time takes `now`. That is what lets the tests assert on a whole countdown,
/// including both warnings and the fire, in microseconds and without ever
/// reaching the OS.
pub struct ShutdownTimer {
    state: Mutex<Option<ActiveTimer>>,
    power: Arc<dyn PowerController>,
}

impl std::fmt::Debug for ShutdownTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownTimer")
            .field("dry_run", &self.power.is_dry_run())
            .field("armed", &self.snapshot().is_some())
            .finish()
    }
}

impl Default for ShutdownTimer {
    fn default() -> Self {
        // The safe default: a timer built by accident cannot shut anything down.
        Self::new(Arc::new(DryRunPower::new()))
    }
}

impl ShutdownTimer {
    /// A timer driven by `power`.
    pub fn new(power: Arc<dyn PowerController>) -> Self {
        Self {
            state: Mutex::new(None),
            power,
        }
    }

    /// Whether the installed controller only pretends.
    pub fn is_dry_run(&self) -> bool {
        self.power.is_dry_run()
    }

    /// Work out what a request means, without arming anything.
    ///
    /// Separate from [`ShutdownTimer::arm`] so the command can persist first
    /// and arm second. Doing it the other way round left a live countdown
    /// behind whenever the write failed, while the UI reported the failure and
    /// showed no timer (security review H4).
    ///
    /// # Errors
    /// See [`ScheduleMode::resolve`].
    pub fn prepare(
        mode: ScheduleMode,
        action: PowerAction,
        now: DateTime<Utc>,
    ) -> Result<PersistedTimer> {
        Ok(PersistedTimer {
            id: Uuid::new_v4(),
            action,
            target: mode.resolve(now)?,
            scheduled_at: now,
            mode: Some(mode),
        })
    }

    /// Arm `persisted`, replacing any existing timer.
    ///
    /// Replacing is deliberate: the UI confirms first, and two competing
    /// timers would be worse than one clear winner (module spec, edge cases).
    pub fn arm(&self, persisted: PersistedTimer) -> Option<PersistedTimer> {
        let (id, action, target) = (persisted.id, persisted.action, persisted.target);
        let replaced = self.replace(Some(ActiveTimer {
            persisted,
            fired_warnings: Vec::new(),
        }));

        tracing::info!(
            timer = %id,
            action = action.slug(),
            target = %target,
            replaced = replaced.is_some(),
            dry_run = self.power.is_dry_run(),
            "timer armed"
        );
        replaced.map(|timer| timer.persisted)
    }

    /// Resolve and arm in one step. The app persists in between, so it uses
    /// [`ShutdownTimer::prepare`] and [`ShutdownTimer::arm`] separately.
    ///
    /// # Errors
    /// See [`ScheduleMode::resolve`].
    pub fn schedule(
        &self,
        mode: ScheduleMode,
        action: PowerAction,
        now: DateTime<Utc>,
    ) -> Result<PersistedTimer> {
        let persisted = Self::prepare(mode, action, now)?;
        self.arm(persisted.clone());
        Ok(persisted)
    }

    /// Re-arm a timer read back from settings, if its target is still ahead.
    ///
    /// Returns whether it was re-armed. A target in the past is discarded: the
    /// machine clearly did not shut down, and firing now would be a nasty
    /// surprise on the next app start.
    ///
    /// An `At` target is re-resolved here rather than trusted. "At 23:30" is a
    /// wall-clock request, and the instant it resolved to was fixed against the
    /// UTC offset in force when it was set; a DST boundary or a timezone change
    /// between two runs moves it by an hour (security review L3). Re-resolving
    /// only ever happens for a timer that is *still live* — a stale one is
    /// discarded first — so this cannot resurrect a timer as tomorrow's.
    /// Re-resolution per tick is deliberately not done: a countdown that jumps
    /// mid-count is worse than one that is right at every restart.
    pub fn rearm(&self, persisted: PersistedTimer, now: DateTime<Utc>) -> bool {
        if persisted.target <= now {
            tracing::info!(
                timer = %persisted.id,
                target = %persisted.target,
                "discarding a persisted timer whose target has passed"
            );
            return false;
        }

        let persisted = match persisted.mode {
            Some(mode @ ScheduleMode::At { .. }) => match mode.resolve(now) {
                Ok(target) if target != persisted.target => {
                    tracing::info!(
                        timer = %persisted.id,
                        was = %persisted.target,
                        now_targets = %target,
                        "re-resolved a wall-clock timer; the local offset has changed"
                    );
                    PersistedTimer { target, ..persisted }
                }
                Ok(_) => persisted,
                // The clock time no longer resolves at all (it fell into a DST
                // gap). The stored instant is still in the future, so honour it.
                Err(err) => {
                    tracing::warn!(
                        timer = %persisted.id,
                        error = %err,
                        "could not re-resolve a wall-clock timer; keeping the stored instant"
                    );
                    persisted
                }
            },
            _ => persisted,
        };

        tracing::info!(timer = %persisted.id, target = %persisted.target, "timer re-armed after restart");
        self.replace(Some(ActiveTimer {
            persisted,
            fired_warnings: Vec::new(),
        }));
        true
    }

    /// Cancel the timer. Returns what was cancelled, or `None` if nothing was
    /// armed — cancelling twice is not an error.
    pub fn cancel(&self) -> Option<PersistedTimer> {
        let previous = self.replace(None).map(|timer| timer.persisted);
        if let Some(timer) = &previous {
            tracing::info!(timer = %timer.id, "timer cancelled");
        }
        previous
    }

    /// The armed timer, if any.
    pub fn snapshot(&self) -> Option<PersistedTimer> {
        self.lock().as_ref().map(|timer| timer.persisted.clone())
    }

    /// Status for the UI at `now`.
    pub fn status(&self, now: DateTime<Utc>) -> TimerStatus {
        TimerStatus {
            active: self.snapshot().map(|timer| ActiveTimerView {
                remaining_secs: remaining_secs(timer.target, now),
                id: timer.id,
                action: timer.action,
                target: timer.target,
                scheduled_at: timer.scheduled_at,
            }),
            dry_run: self.power.is_dry_run(),
        }
    }

    /// Advance the timer to `now`.
    ///
    /// Called once a second by the app, and directly by tests with whatever
    /// instant they want to prove something about.
    pub fn tick(&self, now: DateTime<Utc>) -> Tick {
        let mut guard = self.lock();
        let Some(timer) = guard.as_mut() else {
            return Tick::Idle;
        };

        let remaining = remaining_secs(timer.persisted.target, now);
        if remaining > 0 {
            let warning = WARNING_MINUTES
                .iter()
                .copied()
                .filter(|minutes| {
                    remaining <= minutes * 60 && !timer.fired_warnings.contains(minutes)
                })
                // Several thresholds can be crossed at once if the app was
                // suspended; report the most urgent and mark them all fired.
                .min();

            if let Some(minutes) = warning {
                for threshold in WARNING_MINUTES.iter().copied() {
                    if remaining <= threshold * 60 && !timer.fired_warnings.contains(&threshold) {
                        timer.fired_warnings.push(threshold);
                    }
                }
                tracing::info!(
                    timer = %timer.persisted.id,
                    minutes,
                    "warning threshold crossed"
                );
            }

            return Tick::Counting {
                remaining_secs: remaining,
                warning,
            };
        }

        // T-0. Disarm before acting, so a failing power call cannot loop.
        let action = timer.persisted.action;
        let id = timer.persisted.id;
        *guard = None;
        drop(guard);

        match self.power.apply(action) {
            Ok(()) => {
                tracing::info!(timer = %id, action = action.slug(), "timer fired");
                Tick::Fired { action }
            }
            Err(err) => {
                tracing::error!(timer = %id, action = action.slug(), error = %err, "timer fired but the action failed");
                Tick::Failed {
                    action,
                    error: err.to_string(),
                }
            }
        }
    }

    fn replace(&self, next: Option<ActiveTimer>) -> Option<ActiveTimer> {
        std::mem::replace(&mut self.lock(), next)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<ActiveTimer>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Whole seconds until `target`, never negative.
pub fn remaining_secs(target: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (target - now).num_seconds().max(0)
}

/// `2h 05m 30s`, for the tray tooltip. The UI formats its own countdown.
pub fn format_remaining(total_secs: i64) -> String {
    let total = total_secs.max(0);
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Registry entry. `scan`/`analyze`/`execute` stay at their no-op defaults:
/// this module is about commands, not findings.
#[derive(Debug, Default)]
pub struct ShutdownTimerModule;

impl ToolModule for ShutdownTimerModule {
    fn id(&self) -> &'static str {
        "shutdown-timer"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Shutdown Timer".to_owned(),
            description:
                "Schedule a shutdown, restart or sleep with a countdown you can cancel any time."
                    .to_owned(),
            icon: "power".to_owned(),
            category: ModuleCategory::Power,
            capabilities: ModuleCapabilities {
                has_scan: false,
                has_execute: false,
                has_settings: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> (ShutdownTimer, DryRunPower) {
        let power = DryRunPower::new();
        (ShutdownTimer::new(Arc::new(power.clone())), power)
    }

    fn at(hours: i64, minutes: i64) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH + Duration::hours(hours) + Duration::minutes(minutes)
    }

    #[test]
    fn a_fresh_timer_is_idle_and_dry_run() {
        let (timer, power) = timer();
        assert_eq!(timer.tick(Utc::now()), Tick::Idle);
        assert_eq!(timer.snapshot(), None);
        assert!(timer.is_dry_run());
        assert!(power.is_empty());
    }

    #[test]
    fn scheduling_a_duration_sets_an_absolute_target() {
        let (timer, _) = timer();
        let now = at(10, 0);
        let persisted = timer
            .schedule(ScheduleMode::In { minutes: 90 }, PowerAction::Shutdown, now)
            .expect("schedule");

        assert_eq!(persisted.target, at(11, 30));
        assert_eq!(persisted.action, PowerAction::Shutdown);
        assert_eq!(timer.snapshot(), Some(persisted));
    }

    #[test]
    fn a_zero_minute_timer_is_refused() {
        let (timer, power) = timer();
        let err = timer
            .schedule(ScheduleMode::In { minutes: 0 }, PowerAction::Shutdown, at(10, 0))
            .expect_err("zero minutes");
        assert!(matches!(err, CoreError::InvalidTimer(_)));
        assert!(power.is_empty());
    }

    #[test]
    fn a_timer_longer_than_a_day_is_refused_with_a_friendly_message() {
        let (timer, _) = timer();
        let err = timer
            .schedule(
                ScheduleMode::In { minutes: 24 * 60 + 1 },
                PowerAction::Shutdown,
                at(10, 0),
            )
            .expect_err("over 24h");
        match err {
            CoreError::InvalidTimer(message) => {
                assert!(message.contains("24"), "got `{message}`");
                assert!(message.contains("sooner"), "got `{message}`");
            }
            other => panic!("wrong error: {other}"),
        }
    }

    #[test]
    fn exactly_twenty_four_hours_is_still_allowed() {
        let (timer, _) = timer();
        assert!(timer
            .schedule(
                ScheduleMode::In { minutes: 24 * 60 },
                PowerAction::Shutdown,
                at(10, 0)
            )
            .is_ok());
    }

    #[test]
    fn a_clock_time_resolves_to_the_next_occurrence() {
        let now = Utc::now();
        let local = now.with_timezone(&chrono::Local);
        let in_two_hours = local + Duration::hours(2);

        let target = ScheduleMode::At {
            hour: in_two_hours.format("%H").to_string().parse().expect("hour"),
            minute: in_two_hours
                .format("%M")
                .to_string()
                .parse()
                .expect("minute"),
        }
        .resolve(now)
        .expect("resolve");

        assert!(target > now, "the target must be in the future");
        assert!(
            target - now <= Duration::hours(24),
            "the next occurrence is within a day"
        );
    }

    #[test]
    fn a_clock_time_already_past_today_rolls_to_tomorrow() {
        let now = Utc::now();
        let local = now.with_timezone(&chrono::Local);
        let an_hour_ago = local - Duration::hours(1);

        let target = ScheduleMode::At {
            hour: an_hour_ago.format("%H").to_string().parse().expect("hour"),
            minute: an_hour_ago.format("%M").to_string().parse().expect("minute"),
        }
        .resolve(now)
        .expect("resolve");

        assert!(target > now);
        assert!(target - now > Duration::hours(22), "it rolled to tomorrow");
    }

    #[test]
    fn an_impossible_clock_time_is_refused() {
        let err = ScheduleMode::At {
            hour: 25,
            minute: 0,
        }
        .resolve(Utc::now())
        .expect_err("25:00 does not exist");
        assert!(matches!(err, CoreError::InvalidTimer(_)));
    }

    #[test]
    fn scheduling_twice_replaces_rather_than_stacks() {
        let (timer, _) = timer();
        let now = at(10, 0);
        timer
            .schedule(ScheduleMode::In { minutes: 30 }, PowerAction::Shutdown, now)
            .expect("first");
        let second = timer
            .schedule(ScheduleMode::In { minutes: 60 }, PowerAction::Restart, now)
            .expect("second");

        let armed = timer.snapshot().expect("armed");
        assert_eq!(armed, second);
        assert_eq!(armed.action, PowerAction::Restart);
        assert_eq!(armed.target, at(11, 0));
    }

    #[test]
    fn cancelling_disarms_and_is_idempotent() {
        let (timer, power) = timer();
        timer
            .schedule(ScheduleMode::In { minutes: 5 }, PowerAction::Sleep, at(10, 0))
            .expect("schedule");

        assert!(timer.cancel().is_some());
        assert_eq!(timer.snapshot(), None);
        assert_eq!(timer.cancel(), None, "cancelling twice is not an error");
        assert_eq!(timer.tick(at(20, 0)), Tick::Idle);
        assert!(power.is_empty(), "a cancelled timer must never fire");
    }

    /// The acceptance item from the module spec, minus the part that would
    /// shut this machine down: a full countdown, both warnings, and a fire
    /// that lands in the dry-run log instead of the OS.
    #[test]
    fn a_countdown_fires_both_warnings_and_then_the_action() {
        let (timer, power) = timer();
        let now = at(10, 0);
        timer
            .schedule(ScheduleMode::In { minutes: 10 }, PowerAction::Shutdown, now)
            .expect("schedule");

        // Well clear of any threshold.
        assert_eq!(
            timer.tick(now + Duration::minutes(1)),
            Tick::Counting {
                remaining_secs: 9 * 60,
                warning: None,
            }
        );

        // Crossing T-5.
        assert_eq!(
            timer.tick(now + Duration::minutes(5)),
            Tick::Counting {
                remaining_secs: 5 * 60,
                warning: Some(5),
            }
        );
        // ...and it does not repeat.
        assert_eq!(
            timer.tick(now + Duration::minutes(6)),
            Tick::Counting {
                remaining_secs: 4 * 60,
                warning: None,
            }
        );

        // Crossing T-1.
        assert_eq!(
            timer.tick(now + Duration::minutes(9)),
            Tick::Counting {
                remaining_secs: 60,
                warning: Some(1),
            }
        );

        // T-0.
        assert_eq!(
            timer.tick(now + Duration::minutes(10)),
            Tick::Fired {
                action: PowerAction::Shutdown,
            }
        );

        // The only place the "shutdown" exists is the dry-run log.
        let log: Vec<_> = power.log().into_iter().map(|e| e.action).collect();
        assert_eq!(log, vec![PowerAction::Shutdown]);
        assert_eq!(timer.snapshot(), None, "firing disarms the timer");
        assert_eq!(timer.tick(now + Duration::hours(1)), Tick::Idle);
    }

    #[test]
    fn a_missed_tick_still_reports_the_most_urgent_warning_once() {
        let (timer, _) = timer();
        let now = at(10, 0);
        timer
            .schedule(ScheduleMode::In { minutes: 10 }, PowerAction::Shutdown, now)
            .expect("schedule");

        // The machine was asleep; the next tick is already inside T-1.
        assert_eq!(
            timer.tick(now + Duration::seconds(9 * 60 + 30)),
            Tick::Counting {
                remaining_secs: 30,
                warning: Some(1),
            }
        );
        // Both thresholds are marked fired, so nothing repeats.
        assert_eq!(
            timer.tick(now + Duration::seconds(9 * 60 + 40)),
            Tick::Counting {
                remaining_secs: 20,
                warning: None,
            }
        );
    }

    /// Resume-from-sleep and clock changes: the target is absolute, so a tick
    /// long past it fires exactly once rather than counting the lost time.
    #[test]
    fn a_tick_long_after_the_target_fires_once() {
        let (timer, power) = timer();
        let now = at(10, 0);
        timer
            .schedule(ScheduleMode::In { minutes: 30 }, PowerAction::Restart, now)
            .expect("schedule");

        assert_eq!(
            timer.tick(now + Duration::hours(9)),
            Tick::Fired {
                action: PowerAction::Restart,
            }
        );
        assert_eq!(timer.tick(now + Duration::hours(10)), Tick::Idle);
        assert_eq!(power.log().len(), 1);
    }

    #[test]
    fn a_failing_power_action_is_reported_and_disarms_the_timer() {
        struct Broken;
        impl PowerController for Broken {
            fn apply(&self, _action: PowerAction) -> Result<()> {
                Err(CoreError::PowerAction("no".to_owned()))
            }
        }

        let timer = ShutdownTimer::new(Arc::new(Broken));
        let now = at(10, 0);
        timer
            .schedule(ScheduleMode::In { minutes: 1 }, PowerAction::Sleep, now)
            .expect("schedule");

        match timer.tick(now + Duration::minutes(1)) {
            Tick::Failed { action, error } => {
                assert_eq!(action, PowerAction::Sleep);
                assert!(!error.is_empty());
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(
            timer.snapshot(),
            None,
            "a failed fire must not retry forever"
        );
    }

    #[test]
    fn a_persisted_timer_with_a_future_target_is_rearmed() {
        let (timer, _) = timer();
        let now = at(10, 0);
        let persisted = PersistedTimer {
            id: Uuid::new_v4(),
            action: PowerAction::Shutdown,
            target: at(10, 30),
            scheduled_at: at(9, 0),
            mode: Some(ScheduleMode::In { minutes: 90 }),
        };

        assert!(timer.rearm(persisted.clone(), now));
        assert_eq!(timer.snapshot(), Some(persisted));
    }

    #[test]
    fn a_persisted_timer_whose_target_has_passed_is_discarded() {
        let (timer, power) = timer();
        let persisted = PersistedTimer {
            id: Uuid::new_v4(),
            action: PowerAction::Shutdown,
            target: at(9, 0),
            scheduled_at: at(8, 0),
            mode: None,
        };

        assert!(!timer.rearm(persisted, at(10, 0)));
        assert_eq!(timer.snapshot(), None);
        assert!(
            power.is_empty(),
            "a stale timer must never fire on the next app start"
        );
    }

    #[test]
    fn a_persisted_timer_round_trips_through_json() {
        let persisted = PersistedTimer {
            id: Uuid::nil(),
            action: PowerAction::Sleep,
            target: at(11, 0),
            scheduled_at: at(10, 0),
            mode: Some(ScheduleMode::At {
                hour: 23,
                minute: 30,
            }),
        };
        let json = serde_json::to_string(&persisted).expect("serialize");
        assert!(json.contains(r#""scheduledAt""#), "got {json}");
        assert!(json.contains(r#""mode""#), "got {json}");
        assert_eq!(
            serde_json::from_str::<PersistedTimer>(&json).expect("deserialize"),
            persisted
        );
    }

    /* ---------------------------------------------------------------------
     * Security-review regressions
     * ------------------------------------------------------------------ */

    /// H4: preparing must not arm. The command persists in between, and an
    /// armed-but-unpersisted timer is a live countdown the UI says failed.
    #[test]
    fn preparing_a_timer_does_not_arm_it() {
        let (timer, power) = timer();
        let now = at(10, 0);

        let persisted = ShutdownTimer::prepare(
            ScheduleMode::In { minutes: 30 },
            PowerAction::Shutdown,
            now,
        )
        .expect("prepare");

        assert_eq!(persisted.target, at(10, 30));
        assert_eq!(timer.snapshot(), None, "nothing is armed yet");
        assert_eq!(timer.tick(at(11, 0)), Tick::Idle);
        assert!(power.is_empty(), "an unarmed timer can never fire");

        // ...and arming it afterwards behaves exactly as scheduling did.
        timer.arm(persisted.clone());
        assert_eq!(timer.snapshot(), Some(persisted));
    }

    /// H4: an invalid request is refused before anything is armed, so a
    /// failure can never leave a timer behind.
    #[test]
    fn a_refused_request_arms_nothing() {
        let (timer, power) = timer();
        timer
            .schedule(ScheduleMode::In { minutes: 20 }, PowerAction::Sleep, at(10, 0))
            .expect("a good timer first");

        assert!(ShutdownTimer::prepare(
            ScheduleMode::In { minutes: 0 },
            PowerAction::Shutdown,
            at(10, 0)
        )
        .is_err());

        let armed = timer.snapshot().expect("the good timer survives");
        assert_eq!(armed.action, PowerAction::Sleep);
        assert!(power.is_empty());
    }

    #[test]
    fn arming_reports_what_it_replaced() {
        let (timer, _) = timer();
        let now = at(10, 0);
        let first =
            ShutdownTimer::prepare(ScheduleMode::In { minutes: 10 }, PowerAction::Sleep, now)
                .expect("prepare");
        let second = ShutdownTimer::prepare(
            ScheduleMode::In { minutes: 20 },
            PowerAction::Restart,
            now,
        )
        .expect("prepare");

        assert_eq!(timer.arm(first.clone()), None);
        assert_eq!(timer.arm(second.clone()), Some(first));
        assert_eq!(timer.snapshot(), Some(second));
    }

    /// L3: "at 23:30" is a wall-clock request. Storing only the instant froze
    /// it against the offset in force when it was set.
    #[test]
    fn a_wall_clock_timer_is_re_resolved_when_it_is_rearmed() {
        let (timer, _) = timer();
        let now = Utc::now();
        let local = now.with_timezone(&chrono::Local) + Duration::hours(3);
        let mode = ScheduleMode::At {
            hour: local.format("%H").to_string().parse().expect("hour"),
            minute: local.format("%M").to_string().parse().expect("minute"),
        };
        let correct = mode.resolve(now).expect("resolve");

        // As if a DST change had shifted the stored instant by an hour.
        let stored = PersistedTimer {
            id: Uuid::new_v4(),
            action: PowerAction::Shutdown,
            target: correct + Duration::hours(1),
            scheduled_at: now - Duration::hours(1),
            mode: Some(mode),
        };

        assert!(timer.rearm(stored, now));
        let armed = timer.snapshot().expect("armed");
        assert_eq!(
            armed.target, correct,
            "the wall-clock time the user asked for wins over the stored instant"
        );
    }

    /// L3: a duration is not a wall-clock request, so its instant is honoured
    /// as written — and a stale timer is still discarded rather than rolled
    /// forward to tomorrow.
    #[test]
    fn re_resolution_never_resurrects_a_stale_timer() {
        let (timer, power) = timer();
        let now = Utc::now();
        let local = now.with_timezone(&chrono::Local) - Duration::hours(2);
        let stale = PersistedTimer {
            id: Uuid::new_v4(),
            action: PowerAction::Shutdown,
            // Already passed while the app was closed.
            target: now - Duration::hours(1),
            scheduled_at: now - Duration::hours(3),
            mode: Some(ScheduleMode::At {
                hour: local.format("%H").to_string().parse().expect("hour"),
                minute: local.format("%M").to_string().parse().expect("minute"),
            }),
        };

        assert!(
            !timer.rearm(stale, now),
            "a target that has passed is discarded, not rolled on to tomorrow"
        );
        assert_eq!(timer.snapshot(), None);
        assert!(power.is_empty());

        // A duration keeps its instant untouched.
        let duration = PersistedTimer {
            id: Uuid::new_v4(),
            action: PowerAction::Restart,
            target: now + Duration::minutes(30),
            scheduled_at: now,
            mode: Some(ScheduleMode::In { minutes: 45 }),
        };
        assert!(timer.rearm(duration.clone(), now));
        assert_eq!(timer.snapshot().expect("armed").target, duration.target);
    }

    /// L3: a timer written before `mode` existed still re-arms at its instant.
    #[test]
    fn a_timer_persisted_without_a_mode_still_rearms() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "action": "shutdown",
            "target": "2030-01-01T10:30:00Z",
            "scheduledAt": "2030-01-01T10:00:00Z"
        }"#;
        let persisted: PersistedTimer = serde_json::from_str(json).expect("older format");
        assert_eq!(persisted.mode, None);

        let (timer, _) = timer();
        let now = DateTime::parse_from_rfc3339("2030-01-01T10:00:00Z")
            .expect("parse")
            .with_timezone(&Utc);
        assert!(timer.rearm(persisted.clone(), now));
        assert_eq!(timer.snapshot().expect("armed").target, persisted.target);
    }

    #[test]
    fn the_schedule_mode_wire_format_is_a_tagged_union() {
        assert_eq!(
            serde_json::to_string(&ScheduleMode::In { minutes: 30 }).expect("serialize"),
            r#"{"mode":"in","minutes":30}"#
        );
        assert_eq!(
            serde_json::to_string(&ScheduleMode::At {
                hour: 23,
                minute: 5
            })
            .expect("serialize"),
            r#"{"mode":"at","hour":23,"minute":5}"#
        );
    }

    #[test]
    fn status_reports_the_remaining_time_and_the_dry_run_flag() {
        let (timer, _) = timer();
        let now = at(10, 0);
        assert_eq!(
            timer.status(now),
            TimerStatus {
                active: None,
                dry_run: true
            }
        );

        timer
            .schedule(ScheduleMode::In { minutes: 2 }, PowerAction::Shutdown, now)
            .expect("schedule");
        let status = timer.status(now + Duration::seconds(30));
        let active = status.active.expect("active");
        assert_eq!(active.remaining_secs, 90);
        assert!(status.dry_run);
    }

    #[test]
    fn remaining_time_never_goes_negative() {
        assert_eq!(remaining_secs(at(10, 0), at(11, 0)), 0);
        assert_eq!(remaining_secs(at(11, 0), at(10, 0)), 3600);
    }

    #[test]
    fn the_tray_countdown_format_reads_naturally() {
        assert_eq!(format_remaining(0), "0s");
        assert_eq!(format_remaining(9), "9s");
        assert_eq!(format_remaining(90), "1m 30s");
        assert_eq!(format_remaining(3_600), "1h 00m 00s");
        assert_eq!(format_remaining(9_030), "2h 30m 30s");
        assert_eq!(format_remaining(-5), "0s");
    }

    #[test]
    fn the_manifest_declares_no_scan() {
        let module = ShutdownTimerModule;
        let manifest = module.manifest();
        assert_eq!(manifest.id, module.id());
        assert!(!manifest.capabilities.has_scan);
        assert!(!manifest.capabilities.has_execute);
        assert!(manifest.capabilities.has_settings);
    }

    #[test]
    fn the_default_timer_can_never_reach_the_real_power_api() {
        assert!(
            ShutdownTimer::default().is_dry_run(),
            "a timer built by accident must not be able to shut anything down"
        );
    }
}
