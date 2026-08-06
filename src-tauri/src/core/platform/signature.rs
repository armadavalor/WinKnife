//! Authenticode signature checks, behind a trait so tests never touch a real
//! binary's certificate chain.
//!
//! # Why the startup manager needs this
//!
//! Module spec 05 rates an autostart entry partly on whether it is signed and by
//! whom: "Unknown unsigned entries → NeedsReview with `unsigned, unknown
//! publisher` reason." A signature is the only cheap, offline evidence about who
//! wrote a program, and it is evidence about the *file*, so it belongs behind the
//! same kind of wrapper the lock probe and the process probe use.
//!
//! # What a verdict does and does not mean
//!
//! [`Signature::Signed`] means Windows itself verified the embedded Authenticode
//! signature against the machine's trust store *right now*. It does not mean the
//! program is safe; it means the publisher is accountable. [`Signature::Unknown`]
//! is returned whenever the answer could not be obtained — a missing file, a
//! revocation check that could not run offline — and is deliberately distinct
//! from [`Signature::Unsigned`], because "we could not tell" and "there is no
//! signature" are different things to show a user.
//!
//! # Read-only
//!
//! `WinVerifyTrust` opens the file for reading. Like the lock probe
//! (`lock.rs:173`), the handle-based path suppresses last-access updates —
//! except that here Windows owns the handle, so instead the check is only ever
//! run on files the scan has already decided to report, and never in a loop over
//! a whole tree. Autostart entries number in the dozens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What is known about a file's Authenticode signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signature {
    /// Verified against the machine's trust store.
    Signed {
        /// Subject name of the signing certificate, when it could be read.
        publisher: Option<String>,
    },
    /// The file carries no valid Authenticode signature.
    Unsigned,
    /// The question could not be answered.
    Unknown {
        /// Why, in words that can be shown to a person.
        reason: String,
    },
}

impl Signature {
    /// Whether the file is signed and verified.
    pub fn is_signed(&self) -> bool {
        matches!(self, Self::Signed { .. })
    }

    /// The publisher, when one is known.
    pub fn publisher(&self) -> Option<&str> {
        match self {
            Self::Signed { publisher } => publisher.as_deref(),
            _ => None,
        }
    }

    /// A short phrase for the UI and for a recommendation's `reason`.
    pub fn describe(&self) -> String {
        match self {
            Self::Signed {
                publisher: Some(name),
            } => format!("signed by {name}"),
            Self::Signed { publisher: None } => "signed, publisher unreadable".to_owned(),
            Self::Unsigned => "unsigned, unknown publisher".to_owned(),
            Self::Unknown { reason } => format!("signature could not be checked: {reason}"),
        }
    }
}

/// Ask about a file's signature.
pub trait SignatureProbe: Send + Sync {
    /// Check `path`. Never fails: an unanswerable question is
    /// [`Signature::Unknown`], because a startup list must still render.
    fn check(&self, path: &Path) -> Signature;
}

/// A fake probe: answers from a table, with a configurable default.
#[derive(Debug, Default)]
pub struct MapSignatures {
    answers: HashMap<PathBuf, Signature>,
    default: Option<Signature>,
}

impl MapSignatures {
    /// A probe that reports everything as unsigned unless told otherwise.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer `signature` for `path`.
    #[must_use]
    pub fn with(mut self, path: impl Into<PathBuf>, signature: Signature) -> Self {
        self.answers.insert(path.into(), signature);
        self
    }

    /// Answer `signature` for everything not named explicitly.
    #[must_use]
    pub fn defaulting_to(mut self, signature: Signature) -> Self {
        self.default = Some(signature);
        self
    }
}

impl SignatureProbe for MapSignatures {
    fn check(&self, path: &Path) -> Signature {
        self.answers
            .get(path)
            .cloned()
            .or_else(|| self.default.clone())
            .unwrap_or(Signature::Unsigned)
    }
}

/// The probe that asks Windows.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemSignatureProbe;

