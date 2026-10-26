//! Hosts editor (C4, K-019): view, edit and restore
//! `%SystemRoot%\System32\drivers\etc\hosts`.
//!
//! Everything here runs unelevated. It parses and validates the text, shows a
//! diff, keeps a timestamped backup in app data before every write and hands
//! the validated bytes to the elevated writer in `core/platform/hosts.rs`,
//! which owns the path fence. The file's encoding and line endings are kept;
//! empty or invalid content is never sent to the writer. Backups are never
//! deleted by WinKnife.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::core::error::{CoreError, Result};
use crate::core::module::{ModuleCapabilities, ModuleCategory, ModuleManifest, ToolModule};
use crate::core::platform::{
    content_hash, failure_from_exit_code, HelperRun, HostsFence, HostsWriter, EXIT_CHANGED,
    EXIT_INVALID, EXIT_VERIFY_FAILED, EXIT_WRITTEN, FLUSH_DNS_COMMAND,
};

/// Larger than any real hosts file; a guard against pasting a disk image.
pub const MAX_HOSTS_BYTES: usize = 16 * 1024 * 1024;
/// Windows rejects longer host names.
const MAX_HOSTNAME_CHARS: usize = 255;
/// Unchanged lines shown around each change in the diff.
const DIFF_CONTEXT: usize = 3;
/// Above this many cells the diff stops looking for moved lines and shows the
/// changed block as removed + added.
const DIFF_CELL_CAP: usize = 4_000_000;
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const BACKUP_PREFIX: &str = "hosts-";
const BACKUP_SUFFIX: &str = ".bak";

/// What one line of the file is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LineKind {
    /// Empty or whitespace only.
    Blank,
    /// A `#` comment; `disabled` when the comment is a commented-out entry.
    Comment {
        /// The text after the `#` is itself a valid entry.
        disabled: bool,
    },
    /// An address and its host names.
    Entry {
        /// As written, zone index included.
        ip: String,
        /// At least one.
        hostnames: Vec<String>,
        /// Text after a trailing `#`.
        comment: Option<String>,
    },
    /// Windows would ignore or misread this line; saving is refused.
    Invalid {
        /// One sentence.
        reason: String,
    },
}

/// One parsed line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsLine {
    /// 1-based.
    pub number: usize,
    /// The line as written.
    pub text: String,
    /// What it is.
    pub kind: LineKind,
}

fn parse_entry(body: &str) -> std::result::Result<(String, Vec<String>), String> {
    let mut fields = body.split_whitespace();
    let ip = fields.next().unwrap_or_default();
    // `fe80::1%4`: the zone index is not part of the address.
    let address = ip.split('%').next().unwrap_or_default();
    if address.parse::<IpAddr>().is_err() {
        return Err(format!("`{ip}` is not an IP address"));
    }
    let hostnames: Vec<String> = fields.map(str::to_owned).collect();
    if hostnames.is_empty() {
        return Err("no host name after the address".to_owned());
    }
    if let Some(bad) = hostnames.iter().find(|name| !is_hostname(name)) {
        return Err(format!("`{bad}` is not a valid host name"));
    }
    Ok((ip.to_owned(), hostnames))
}

