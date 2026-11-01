//! Opt-in "a new version is out" reminder.
//!
//! WinKnife makes no network calls unless the user turns this on. When on, it
//! asks GitHub's Releases API for the latest release of this one repository at
//! most once a day, compares the version, and lets the UI show a banner. It
//! never downloads or installs anything: the banner opens the release page in
//! the default browser and the user takes it from there.
//!
//! The URL is a constant and the UI never supplies one. Any failure (offline,
//! rate limit, bad JSON) is logged and otherwise ignored.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, Result};

/// The only endpoint this module ever contacts.
const RELEASES_URL: &str = "https://api.github.com/repos/armadavalor/WinKnife/releases/latest";
/// Release pages must live here, or the link is ignored.
const URL_PREFIX: &str = "https://github.com/armadavalor/WinKnife/releases/";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const CHECK_INTERVAL_MS: i64 = DAY_MS;
const SNOOZE_MS: i64 = 7 * DAY_MS;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// A release JSON is a few KB; anything bigger is not what we asked for.
const MAX_BODY_BYTES: u64 = 1 << 20;
/// Module id under which the state is stored (`module.update-check`).
pub const SETTINGS_ID: &str = "update-check";

/// Everything persisted for the reminder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateSettings {
    /// The user said yes. Off until they do.
    pub enabled: bool,
    /// The one-time question has been answered (either way).
    pub prompted: bool,
    /// Epoch ms of the last attempt, successful or not.
    pub last_check_ms: i64,
    /// Newest stable version seen, e.g. `0.6.0`.
    pub latest_version: Option<String>,
    /// Its release page, already checked against [`URL_PREFIX`].
    pub latest_url: Option<String>,
    /// Hide the banner until this epoch ms.
    pub snooze_until_ms: i64,
    /// A version the user waved away for good.
    pub skipped_version: Option<String>,
}

/// What the banner shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNotice {
    /// Version without a leading `v`.
    pub version: String,
    /// Release page URL.
    pub url: String,
}

/// What the UI gets back from every command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    /// Checking is on.
    pub enabled: bool,
    /// The opt-in question was already answered.
    pub prompted: bool,
    /// Set when a newer version should be announced right now.
    pub notice: Option<UpdateNotice>,
}

/// Build the UI view of `settings` at time `now_ms`.
#[must_use]
pub fn state(settings: &UpdateSettings, current: &str, now_ms: i64) -> UpdateState {
    UpdateState {
        enabled: settings.enabled,
        prompted: settings.prompted,
        notice: notice(settings, current, now_ms),
    }
}

/// Parse `1.2.3` / `v1.2.3` into a comparable triple. Pre-release or build
/// suffixes (`1.2.3-rc1`) are rejected: we only announce stable releases.
#[must_use]
pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// `candidate` is a valid version strictly newer than `current`.
#[must_use]
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(c), Some(k)) if c > k)
}

/// A check is due when none has happened yet, a day has passed, or the clock
/// went backwards (so a bad clock cannot silence checks forever).
#[must_use]
pub fn check_due(last_check_ms: i64, now_ms: i64) -> bool {
    last_check_ms <= 0 || now_ms < last_check_ms || now_ms - last_check_ms >= CHECK_INTERVAL_MS
}

/// Whether `url` is a release page of this repository.
#[must_use]
pub fn is_release_url(url: &str) -> bool {
    url.starts_with(URL_PREFIX)
}

/// Pull `(version, html_url)` out of a `releases/latest` body. Drafts,
/// pre-releases, unparsable tags and foreign URLs yield `None`.
#[must_use]
pub fn parse_release(body: &str) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }
    let release: Release = serde_json::from_str(body).ok()?;
    if release.draft || release.prerelease || !is_release_url(&release.html_url) {
        return None;
    }
    let (major, minor, patch) = parse_version(&release.tag_name)?;
    Some((format!("{major}.{minor}.{patch}"), release.html_url))
}

/// The banner to show, if any: a stored newer version that is neither snoozed
/// nor skipped.
#[must_use]
pub fn notice(settings: &UpdateSettings, current: &str, now_ms: i64) -> Option<UpdateNotice> {
    if !settings.enabled || now_ms < settings.snooze_until_ms {
        return None;
    }
    let version = settings.latest_version.as_deref()?;
    let url = settings.latest_url.as_deref()?;
    if settings.skipped_version.as_deref() == Some(version)
        || !is_newer(version, current)
        || !is_release_url(url)
    {
        return None;
    }
    Some(UpdateNotice {
        version: version.to_owned(),
        url: url.to_owned(),
    })
}