impl SignatureProbe for SystemSignatureProbe {
    fn check(&self, path: &Path) -> Signature {
        #[cfg(windows)]
        {
            sys::check(path)
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Signature::Unknown {
                reason: "Authenticode is a Windows concept".to_owned(),
            }
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::Signature;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HANDLE, HWND, TRUST_E_NOSIGNATURE};
    use windows::Win32::Security::Cryptography::{
        CertCloseStore, CertFindCertificateInStore, CertFreeCertificateContext, CertGetNameStringW,
        CryptMsgClose, CryptMsgGetParam, CryptQueryObject, CERT_FIND_SUBJECT_CERT, CERT_INFO,
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
        CERT_QUERY_ENCODING_TYPE, CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_FILE,
        CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM, HCERTSTORE, PKCS_7_ASN_ENCODING,
        X509_ASN_ENCODING,
    };
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Verify the embedded signature, then read the signer's name.
    pub(super) fn check(path: &Path) -> Signature {
        if !path.is_file() {
            return Signature::Unknown {
                reason: format!("`{}` is not a file on this machine", path.display()),
            };
        }

        match verify(path) {
            Ok(true) => Signature::Signed {
                publisher: publisher(path),
            },
            Ok(false) => Signature::Unsigned,
            Err(reason) => Signature::Unknown { reason },
        }
    }

    /// `WinVerifyTrust` against the generic Authenticode policy.
    ///
    /// `Ok(false)` means "no signature"; an error means "could not decide", and
    /// the two are kept apart deliberately (an expired certificate or an
    /// unreachable revocation list is not the same as an unsigned binary).
    fn verify(path: &Path) -> std::result::Result<bool, String> {
        let file = wide(path);
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(file.as_ptr()),
            hFile: HANDLE::default(),
            pgKnownSubject: std::ptr::null_mut(),
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 {
                pFile: &mut file_info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

        // SAFETY: `data` and `action` are live locals for the whole call, and
        // `file_info` (pointed at by the union) outlives both. `WTD_UI_NONE`
        // guarantees no window is shown, which matters in a service-like process.
        let status = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            )
        };

        // The state handle has to be released whatever the verdict was.
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: same arguments, closing the state opened above.
        let _ = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            )
        };

        match status {
            0 => Ok(true),
            code if code == TRUST_E_NOSIGNATURE.0 => Ok(false),
            code => Err(format!("Windows reported 0x{:08X}", code as u32)),
        }
    }

    /// Subject name of the signing certificate, or `None` when it cannot be read.
    ///
    /// Only called after [`verify`] said the signature is good, so a failure here
    /// downgrades the display ("signed, publisher unreadable") rather than the
    /// verdict.
    fn publisher(path: &Path) -> Option<String> {
        let file = wide(path);
        let mut store = HCERTSTORE::default();
        let mut msg: *mut std::ffi::c_void = std::ptr::null_mut();

        // SAFETY: `file` is NUL-terminated and outlives the call; the two out
        // handles are valid pointers and are released below.
        let queried = unsafe {
            CryptQueryObject(
                CERT_QUERY_OBJECT_FILE,
                file.as_ptr().cast(),
                CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
                CERT_QUERY_FORMAT_FLAG_BINARY,
                0,
                None,
                None,
                None,
                Some(&mut store),
                Some(&mut msg),
                None,
            )
        };
        if queried.is_err() || msg.is_null() {
            return None;
        }

        let name = signer_name(store, msg);

        // SAFETY: both handles came from the successful call above.
        unsafe {
            let _ = CryptMsgClose(Some(msg));
            let _ = CertCloseStore(Some(store), 0);
        }
        name
    }

    /// Pull the signer certificate out of the message and read its display name.
    fn signer_name(store: HCERTSTORE, msg: *mut std::ffi::c_void) -> Option<String> {
        let mut size = 0u32;
        // SAFETY: asking for the size only; `pvdata` is null.
        unsafe {
            CryptMsgGetParam(msg, CMSG_SIGNER_INFO_PARAM, 0, None, &mut size).ok()?;
        }
        if size == 0 {
            return None;
        }

        let mut buffer = vec![0u8; size as usize];
        // SAFETY: the buffer really is `size` bytes long.
        unsafe {
            CryptMsgGetParam(
                msg,
                CMSG_SIGNER_INFO_PARAM,
                0,
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
            )
            .ok()?;
        }

        // SAFETY: the buffer was filled by the call above with a CMSG_SIGNER_INFO
        // followed by its variable-length parts, which stay inside `buffer`.
        let signer = unsafe { &*(buffer.as_ptr() as *const CMSG_SIGNER_INFO) };
        let mut info = CERT_INFO {
            Issuer: signer.Issuer,
            SerialNumber: signer.SerialNumber,
            ..Default::default()
        };

        // SAFETY: `info` borrows from `buffer`, which is still alive; the raw
        // pointer the call returns is null when nothing matched and is freed
        // below when it does not.
        let context = unsafe {
            CertFindCertificateInStore(
                store,
                CERT_QUERY_ENCODING_TYPE(X509_ASN_ENCODING.0 | PKCS_7_ASN_ENCODING.0),
                0,
                CERT_FIND_SUBJECT_CERT,
                Some((&mut info as *mut CERT_INFO).cast()),
                None,
            )
        };
        if context.is_null() {
            return None;
        }

        // SAFETY: `context` is a valid certificate context; the first call sizes
        // the name and the second fills the buffer. Freed on both exits.
        let name = unsafe {
            let needed = CertGetNameStringW(context, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None);
            if needed <= 1 {
                let _ = CertFreeCertificateContext(Some(context));
                return None;
            }
            let mut text = vec![0u16; needed as usize];
            CertGetNameStringW(
                context,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                None,
                Some(&mut text),
            );
            let _ = CertFreeCertificateContext(Some(context));
            String::from_utf16_lossy(&text)
                .trim_end_matches('\0')
                .to_owned()
        };

        (!name.is_empty()).then_some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_verdict_names_the_publisher() {
        let signature = Signature::Signed {
            publisher: Some("Microsoft Corporation".to_owned()),
        };
        assert!(signature.is_signed());
        assert_eq!(signature.publisher(), Some("Microsoft Corporation"));
        assert_eq!(signature.describe(), "signed by Microsoft Corporation");
    }

    /// The distinction module spec 05 relies on: "no signature" is a fact about
    /// the file, "could not check" is a fact about us, and they must not read the
    /// same in the UI.
    #[test]
    fn unknown_is_not_the_same_as_unsigned() {
        let unknown = Signature::Unknown {
            reason: "the file has gone".to_owned(),
        };
        assert!(!unknown.is_signed());
        assert!(!Signature::Unsigned.is_signed());
        assert_ne!(unknown.describe(), Signature::Unsigned.describe());
        assert!(Signature::Unsigned
            .describe()
            .contains("unsigned, unknown publisher"));
    }

    #[test]
    fn the_fake_answers_per_path_and_falls_back_to_its_default() {
        let probe = MapSignatures::new()
            .with(
                "C:\\Program Files\\Vendor\\app.exe",
                Signature::Signed {
                    publisher: Some("Vendor Ltd".to_owned()),
                },
            )
            .defaulting_to(Signature::Unsigned);

        assert_eq!(
            probe
                .check(Path::new("C:\\Program Files\\Vendor\\app.exe"))
                .publisher(),
            Some("Vendor Ltd")
        );
        assert_eq!(
            probe.check(Path::new("C:\\Users\\me\\thing.exe")),
            Signature::Unsigned
        );
    }

    #[test]
    fn a_probe_with_no_table_reports_unsigned() {
        assert_eq!(
            MapSignatures::new().check(Path::new("C:\\x.exe")),
            Signature::Unsigned
        );
    }

    /// The real probe must never panic or block on a path that is not there —
    /// a `Run` value can name a program that has been uninstalled.
    #[test]
    fn the_real_probe_reports_unknown_for_a_path_that_does_not_exist() {
        let verdict =
            SystemSignatureProbe.check(Path::new("C:\\WinKnife\\definitely\\not\\here\\ghost.exe"));
        assert!(
            matches!(verdict, Signature::Unknown { .. }),
            "got {verdict:?}"
        );
    }
}
