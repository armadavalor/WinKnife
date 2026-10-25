//! Running SFC and DISM for the repair kit (module C6).
//!
//! Why here: spawning a System32 tool and turning its pipe bytes into text is
//! platform work. What the text *means* lives in `modules/repair.rs`.
//!
//! The pipe encoding differs per tool: `sfc.exe` writes UTF-16LE when its
//! output is redirected, `dism.exe` writes the OEM code page. [`OutputDecoder`]
//! tells them apart from the first bytes.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::core::error::{CoreError, Result};
use crate::core::scanner::CancellationToken;

use super::{EnvResolver, SystemEnv};

/// `CREATE_NO_WINDOW`: no console window flashes up behind the app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How often the run loop wakes to look at the cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Bytes read from a pipe at a time.
const READ_CHUNK: usize = 4096;

/// Spawns a repair tool and streams its output.
pub trait RepairRunner: Send + Sync {
    /// Whether this process runs with administrator rights. SFC and DISM
    /// refuse to run without them.
    fn is_elevated(&self) -> bool;

    /// Run `exe` (a file name under `%SystemRoot%\System32`) with `args`,
    /// handing each piece of decoded output to `on_text` as it arrives.
    ///
    /// Returns the exit code, or `None` when `cancel` fired and the child was
    /// killed.
    ///
    /// # Errors
    /// Returns an error when the tool cannot be started.
    fn run(
        &self,
        exe: &str,
        args: &[&str],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Option<i32>>;
}

/// The real runner. Refuses until `run()` has armed real OS effects.
#[derive(Debug, Default)]
pub struct SystemRepairRunner;

impl RepairRunner for SystemRepairRunner {
    fn is_elevated(&self) -> bool {
        super::is_elevated()
    }

    fn run(
        &self,
        exe: &str,
        args: &[&str],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Option<i32>> {
        if !super::real_os_effects_armed() {
            return Err(CoreError::Repair(
                "this build must never run SFC or DISM".to_owned(),
            ));
        }
        let root = SystemEnv
            .var("SystemRoot")
            .ok_or_else(|| CoreError::Repair("%SystemRoot% is not set".to_owned()))?;
        // The full path, so a planted `sfc.exe` on PATH is never run.
        let mut command = Command::new(Path::new(&root).join("System32").join(exe));
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, CREATE_NO_WINDOW);
        let mut child = command.spawn()?;

        let (tx, rx) = mpsc::channel::<(usize, Vec<u8>)>();
        let pipes: [Option<Box<dyn Read + Send>>; 2] = [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ];
        for (index, pipe) in pipes.into_iter().enumerate() {
            let Some(mut pipe) = pipe else { continue };
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut buf = vec![0_u8; READ_CHUNK];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 || tx.send((index, buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut decoders = [OutputDecoder::default(), OutputDecoder::default()];
        loop {
            if cancel.is_cancelled() {
                // shortcut: kills dism.exe / sfc.exe only; a DismHost or
                // TrustedInstaller step already started may finish on its own.
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            match rx.recv_timeout(CANCEL_POLL) {
                Ok((index, bytes)) => on_text(&decoders[index].push(&bytes)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        for decoder in &mut decoders {
            on_text(&decoder.finish());
        }
        Ok(child.wait()?.code())
    }
}

/// Test and dev runner: prints a canned transcript, never spawns anything.
#[derive(Debug)]
pub struct FakeRepairRunner {
    /// What [`RepairRunner::is_elevated`] answers.
    pub elevated: bool,
}

impl RepairRunner for FakeRepairRunner {
    fn is_elevated(&self) -> bool {
        self.elevated
    }

    fn run(
        &self,
        exe: &str,
        args: &[&str],
        _cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Option<i32>> {
        on_text(&format!(
            "[dry run] {exe} {} was not started.\r\n",
            args.join(" ")
        ));
        on_text("[==========================100.0%==========================]\r\n");
        Ok(Some(0))
    }
}

/// Text encoding detected on a pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf16Le,
    Utf8,
    Oem,
}

/// Turns pipe bytes into text, carrying a split character over to the next
/// chunk.
#[derive(Debug, Default)]
pub struct OutputDecoder {
    encoding: Option<Encoding>,
    pending: Vec<u8>,
}

impl OutputDecoder {
    /// Decode as much of `bytes` (plus any carried-over tail) as is complete.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let encoding = match self.encoding {
            Some(encoding) => encoding,
            None if self.pending.len() < 2 => return String::new(),
            None => {
                let encoding = detect(&self.pending);
                if encoding == Encoding::Utf16Le && self.pending.starts_with(&[0xFF, 0xFE]) {
                    self.pending.drain(..2);
                }
                self.encoding = Some(encoding);
                encoding
            }
        };
        match encoding {
            Encoding::Utf16Le => self.take_utf16(),
            Encoding::Utf8 => self.take_utf8(),
            Encoding::Oem => oem_to_string(&std::mem::take(&mut self.pending)),
        }
    }

    /// Decode whatever is left at the end of the stream.
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.pending);
        match self.encoding {
            Some(Encoding::Utf16Le) => String::from_utf16_lossy(&utf16_units(&rest)),
            Some(Encoding::Oem) => oem_to_string(&rest),
            _ => String::from_utf8_lossy(&rest).into_owned(),
        }
    }

    fn take_utf16(&mut self) -> String {
        let mut units = utf16_units(&self.pending);
        // Keep a lone high surrogate for the next chunk; it pairs with the
        // first unit there.
        if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            units.pop();
        }
        self.pending.drain(..units.len() * 2);
        String::from_utf16_lossy(&units)
    }

    fn take_utf8(&mut self) -> String {
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_owned();
                self.pending.clear();
                text
            }
            // Cut mid-character: keep the incomplete tail for the next chunk.
            Err(err) if err.error_len().is_none() => {
                let tail = self.pending.split_off(err.valid_up_to());
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending = tail;
                text
            }
            // Not UTF-8 at all: DISM on a non-English Windows.
            Err(_) => {
                self.encoding = Some(Encoding::Oem);
                oem_to_string(&std::mem::take(&mut self.pending))
            }
        }
    }
}

/// UTF-16LE when the stream opens with a BOM or most odd bytes are zero (ASCII
/// text in UTF-16), UTF-8 otherwise.
fn detect(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Encoding::Utf16Le;
    }
    let odd: Vec<u8> = bytes.iter().skip(1).step_by(2).copied().collect();
    let zeros = odd.iter().filter(|b| **b == 0).count();
    if zeros * 2 > odd.len() {
        Encoding::Utf16Le
    } else {
        Encoding::Utf8
    }
}