fn is_hostname(name: &str) -> bool {
    name.len() <= MAX_HOSTNAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

/// Classify one line. Pure.
pub fn parse_line(text: &str) -> LineKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return LineKind::Blank;
    }
    if text.chars().any(|c| c.is_ascii_control() && c != '\t') {
        return LineKind::Invalid {
            reason: "the line holds a control character".to_owned(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        let inner = rest.trim_start_matches('#');
        let body = inner.split('#').next().unwrap_or_default();
        return LineKind::Comment {
            disabled: !body.trim().is_empty() && parse_entry(body).is_ok(),
        };
    }
    let (body, comment) = match trimmed.split_once('#') {
        Some((body, comment)) => (body, Some(comment.trim().to_owned())),
        None => (trimmed, None),
    };
    match parse_entry(body) {
        Ok((ip, hostnames)) => LineKind::Entry {
            ip,
            hostnames,
            comment,
        },
        Err(reason) => LineKind::Invalid { reason },
    }
}

/// Parse the whole text (`\n` line ends).
pub fn parse(text: &str) -> Vec<HostsLine> {
    // A final `\n` ends the last line; it does not start an empty one.
    let body = text.strip_suffix('\n').unwrap_or(text);
    if body.is_empty() && text.is_empty() {
        return Vec::new();
    }
    body.split('\n')
        .enumerate()
        .map(|(index, line)| HostsLine {
            number: index + 1,
            text: line.to_owned(),
            kind: parse_line(line),
        })
        .collect()
}

/// Why the text cannot be saved, one entry per problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// 1-based line, or `None` for the whole file.
    pub line: Option<usize>,
    /// One sentence.
    pub reason: String,
}

/// Every reason `text` may not be written. Empty means it may.
pub fn validate(text: &str) -> Vec<Problem> {
    if text.trim().is_empty() {
        return vec![Problem {
            line: None,
            reason: "the hosts file cannot be empty".to_owned(),
        }];
    }
    if text.len() > MAX_HOSTS_BYTES {
        return vec![Problem {
            line: None,
            reason: format!("the text is larger than {} MiB", MAX_HOSTS_BYTES >> 20),
        }];
    }
    parse(text)
        .into_iter()
        .filter_map(|line| match line.kind {
            LineKind::Invalid { reason } => Some(Problem {
                line: Some(line.number),
                reason,
            }),
            _ => None,
        })
        .collect()
}

/// How the file's bytes were stored, kept on save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    encoding: Encoding,
    crlf: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8 { bom: bool },
    /// Not UTF-8: one byte per character, kept byte for byte.
    Latin1,
}

impl Format {
    /// What a new hosts file gets: UTF-8 without BOM, CRLF.
    pub const NEW_FILE: Self = Self {
        encoding: Encoding::Utf8 { bom: false },
        crlf: true,
    };

    fn label(self) -> &'static str {
        match self.encoding {
            Encoding::Utf8 { bom: true } => "UTF-8 with BOM",
            Encoding::Utf8 { bom: false } => "UTF-8",
            Encoding::Latin1 => "ANSI",
        }
    }
}

/// Bytes → (`\n` text, format). Pure.
///
/// # Errors
/// Returns why the file cannot be edited here (UTF-16, broken UTF-8 after a BOM).
pub fn decode(bytes: &[u8]) -> std::result::Result<(String, Format), String> {
    if bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        return Err("the file is UTF-16; WinKnife edits only UTF-8 and ANSI hosts files".to_owned());
    }
    let (raw, encoding) = if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        let text = std::str::from_utf8(rest)
            .map_err(|_| "the file starts like UTF-8 but is not valid UTF-8".to_owned())?;
        (text.to_owned(), Encoding::Utf8 { bom: true })
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        (text.to_owned(), Encoding::Utf8 { bom: false })
    } else {
        (bytes.iter().map(|&b| char::from(b)).collect(), Encoding::Latin1)
    };
    let crlf_count = raw.matches("\r\n").count();
    let lf_count = raw.matches('\n').count() - crlf_count;
    let crlf = crlf_count >= lf_count;
    Ok((raw.replace("\r\n", "\n"), Format { encoding, crlf }))
}