/// Record a finished attempt. `found` is `None` when the request failed; the
/// previous result is kept and the attempt still counts for the throttle.
#[must_use]
pub fn after_check(
    settings: &UpdateSettings,
    now_ms: i64,
    found: Option<(String, String)>,
) -> UpdateSettings {
    let mut next = settings.clone();
    next.last_check_ms = now_ms;
    if let Some((version, url)) = found {
        next.latest_version = Some(version);
        next.latest_url = Some(url);
    }
    next
}

/// Answer the opt-in question.
#[must_use]
pub fn with_enabled(settings: &UpdateSettings, enabled: bool) -> UpdateSettings {
    UpdateSettings {
        enabled,
        prompted: true,
        ..settings.clone()
    }
}

/// "Later": hide the banner for a week.
#[must_use]
pub fn snoozed(settings: &UpdateSettings, now_ms: i64) -> UpdateSettings {
    UpdateSettings {
        snooze_until_ms: now_ms.saturating_add(SNOOZE_MS),
        ..settings.clone()
    }
}

/// "Skip this version": stay quiet until a newer one appears.
#[must_use]
pub fn skipped(settings: &UpdateSettings) -> UpdateSettings {
    UpdateSettings {
        skipped_version: settings.latest_version.clone(),
        ..settings.clone()
    }
}

/// One HTTPS GET to the constant releases URL. Returns the newest stable
/// release, or `None` when the latest one is a draft, pre-release or foreign.
///
/// # Errors
/// Returns an error when this build must not reach the network, or the
/// request fails.
pub fn fetch_latest() -> Result<Option<(String, String)>> {
    if !super::platform::real_os_effects_armed() {
        return Err(CoreError::Network(
            "this build must never reach the network".to_owned(),
        ));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(RELEASES_URL)
        .header("User-Agent", "WinKnife")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| CoreError::Network(err.to_string()))?;
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .map_err(|err| CoreError::Network(err.to_string()))?;
    Ok(parse_release(&body))
}

/// Open a release page in the default browser.
///
/// # Errors
/// Returns an error when `url` is not a release page of this repository, when
/// this build must not start processes, or the browser could not be started.
pub fn open_release_page(url: &str) -> Result<()> {
    if !is_release_url(url) {
        return Err(CoreError::Network(
            "that is not a WinKnife release page".to_owned(),
        ));
    }
    if !super::platform::real_os_effects_armed() {
        return Err(CoreError::Network(
            "this build must never open a browser".to_owned(),
        ));
    }
    spawn_browser(url)
}