fn utf16_units(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

/// Decode bytes in the console's OEM code page.
// shortcut: a double-byte OEM character split across two reads decodes as
// two replacement characters; upgrade if a CJK user reports garbled logs.
#[cfg(windows)]
fn oem_to_string(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{
        MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
    };
    if bytes.is_empty() {
        return String::new();
    }
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    // SAFETY: both calls only read `bytes` and write within `wide`'s length.
    let len = unsafe { MultiByteToWideChar(CP_OEMCP, flags, bytes, None) };
    let Ok(size) = usize::try_from(len) else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    let mut wide = vec![0_u16; size];
    let written = unsafe { MultiByteToWideChar(CP_OEMCP, flags, bytes, Some(&mut wide)) };
    match usize::try_from(written) {
        Ok(written) if written > 0 => String::from_utf16_lossy(&wide[..written]),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(not(windows))]
fn oem_to_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    #[test]
    fn utf16_from_sfc_decodes_across_odd_chunk_boundaries() {
        let bytes = utf16("Verification 100% complete.\r\n", false);
        let mut decoder = OutputDecoder::default();
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            text.push_str(&decoder.push(chunk));
        }
        text.push_str(&decoder.finish());
        assert_eq!(text, "Verification 100% complete.\r\n");
    }

    #[test]
    fn a_utf16_bom_is_dropped_and_a_split_surrogate_is_carried() {
        let bytes = utf16("ok \u{1F600}", true);
        let mut decoder = OutputDecoder::default();
        let split = bytes.len() - 2;
        let mut text = decoder.push(&bytes[..split]);
        text.push_str(&decoder.push(&bytes[split..]));
        assert_eq!(text, "ok \u{1F600}");
    }

    #[test]
    fn utf8_split_mid_character_is_carried_over() {
        let bytes = "Bileşen deposu".as_bytes();
        let cut = "Bile".len() + 1; // inside the two-byte `ş`
        let mut decoder = OutputDecoder::default();
        let mut text = decoder.push(&bytes[..cut]);
        text.push_str(&decoder.push(&bytes[cut..]));
        assert_eq!(text, "Bileşen deposu");
    }

    #[test]
    fn ascii_dism_output_is_plain_text() {
        let mut decoder = OutputDecoder::default();
        assert_eq!(
            decoder.push(b"No component store corruption detected.\r\n"),
            "No component store corruption detected.\r\n"
        );
    }

    #[test]
    fn invalid_utf8_falls_back_to_the_oem_page_without_losing_ascii() {
        let mut decoder = OutputDecoder::default();
        let text = decoder.push(b"Image \x9f ok");
        assert!(text.starts_with("Image "));
        assert!(text.ends_with(" ok"));
    }

    #[test]
    fn the_unarmed_system_runner_refuses() {
        let err = SystemRepairRunner
            .run(
                "sfc.exe",
                &["/scannow"],
                &CancellationToken::new(),
                &mut |_| {},
            )
            .expect_err("a test build must never run sfc");
        assert_eq!(err.code(), "repair");
    }
}