/// `\n` text → bytes in `format`. Pure.
///
/// # Errors
/// Returns which character the file's encoding cannot hold.
pub fn encode(text: &str, format: Format) -> std::result::Result<Vec<u8>, String> {
    let text = if format.crlf {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    match format.encoding {
        Encoding::Utf8 { bom } => {
            let prefix = if bom { UTF8_BOM } else { &[] };
            Ok([prefix, text.as_bytes()].concat())
        }
        Encoding::Latin1 => text
            .chars()
            .map(|c| {
                u8::try_from(u32::from(c))
                    .map_err(|_| format!("`{c}` cannot be saved in this file's ANSI encoding"))
            })
            .collect(),
    }
}

/// One line of the diff preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum DiffLine {
    /// Unchanged context.
    Same {
        /// The line.
        text: String,
    },
    /// Only in the new text.
    Added {
        /// The line.
        text: String,
    },
    /// Only in the current file.
    Removed {
        /// The line.
        text: String,
    },
    /// Unchanged lines left out.
    Gap {
        /// How many.
        lines: usize,
    },
}

/// Line diff of `old` → `new`, unchanged runs cut to [`DIFF_CONTEXT`] lines.
pub fn diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut ops: Vec<DiffLine> = a[..prefix].iter().map(|t| same(t)).collect();
    ops.extend(middle_diff(mid_a, mid_b));
    ops.extend(a[a.len() - suffix..].iter().map(|t| same(t)));
    collapse(ops)
}

fn same(text: &str) -> DiffLine {
    DiffLine::Same {
        text: text.to_owned(),
    }
}

fn middle_diff(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let removed = |t: &&str| DiffLine::Removed { text: (*t).to_owned() };
    let added = |t: &&str| DiffLine::Added { text: (*t).to_owned() };
    if a.len().saturating_mul(b.len()) > DIFF_CELL_CAP {
        return a.iter().map(removed).chain(b.iter().map(added)).collect();
    }
    // lcs[i][j]: longest common run of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::with_capacity(a.len() + b.len()));
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(same(a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i * width + j + 1] >= lcs[(i + 1) * width + j]) {
            out.push(added(&b[j]));
            j += 1;
        } else {
            out.push(removed(&a[i]));
            i += 1;
        }
    }
    out
}

fn collapse(ops: Vec<DiffLine>) -> Vec<DiffLine> {
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, DiffLine::Same { .. }))
        .map(|(index, _)| index)
        .collect();
    let near_change = |index: usize| changed.iter().any(|&c| c.abs_diff(index) <= DIFF_CONTEXT);
    let mut out = Vec::new();
    let mut skipped = 0;
    for (index, op) in ops.into_iter().enumerate() {
        if matches!(op, DiffLine::Same { .. }) && !near_change(index) {
            skipped += 1;
            continue;
        }
        if skipped > 0 {
            out.push(DiffLine::Gap { lines: skipped });
            skipped = 0;
        }
        out.push(op);
    }
    if skipped > 0 && !out.is_empty() {
        out.push(DiffLine::Gap { lines: skipped });
    }
    out
}

/// What the page shows on open.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsDocument {
    /// The fenced path.
    pub path: String,
    /// `false`: there is no hosts file; saving creates one.
    pub exists: bool,
    /// `\n` line ends.
    pub text: String,
    /// "UTF-8", "UTF-8 with BOM" or "ANSI".
    pub encoding: String,
    /// "CRLF" or "LF".
    pub line_ending: String,
    /// Set when the file can be viewed but not edited here.
    pub read_only: Option<String>,
    /// Parsed `text`.
    pub lines: Vec<HostsLine>,
    /// The exact command behind the "Flush DNS cache" button.
    pub flush_dns_command: String,
}

/// The save dialog's content.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsPreview {
    /// Parsed new text.
    pub lines: Vec<HostsLine>,
    /// Empty when it may be saved.
    pub problems: Vec<Problem>,
    /// Current file → new text.
    pub diff: Vec<DiffLine>,
    /// `false`: saving would write nothing.
    pub changed: bool,
}