#[cfg(windows)]
fn spawn_browser(url: &str) -> Result<()> {
    use std::process::{Command, Stdio};

    // Explorer hands a URL to the default browser; the URL is one argument, so
    // nothing is shell-interpreted.
    Command::new("explorer.exe")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| CoreError::Network(format!("could not open the browser: {err}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_browser(_url: &str) -> Result<()> {
    Err(CoreError::Network(
        "there is no default browser launcher on this platform".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/armadavalor/WinKnife/releases/tag/v0.6.0";

    fn enabled_with(version: &str) -> UpdateSettings {
        UpdateSettings {
            enabled: true,
            prompted: true,
            latest_version: Some(version.to_owned()),
            latest_url: Some(URL.to_owned()),
            ..UpdateSettings::default()
        }
    }

    #[test]
    fn version_comparison() {
        let cases = [
            ("0.6.0", "0.5.0", true),
            ("v0.5.1", "0.5.0", true),
            ("0.10.0", "0.9.9", true),
            ("1.0.0", "0.99.99", true),
            ("0.5.0", "0.5.0", false),
            ("0.4.9", "0.5.0", false),
            ("0.6.0-rc1", "0.5.0", false),
            ("0.6", "0.5.0", false),
            ("0.6.0.1", "0.5.0", false),
            ("nonsense", "0.5.0", false),
            ("0.6.0", "nonsense", false),
        ];
        for (candidate, current, expected) in cases {
            assert_eq!(
                is_newer(candidate, current),
                expected,
                "{candidate} vs {current}"
            );
        }
    }

    #[test]
    fn throttle_is_one_day() {
        let t = 1_000_000_000_000;
        let cases = [
            (0, t, true),
            (t, t + DAY_MS - 1, false),
            (t, t + DAY_MS, true),
            (t, t - 1, true),
        ];
        for (last, now, due) in cases {
            assert_eq!(check_due(last, now), due, "last {last} now {now}");
        }
    }

    #[test]
    fn release_url_guard() {
        assert!(is_release_url(URL));
        assert!(!is_release_url(
            "https://github.com/evil/WinKnife/releases/tag/v9"
        ));
        assert!(!is_release_url(
            "http://github.com/armadavalor/WinKnife/releases/tag/v9"
        ));
        assert!(!is_release_url(
            "https://github.com/armadavalor/WinKnife/releases.evil.com/x"
        ));
        assert!(!is_release_url("https://github.com/armadavalor/WinKnife"));
        assert!(open_release_page("https://example.com/").is_err());
    }

    #[test]
    fn release_parsing() {
        let ok = format!(
            r#"{{"tag_name":"v0.6.0","html_url":"{URL}","draft":false,"prerelease":false}}"#
        );
        assert_eq!(
            parse_release(&ok),
            Some(("0.6.0".to_owned(), URL.to_owned()))
        );
        let cases = [
            format!(r#"{{"tag_name":"v0.6.0","html_url":"{URL}","draft":true}}"#),
            format!(r#"{{"tag_name":"v0.6.0","html_url":"{URL}","prerelease":true}}"#),
            r#"{"tag_name":"v0.6.0","html_url":"https://evil.example/x"}"#.to_owned(),
            format!(r#"{{"tag_name":"latest","html_url":"{URL}"}}"#),
            "not json".to_owned(),
            r#"{"message":"Not Found"}"#.to_owned(),
        ];
        for body in cases {
            assert_eq!(parse_release(&body), None, "{body}");
        }
    }

    #[test]
    fn notice_honours_enable_snooze_and_skip() {
        let now = 5 * DAY_MS;
        let base = enabled_with("0.6.0");
        assert!(notice(&base, "0.5.0", now).is_some());

        // Not newer, or opted out: silent.
        assert!(notice(&base, "0.6.0", now).is_none());
        assert!(notice(
            &UpdateSettings {
                enabled: false,
                ..base.clone()
            },
            "0.5.0",
            now
        )
        .is_none());

        // Snooze for a week, then it returns.
        let later = snoozed(&base, now);
        assert!(notice(&later, "0.5.0", now).is_none());
        assert!(notice(&later, "0.5.0", now + SNOOZE_MS - 1).is_none());
        assert!(notice(&later, "0.5.0", now + SNOOZE_MS).is_some());

        // Skip hides that version only.
        let skip = skipped(&base);
        assert!(notice(&skip, "0.5.0", now).is_none());
        let newer = UpdateSettings {
            latest_version: Some("0.7.0".to_owned()),
            ..skip
        };
        assert!(notice(&newer, "0.5.0", now).is_some());

        // A tampered stored URL is never offered.
        let bad = UpdateSettings {
            latest_url: Some("https://evil.example/".to_owned()),
            ..base
        };
        assert!(notice(&bad, "0.5.0", now).is_none());
    }

    #[test]
    fn failed_check_keeps_old_result_but_counts_for_throttle() {
        let base = enabled_with("0.6.0");
        let failed = after_check(&base, 42, None);
        assert_eq!(failed.last_check_ms, 42);
        assert_eq!(failed.latest_version, base.latest_version);
        let found = after_check(&base, 43, Some(("0.7.0".to_owned(), URL.to_owned())));
        assert_eq!(found.latest_version.as_deref(), Some("0.7.0"));
    }

    #[test]
    fn answering_the_prompt_is_remembered() {
        let no = with_enabled(&UpdateSettings::default(), false);
        assert!(!no.enabled && no.prompted);
        let yes = with_enabled(&no, true);
        assert!(yes.enabled && yes.prompted);
    }

    /// A test run must never touch the network or a browser.
    #[test]
    fn a_test_process_cannot_reach_the_network() {
        assert!(fetch_latest().is_err());
        assert!(open_release_page(URL).is_err());
    }
}
