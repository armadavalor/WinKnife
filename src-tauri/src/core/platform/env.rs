//! Environment-variable expansion for rule-pack paths.
//!
//! Why an indirection over `std::env::var`: rule packs are written as
//! `%TEMP%\**`. A test that expanded that for real would walk the developer's
//! actual temp folder, which CONVENTIONS.md forbids ("never scan the real
//! machine in tests"). Fixtures inject a [`MapEnv`] pointing at a tempdir, so
//! the exact same rule pack that ships is the one the tests exercise.

use std::collections::BTreeMap;

/// Resolves `%NAME%` placeholders found in rule-pack paths.
pub trait EnvResolver: Send + Sync {
    /// Value of `name`, or `None` when it is not set.
    fn var(&self, name: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnv;

impl EnvResolver for SystemEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// A fixed map, for fixtures and tests. Lookups are case-insensitive, like
/// Windows' own environment block.
#[derive(Debug, Clone, Default)]
pub struct MapEnv {
    vars: BTreeMap<String, String>,
}

impl MapEnv {
    /// An empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a variable.
    #[must_use]
    pub fn with(mut self, name: &str, value: impl Into<String>) -> Self {
        self.vars.insert(name.to_ascii_uppercase(), value.into());
        self
    }
}

impl EnvResolver for MapEnv {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(&name.to_ascii_uppercase()).cloned()
    }
}

/// Result of expanding a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expansion {
    /// Every placeholder resolved.
    Expanded(String),
    /// A placeholder had no value; the name is returned so the caller can say
    /// which one, and skip the rule instead of walking a nonsense path.
    Missing(String),
}

/// Replace every `%NAME%` in `pattern` using `env`.
///
/// A literal `%` that is not part of a well-formed `%NAME%` pair is left alone,
/// so a path containing a stray percent sign does not silently vanish.
pub fn expand(pattern: &str, env: &dyn EnvResolver) -> Expansion {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;

    while let Some(open) = rest.find('%') {
        let (before, after_open) = rest.split_at(open);
        out.push_str(before);
        let after_open = &after_open[1..];

        let Some(close) = after_open.find('%') else {
            // No closing percent: the rest is literal.
            out.push('%');
            out.push_str(after_open);
            return Expansion::Expanded(out);
        };

        let name = &after_open[..close];
        if name.is_empty() || name.contains(['\\', '/']) {
            // `%%` or `%C:\x%` — not a variable reference.
            out.push('%');
            rest = after_open;
            continue;
        }

        match env.var(name) {
            Some(value) => out.push_str(value.trim_end_matches(['\\', '/'])),
            None => return Expansion::Missing(name.to_owned()),
        }
        rest = &after_open[close + 1..];
    }

    out.push_str(rest);
    Expansion::Expanded(out)
}

/// Tell running programs (Explorer, new shells) that the user environment
/// changed, the way the System Properties dialog does. A hung window is
/// skipped after a short wait. No-op unless real OS effects are armed.
pub fn broadcast_environment_change() {
    if !super::real_os_effects_armed() {
        return;
    }
    #[cfg(windows)]
    sys::broadcast();
}

#[cfg(windows)]
mod sys {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    /// Per-window wait before a non-responding window is skipped.
    const TIMEOUT_MS: u32 = 1000;

    pub(super) fn broadcast() {
        let area = w!("Environment");
        // SAFETY: `area` is a static NUL-terminated string that outlives the call.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(area.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                TIMEOUT_MS,
                None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> MapEnv {
        MapEnv::new()
            .with("TEMP", "C:\\Users\\dev\\AppData\\Local\\Temp")
            .with("WINDIR", "C:\\Windows\\")
    }

    #[test]
    fn expands_a_single_placeholder() {
        assert_eq!(
            expand("%TEMP%\\**", &env()),
            Expansion::Expanded("C:\\Users\\dev\\AppData\\Local\\Temp\\**".to_owned())
        );
    }

    #[test]
    fn lookup_is_case_insensitive_like_windows() {
        assert_eq!(
            expand("%temp%\\a", &env()),
            Expansion::Expanded("C:\\Users\\dev\\AppData\\Local\\Temp\\a".to_owned())
        );
    }

    #[test]
    fn trailing_separators_in_the_value_are_dropped() {
        assert_eq!(
            expand("%WINDIR%\\Temp\\**", &env()),
            Expansion::Expanded("C:\\Windows\\Temp\\**".to_owned())
        );
    }

    #[test]
    fn expands_several_placeholders() {
        let resolved = expand("%WINDIR%;%TEMP%", &env());
        assert_eq!(
            resolved,
            Expansion::Expanded("C:\\Windows;C:\\Users\\dev\\AppData\\Local\\Temp".to_owned())
        );
    }

    #[test]
    fn an_unset_variable_is_reported_by_name() {
        assert_eq!(
            expand("%NOPE%\\x", &env()),
            Expansion::Missing("NOPE".to_owned())
        );
    }

    #[test]
    fn a_path_without_placeholders_is_returned_verbatim() {
        assert_eq!(
            expand("C:\\Windows\\Temp", &env()),
            Expansion::Expanded("C:\\Windows\\Temp".to_owned())
        );
    }

    #[test]
    fn a_stray_percent_sign_is_left_alone() {
        assert_eq!(
            expand("C:\\100% done\\x", &env()),
            Expansion::Expanded("C:\\100% done\\x".to_owned())
        );
        assert_eq!(expand("50%", &env()), Expansion::Expanded("50%".to_owned()));
    }

    #[test]
    fn the_system_env_reads_the_real_process_environment() {
        // `PATH` exists on every platform this builds for; the value is not
        // asserted on, only that the resolver reaches the real environment.
        assert!(SystemEnv.var("PATH").is_some());
        assert_eq!(SystemEnv.var("WINKNIFE_DEFINITELY_NOT_SET"), None);
    }
}