/// How a save or restore ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SaveState {
    /// The file was replaced and read back byte for byte.
    Written,
    /// The new text equals the file; nothing was written.
    NoChange,
    /// The user said No at the UAC prompt.
    Declined,
    /// Something else changed the file after WinKnife read it; nothing written.
    Changed,
    /// The helper refused the staged content.
    Invalid,
    /// The swap happened but the read-back differed; restore the backup.
    VerifyFailed,
    /// The helper did not finish in five minutes.
    TimedOut,
    /// Anything else, with the Win32 error code.
    Failed {
        /// The error the page shows.
        code: u32,
    },
}

/// A save's result and the backup taken before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOutcome {
    /// What happened.
    #[serde(flatten)]
    pub state: SaveState,
    /// The backup holding the file as it was, if one was needed.
    pub backup: Option<String>,
}

/// The exit-code → result table. Pure.
pub fn state_from_run(run: HelperRun) -> SaveState {
    match run {
        HelperRun::Declined => SaveState::Declined,
        HelperRun::TimedOut => SaveState::TimedOut,
        HelperRun::LaunchFailed(code) => SaveState::Failed { code },
        HelperRun::Exited(EXIT_WRITTEN) => SaveState::Written,
        HelperRun::Exited(EXIT_INVALID) => SaveState::Invalid,
        HelperRun::Exited(EXIT_CHANGED) => SaveState::Changed,
        HelperRun::Exited(EXIT_VERIFY_FAILED) => SaveState::VerifyFailed,
        HelperRun::Exited(code) => SaveState::Failed {
            code: failure_from_exit_code(code).unwrap_or(code),
        },
    }
}

/// One backup in app data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsBackup {
    /// File name; also its id.
    pub name: String,
    /// Bytes.
    pub size: u64,
    /// When it was taken.
    pub taken_at: DateTime<Utc>,
}

fn is_backup_name(name: &str) -> bool {
    name.starts_with(BACKUP_PREFIX)
        && name.ends_with(BACKUP_SUFFIX)
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
}

/// The app side of the module.
pub struct HostsService {
    fence: HostsFence,
    writer: Arc<dyn HostsWriter>,
    data_dir: PathBuf,
    busy: AtomicBool,
}

impl HostsService {
    /// Compose the service; backups and staged files live under `data_dir`.
    pub fn new(fence: HostsFence, writer: Arc<dyn HostsWriter>, data_dir: PathBuf) -> Self {
        Self {
            fence,
            writer,
            data_dir,
            busy: AtomicBool::new(false),
        }
    }

    fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("hosts-backups")
    }

    fn current_bytes(&self) -> Result<Option<Vec<u8>>> {
        match fs::read(self.fence.path()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// The current file as text and format; `None` text for a missing file.
    fn current(&self) -> Result<(Option<Vec<u8>>, String, Format)> {
        let bytes = self.current_bytes()?;
        let (text, format) = match &bytes {
            Some(bytes) => decode(bytes).map_err(CoreError::Hosts)?,
            None => (String::new(), Format::NEW_FILE),
        };
        Ok((bytes, text, format))
    }

    /// Read and parse the hosts file. Read only.
    ///
    /// # Errors
    /// Returns the I/O error when the file exists but cannot be read.
    pub fn read(&self) -> Result<HostsDocument> {
        let bytes = self.current_bytes()?;
        let (text, format, read_only) = match bytes.as_deref().map(decode) {
            None => (String::new(), Format::NEW_FILE, None),
            Some(Ok((text, format))) => (text, format, None),
            Some(Err(reason)) => (String::new(), Format::NEW_FILE, Some(reason)),
        };
        Ok(HostsDocument {
            path: self.fence.path().display().to_string(),
            exists: bytes.is_some(),
            lines: parse(&text),
            text,
            encoding: format.label().to_owned(),
            line_ending: if format.crlf { "CRLF" } else { "LF" }.to_owned(),
            read_only,
            flush_dns_command: FLUSH_DNS_COMMAND.to_owned(),
        })
    }

    /// Parse, validate and diff `text` against the file. Read only.
    ///
    /// # Errors
    /// Returns an error when the current file cannot be read or decoded.
    pub fn preview(&self, text: &str) -> Result<HostsPreview> {
        let (_, current, _) = self.current()?;
        let text = text.replace("\r\n", "\n");
        Ok(HostsPreview {
            lines: parse(&text),
            problems: validate(&text),
            diff: diff(&current, &text),
            changed: text != current,
        })
    }

    /// Validate `text`, back the file up and have the elevated helper write it.
    /// Blocks on the UAC prompt; call it off the UI thread.
    ///
    /// # Errors
    /// Returns an error for invalid text, while another save runs, or when the
    /// backup or the staging copy cannot be written.
    pub fn save(&self, text: &str) -> Result<SaveOutcome> {
        self.exclusive(|| {
            let (bytes, current, format) = self.current()?;
            let text = text.replace("\r\n", "\n");
            if bytes.is_some() && text == current {
                return Ok(no_change());
            }
            if let Some(problem) = validate(&text).into_iter().next() {
                let at = problem.line.map(|n| format!("line {n}: ")).unwrap_or_default();
                return Err(CoreError::Hosts(format!("{at}{}", problem.reason)));
            }
            let new_bytes = encode(&text, format).map_err(CoreError::Hosts)?;
            self.apply(bytes, &new_bytes)
        })
    }

    /// Put a backup's bytes back, after backing up the file as it is now.
    ///
    /// # Errors
    /// Returns an error for an unknown or empty backup, while another save
    /// runs, or when the file cannot be backed up first.
    pub fn restore(&self, name: &str) -> Result<SaveOutcome> {
        self.exclusive(|| {
            let backup = fs::read(self.backup_path(name)?)?;
            // Restore writes the backup as it was, invalid lines included, but
            // never an empty or undecodable file.
            if backup.is_empty() {
                return Err(CoreError::Hosts(format!("backup {name} is empty")));
            }
            decode(&backup).map_err(CoreError::Hosts)?;
            let bytes = self.current_bytes()?;
            self.apply(bytes, &backup)
        })
    }

    /// Every backup, newest first. Read only.
    ///
    /// # Errors
    /// Returns the I/O error when the backup folder cannot be listed.
    pub fn backups(&self) -> Result<Vec<HostsBackup>> {
        let entries = match fs::read_dir(self.backup_dir()) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut list = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata()?;
            if !meta.is_file() || !is_backup_name(&name) {
                continue;
            }
            list.push(HostsBackup {
                name,
                size: meta.len(),
                taken_at: meta.modified().map(DateTime::<Utc>::from)?,
            });
        }
        // The names carry the timestamp, so name order is time order.
        list.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(list)
    }

    /// Where backup `name` lives, for "Show in Explorer".
    ///
    /// # Errors
    /// Returns an error for a name that is not one of WinKnife's backups.
    pub fn backup_path(&self, name: &str) -> Result<PathBuf> {
        let path = self.backup_dir().join(name);
        if is_backup_name(name) && path.is_file() {
            Ok(path)
        } else {
            Err(CoreError::Hosts(format!("there is no backup named {name}")))
        }
    }

    fn exclusive(&self, job: impl FnOnce() -> Result<SaveOutcome>) -> Result<SaveOutcome> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(CoreError::Hosts("the hosts file is already being saved".to_owned()));
        }
        let result = job();
        self.busy.store(false, Ordering::SeqCst);
        if let Ok(outcome) = &result {
            tracing::info!(?outcome, "hosts save finished");
        }
        result
    }

    fn apply(&self, current: Option<Vec<u8>>, new_bytes: &[u8]) -> Result<SaveOutcome> {
        if current.as_deref() == Some(new_bytes) {
            return Ok(no_change());
        }
        if new_bytes.is_empty() {
            return Err(CoreError::Hosts("refusing to write an empty hosts file".to_owned()));
        }
        let backup = current.as_deref().map(|bytes| self.back_up(bytes)).transpose()?;
        let staged = self.stage(new_bytes)?;
        let expect = current.as_deref().map(content_hash);
        let run = self
            .writer
            .write(&staged, expect.as_deref(), &content_hash(new_bytes));
        Ok(SaveOutcome {
            state: state_from_run(run?),
            backup,
        })
    }

    fn back_up(&self, bytes: &[u8]) -> Result<String> {
        if let Some(newest) = self.backups()?.into_iter().next() {
            if fs::read(self.backup_dir().join(&newest.name))? == bytes {
                return Ok(newest.name);
            }
        }
        let dir = self.backup_dir();
        fs::create_dir_all(&dir)?;
        let name = format!(
            "{BACKUP_PREFIX}{}{BACKUP_SUFFIX}",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
        );
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(name)
    }

    fn stage(&self, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.data_dir.join("hosts-staging");
        fs::create_dir_all(&dir)?;
        // One fixed name, overwritten by each save (saves never overlap):
        // nothing to delete afterwards, and only quarantine removes files.
        let path = dir.join("hosts.staged");
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

fn no_change() -> SaveOutcome {
    SaveOutcome {
        state: SaveState::NoChange,
        backup: None,
    }
}

impl std::fmt::Debug for HostsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostsService")
            .field("path", &self.fence.path())
            .field("busy", &self.busy.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

/// Registry entry. No scan, no findings: an editor page.
#[derive(Debug, Default)]
pub struct HostsModule;

impl ToolModule for HostsModule {
    fn id(&self) -> &'static str {
        "hosts"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Hosts File".to_owned(),
            description: "View and edit the Windows hosts file, with a backup and diff before every save."
                .to_owned(),
            icon: "hosts".to_owned(),
            category: ModuleCategory::Network,
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
    use crate::core::platform::helper_write;
    use std::path::Path;
    use std::sync::Mutex;

    /// Runs the real helper logic in-process against a tempdir fence.
    struct FakeWriter {
        fence: HostsFence,
        decline: bool,
        calls: Mutex<usize>,
    }

    impl HostsWriter for FakeWriter {
        fn write(&self, staged: &Path, expect: Option<&str>, hash: &str) -> Result<HelperRun> {
            *self.calls.lock().unwrap() += 1;
            if self.decline {
                return Ok(HelperRun::Declined);
            }
            Ok(HelperRun::Exited(helper_write(&self.fence, staged, expect, hash)))
        }
    }

    struct Rig {
        _dir: tempfile::TempDir,
        hosts: PathBuf,
        writer: Arc<FakeWriter>,
        service: HostsService,
    }

    fn rig(initial: Option<&[u8]>, decline: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        let hosts = etc.join("hosts");
        if let Some(bytes) = initial {
            fs::write(&hosts, bytes).unwrap();
        }
        let fence = HostsFence::at(&hosts);
        let writer = Arc::new(FakeWriter {
            fence: fence.clone(),
            decline,
            calls: Mutex::new(0),
        });
        let service = HostsService::new(fence, writer.clone(), dir.path().join("data"));
        Rig {
            _dir: dir,
            hosts,
            writer,
            service,
        }
    }

    const SAMPLE: &[u8] = b"\xEF\xBB\xBF# Copyright\r\n127.0.0.1 localhost\r\n# 0.0.0.0 ads.example.com\r\n\r\n";

    #[test]
    fn lines_are_classified() {
        assert_eq!(parse_line("   "), LineKind::Blank);
        assert_eq!(parse_line("# hello there"), LineKind::Comment { disabled: false });
        assert_eq!(parse_line("#0.0.0.0 ads.test"), LineKind::Comment { disabled: true });
        assert_eq!(
            parse_line("::1\tlocalhost ip6-localhost # loopback"),
            LineKind::Entry {
                ip: "::1".to_owned(),
                hostnames: vec!["localhost".to_owned(), "ip6-localhost".to_owned()],
                comment: Some("loopback".to_owned()),
            }
        );
        assert!(matches!(parse_line("fe80::1%4 router"), LineKind::Entry { .. }));
        for bad in ["localhost 127.0.0.1", "127.0.0.1", "300.1.1.1 x", "1.2.3.4 a/b", "1.2.3.4 a\u{1}b"] {
            assert!(matches!(parse_line(bad), LineKind::Invalid { .. }), "{bad}");
        }
    }

    #[test]
    fn parse_numbers_lines_and_skips_the_final_newline() {
        let lines = parse("a\n\nb\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2].number, 3);
    }

    #[test]
    fn empty_or_invalid_text_is_refused() {
        assert!(!validate("").is_empty());
        assert!(!validate(" \n\t\n").is_empty());
        assert_eq!(validate("127.0.0.1 ok\nnot an entry\n")[0].line, Some(2));
        assert!(validate("# only a comment\n").is_empty());
    }

    #[test]
    fn decode_then_encode_is_byte_identical() {
        let latin1: &[u8] = b"# caf\xE9\n127.0.0.1 a\n";
        for bytes in [SAMPLE, b"127.0.0.1 a\n", latin1, b"no newline at end"] {
            let (text, format) = decode(bytes).unwrap();
            assert!(!text.contains('\r'));
            assert_eq!(encode(&text, format).unwrap(), bytes);
        }
        assert!(decode(b"\xFF\xFEa\0").is_err());
        let (_, latin) = decode(latin1).unwrap();
        assert!(encode("ş", latin).is_err());
    }

    #[test]
    fn diff_shows_changes_with_context() {
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 10\n", "line ten\n");
        let ops = diff(&old, &new);
        assert_eq!(ops.first(), Some(&DiffLine::Gap { lines: 6 }));
        assert!(ops.contains(&DiffLine::Removed { text: "line 10".to_owned() }));
        assert!(ops.contains(&DiffLine::Added { text: "line ten".to_owned() }));
        assert_eq!(ops.last(), Some(&DiffLine::Gap { lines: 7 }));
        assert!(diff(&old, &old).is_empty());
    }

    #[test]
    fn unchanged_text_round_trips_without_writing() {
        let rig = rig(Some(SAMPLE), false);
        let text = rig.service.read().unwrap().text;
        let outcome = rig.service.save(&text).unwrap();
        assert_eq!(outcome.state, SaveState::NoChange);
        assert_eq!(*rig.writer.calls.lock().unwrap(), 0);
        assert_eq!(fs::read(&rig.hosts).unwrap(), SAMPLE);
    }

    #[test]
    fn save_backs_up_writes_and_keeps_the_format() {
        let rig = rig(Some(SAMPLE), false);
        let text = rig.service.read().unwrap().text + "10.0.0.1 nas.lan\n";
        let outcome = rig.service.save(&text).unwrap();
        assert_eq!(outcome.state, SaveState::Written);
        let written = fs::read(&rig.hosts).unwrap();
        assert!(written.starts_with(UTF8_BOM));
        assert!(written.ends_with(b"10.0.0.1 nas.lan\r\n"));
        let backup = outcome.backup.unwrap();
        assert_eq!(fs::read(rig.service.backup_path(&backup).unwrap()).unwrap(), SAMPLE);
        // The staging copy is gone; no temp file was left next to hosts.
        assert_eq!(fs::read_dir(rig.hosts.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn invalid_or_empty_text_never_reaches_the_writer() {
        let rig = rig(Some(SAMPLE), false);
        assert!(rig.service.save("").is_err());
        assert!(rig.service.save("bogus line\n").is_err());
        assert_eq!(*rig.writer.calls.lock().unwrap(), 0);
        assert_eq!(fs::read(&rig.hosts).unwrap(), SAMPLE);
    }

    #[test]
    fn backup_write_restore_gives_back_the_original_bytes() {
        let rig = rig(Some(SAMPLE), false);
        let first = rig.service.save("127.0.0.1 localhost\n0.0.0.0 x.test\n").unwrap();
        assert_eq!(first.state, SaveState::Written);
        assert_ne!(fs::read(&rig.hosts).unwrap(), SAMPLE);
        let restored = rig.service.restore(&first.backup.unwrap()).unwrap();
        assert_eq!(restored.state, SaveState::Written);
        assert_eq!(fs::read(&rig.hosts).unwrap(), SAMPLE);
        // The edited version was backed up before the restore; nothing deleted.
        assert_eq!(rig.service.backups().unwrap().len(), 2);
    }

    #[test]
    fn a_declined_prompt_leaves_the_file_and_keeps_the_backup() {
        let rig = rig(Some(SAMPLE), true);
        let outcome = rig.service.save("127.0.0.1 other\n").unwrap();
        assert_eq!(outcome.state, SaveState::Declined);
        assert!(outcome.backup.is_some());
        assert_eq!(fs::read(&rig.hosts).unwrap(), SAMPLE);
    }

    #[test]
    fn a_file_changed_behind_our_back_is_not_overwritten() {
        struct Racer(FakeWriter, PathBuf);
        impl HostsWriter for Racer {
            fn write(&self, s: &Path, e: Option<&str>, h: &str) -> Result<HelperRun> {
                fs::write(&self.1, b"1.1.1.1 sneaky\n").unwrap();
                self.0.write(s, e, h)
            }
        }
        let rig = rig(Some(SAMPLE), false);
        let racer = Arc::new(Racer(
            FakeWriter {
                fence: HostsFence::at(&rig.hosts),
                decline: false,
                calls: Mutex::new(0),
            },
            rig.hosts.clone(),
        ));
        let service = HostsService::new(HostsFence::at(&rig.hosts), racer, rig.service.data_dir.clone());
        let outcome = service.save("127.0.0.1 mine\n").unwrap();
        assert_eq!(outcome.state, SaveState::Changed);
        assert_eq!(fs::read(&rig.hosts).unwrap(), b"1.1.1.1 sneaky\n");
    }

    #[test]
    fn an_identical_file_reuses_the_newest_backup() {
        let rig = rig(Some(SAMPLE), true);
        let a = rig.service.save("127.0.0.1 one\n").unwrap().backup;
        let b = rig.service.save("127.0.0.1 two\n").unwrap().backup;
        assert_eq!(a, b);
        assert_eq!(rig.service.backups().unwrap().len(), 1);
    }

    #[test]
    fn a_missing_file_is_created_without_a_backup() {
        let rig = rig(None, false);
        assert!(!rig.service.read().unwrap().exists);
        let outcome = rig.service.save("127.0.0.1 localhost\n").unwrap();
        assert_eq!(outcome, SaveOutcome { state: SaveState::Written, backup: None });
        assert_eq!(fs::read(&rig.hosts).unwrap(), b"127.0.0.1 localhost\r\n");
    }

    #[test]
    fn backup_names_cannot_escape_the_backup_folder() {
        let rig = rig(Some(SAMPLE), false);
        for name in ["..\\hosts", "hosts-..\\..\\x.bak", "C:\\Windows\\x.bak", "hosts-1.bak"] {
            assert!(rig.service.restore(name).is_err(), "{name}");
        }
        assert_eq!(*rig.writer.calls.lock().unwrap(), 0);
    }

    #[test]
    fn helper_runs_map_to_states() {
        assert_eq!(state_from_run(HelperRun::Exited(EXIT_WRITTEN)), SaveState::Written);
        assert_eq!(state_from_run(HelperRun::Exited(EXIT_CHANGED)), SaveState::Changed);
        assert_eq!(state_from_run(HelperRun::Declined), SaveState::Declined);
        assert_eq!(state_from_run(HelperRun::LaunchFailed(2)), SaveState::Failed { code: 2 });
    }
}
