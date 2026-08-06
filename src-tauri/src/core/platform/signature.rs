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
//! # Two ways a Windows binary is signed, and why both are asked
//!
//! An Authenticode signature can be **embedded** in the PE, or it can live in a
//! **catalog** — a `.cat` file that signs a table of file hashes. Almost
//! everything Windows ships is catalog-signed and carries no embedded
//! signature at all. Asking only `WTD_CHOICE_FILE` therefore returned
//! `TRUST_E_NOSIGNATURE` for `notepad.exe`, which this module then reported as
//! `Unsigned` — "unsigned, unknown publisher", confidence 80 down to 55, and
//! `KnownApps::lookup` unable to match its publisher-keyed rules. This file is
//! otherwise scrupulous about the difference between "no" and "don't know", and
//! here the two collapsed in the direction that nudges a user to switch off a
//! Windows component (security review M18). So a file with no embedded signature
//! is hashed and looked up in the catalog database before anything is called
//! unsigned.
//!
//! # Read-only, including last-access times
//!
//! `WinVerifyTrust` will open the file itself if it is handed `NULL`, with none
//! of the suppression the lock probe applies — and a control measured an
//! access-time change attributable to the signature check even on a machine with
//! `DisableLastAccess = 2` (security review M19). So the file is opened here,
//! with the same `0xFFFFFFFF` `SetFileTime` opt-out `lock.rs` uses, and the
//! handle is handed to `WINTRUST_FILE_INFO.hFile`. The catalog path needs a
//! handle anyway, to hash the file with.

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
    use windows::Win32::Security::Cryptography::Catalog::{
        CryptCATAdminAcquireContext2, CryptCATAdminCalcHashFromFileHandle2,
        CryptCATAdminEnumCatalogFromHash, CryptCATAdminReleaseCatalogContext,
        CryptCATAdminReleaseContext, CryptCATCatalogInfoFromContext, CATALOG_INFO,
    };
    use windows::Win32::Security::Cryptography::CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED;
    use windows::Win32::Security::Cryptography::{
        CertCloseStore, CertFindCertificateInStore, CertFreeCertificateContext, CertGetNameStringW,
        CryptMsgClose, CryptMsgGetParam, CryptQueryObject, CERT_FIND_SUBJECT_CERT, CERT_INFO,
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
        CERT_QUERY_CONTENT_TYPE_FLAGS, CERT_QUERY_ENCODING_TYPE, CERT_QUERY_FORMAT_FLAG_BINARY,
        CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM, HCERTSTORE,
        PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_CATALOG_INFO, WINTRUST_DATA,
        WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_CATALOG, WTD_CHOICE_FILE, WTD_REVOKE_NONE,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn wide_str(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Open the binary for reading without letting the read move its last-access
    /// time.
    ///
    /// Two attempts, exactly as `lock.rs::open_leaving_the_clock_alone` does, and
    /// the extra right in the first is the whole point: **`SetFileTime` needs
    /// `FILE_WRITE_ATTRIBUTES`**, so asking for `FILE_GENERIC_READ` alone gets
    /// the opt-out rejected with `ACCESS_DENIED` and the clock moves anyway.
    /// Measured: with read-only access the opt-out was refused and reading the
    /// file moved the clock; with the right added, every stage of the check —
    /// the open, the read, `WTD_CHOICE_FILE`, the catalog hash and
    /// `WTD_CHOICE_CATALOG` — left it exactly where it was.
    ///
    /// Shared read/write/delete, unlike the lock probe: this must never be the
    /// reason another process cannot open a file it owns, and a signature check
    /// is not a lock probe.
    fn open_for_verify(path: &Path) -> std::io::Result<std::fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FILE_WRITE_ATTRIBUTES,
        };

        const SHARED: u32 = FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0;
        let open = |access: u32| {
            std::fs::OpenOptions::new()
                .access_mode(access)
                .share_mode(SHARED)
                .open(path)
        };

        if let Ok(file) = open(FILE_GENERIC_READ.0 | FILE_WRITE_ATTRIBUTES.0) {
            if !super::super::lock::suppress_access_time_updates(&file) {
                tracing::debug!(
                    path = %path.display(),
                    "this volume would not suppress access-time updates for a signature check"
                );
            }
            return Ok(file);
        }

        // An ACL that grants read but not write-attributes is unusual and
        // entirely legal. Losing the suppression is the smaller harm: refusing to
        // check the signature would report "unknown publisher" about a file we
        // could perfectly well have verified.
        tracing::debug!(
            path = %path.display(),
            "no write-attributes right, so the access-time opt-out is unavailable here"
        );
        open(FILE_GENERIC_READ.0)
    }

    /// Verify the signature — embedded first, then catalog — and read the
    /// signer's name.
    pub(super) fn check(path: &Path) -> Signature {
        if !path.is_file() {
            return Signature::Unknown {
                reason: format!("`{}` is not a file on this machine", path.display()),
            };
        }

        let file = match open_for_verify(path) {
            Ok(file) => file,
            Err(err) => {
                return Signature::Unknown {
                    reason: format!("`{}` could not be opened: {err}", path.display()),
                }
            }
        };
        let handle = file_handle(&file);

        match verify_embedded(path, handle) {
            Ok(true) => {
                return Signature::Signed {
                    publisher: publisher_of(path, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED),
                }
            }
            Err(reason) => return Signature::Unknown { reason },
            // No embedded signature. Almost everything Windows ships is in this
            // state and is signed by a catalog, so the question is not answered
            // yet (security review M18).
            Ok(false) => {}
        }

        match verify_catalog(path, handle) {
            Ok(Some(catalog)) => Signature::Signed {
                publisher: publisher_of(&catalog, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED),
            },
            // Asked both questions, got "no" to both: this really is unsigned.
            Ok(None) => Signature::Unsigned,
            // The catalog database could not be consulted, so "unsigned" would
            // be a claim we have not earned.
            Err(reason) => Signature::Unknown {
                reason: format!("no embedded signature; may be catalog-signed, but {reason}"),
            },
        }
    }

    fn file_handle(file: &std::fs::File) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(file.as_raw_handle())
    }

    /// The code path as it was before M19: no handle of ours, so `WinVerifyTrust`
    /// opens the file itself with no access-time suppression on it.
    ///
    /// Exists only as the control the read-only test measures against — an
    /// assertion that the clock did not move is worth nothing without evidence
    /// that on this machine, on this run, something could have moved it.
    #[cfg(test)]
    pub(super) fn verify_the_old_way(path: &Path) -> i32 {
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
        verify_with(&mut data)
    }

    /// `WinVerifyTrust` against the generic Authenticode policy, for a signature
    /// embedded in the file itself.
    ///
    /// `Ok(false)` means "no embedded signature"; an error means "could not
    /// decide", and the two are kept apart deliberately (an expired certificate
    /// or an unreachable revocation list is not the same as an unsigned binary).
    fn verify_embedded(path: &Path, handle: HANDLE) -> std::result::Result<bool, String> {
        let file = wide(path);
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(file.as_ptr()),
            // Our handle, not one Windows opens for itself: the one we opened has
            // access-time updates suppressed (security review M19).
            hFile: handle,
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

        match verify_with(&mut data) {
            0 => Ok(true),
            code if code == TRUST_E_NOSIGNATURE.0 => Ok(false),
            code => Err(format!("Windows reported 0x{:08X}", code as u32)),
        }
    }

    /// Hash the file, find the catalog that vouches for it, and verify that.
    ///
    /// `Ok(None)` means the hash is in no catalog on this machine — which,
    /// together with `verify_embedded` having said no, is what "unsigned"
    /// actually means.
    fn verify_catalog(
        path: &Path,
        handle: HANDLE,
    ) -> std::result::Result<Option<std::path::PathBuf>, String> {
        let admin = CatalogAdmin::acquire()?;
        let hash = admin.hash_of(handle)?;
        let Some(catalog) = admin.catalog_for(&hash) else {
            return Ok(None);
        };

        let file = wide(path);
        let catalog_wide = wide(&catalog);
        // The member tag is the file hash as upper-case hex; that is the name
        // the catalog files its members under.
        let mut tag = wide_str(&hex(&hash));
        let mut hash = hash;

        let mut info = WINTRUST_CATALOG_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_CATALOG_INFO>() as u32,
            pcwszCatalogFilePath: PCWSTR(catalog_wide.as_ptr()),
            pcwszMemberTag: PCWSTR(tag.as_mut_ptr()),
            pcwszMemberFilePath: PCWSTR(file.as_ptr()),
            hMemberFile: handle,
            pbCalculatedFileHash: hash.as_mut_ptr(),
            cbCalculatedFileHash: hash.len() as u32,
            hCatAdmin: admin.0,
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_CATALOG,
            Anonymous: WINTRUST_DATA_0 {
                pCatalog: &mut info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };

        match verify_with(&mut data) {
            0 => Ok(Some(catalog)),
            code if code == TRUST_E_NOSIGNATURE.0 => Ok(None),
            code => Err(format!(
                "verifying it against `{}` reported 0x{:08X}",
                catalog.display(),
                code as u32
            )),
        }
    }

    /// Run one verify/close pair and hand back the verify's status.
    fn verify_with(data: &mut WINTRUST_DATA) -> i32 {
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

        // SAFETY: `data` and `action` are live for the whole call, and whatever
        // the union points at outlives both — the caller owns it. `WTD_UI_NONE`
        // guarantees no window is shown, which matters in a service-like process.
        let status = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (data as *mut WINTRUST_DATA).cast(),
            )
        };

        // The state handle has to be released whatever the verdict was.
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: same arguments, closing the state opened above.
        let _ = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (data as *mut WINTRUST_DATA).cast(),
            )
        };
        status
    }

    /// An owned `HCATADMIN`, released on drop.
    struct CatalogAdmin(isize);

    impl CatalogAdmin {
        fn acquire() -> std::result::Result<Self, String> {
            let mut handle = 0isize;
            let algorithm = wide_str("SHA256");
            // SAFETY: `handle` is a valid out pointer and `algorithm` is a
            // NUL-terminated buffer that outlives the call.
            unsafe {
                CryptCATAdminAcquireContext2(
                    &mut handle,
                    None,
                    PCWSTR(algorithm.as_ptr()),
                    None,
                    None,
                )
            }
            .map_err(|err| format!("the catalog database could not be opened: {err}"))?;
            Ok(Self(handle))
        }

        fn hash_of(&self, handle: HANDLE) -> std::result::Result<Vec<u8>, String> {
            let mut size = 0u32;
            // SAFETY: asking for the size only; the buffer pointer is `None`.
            let _ = unsafe {
                CryptCATAdminCalcHashFromFileHandle2(self.0, handle, &mut size, None, None)
            };
            if size == 0 {
                return Err("the file's catalog hash could not be sized".to_owned());
            }

            let mut hash = vec![0u8; size as usize];
            // SAFETY: the buffer really is `size` bytes long.
            unsafe {
                CryptCATAdminCalcHashFromFileHandle2(
                    self.0,
                    handle,
                    &mut size,
                    Some(hash.as_mut_ptr()),
                    None,
                )
            }
            .map_err(|err| format!("the file's catalog hash could not be computed: {err}"))?;
            hash.truncate(size as usize);
            Ok(hash)
        }

        /// The first catalog on this machine that vouches for `hash`.
        fn catalog_for(&self, hash: &[u8]) -> Option<std::path::PathBuf> {
            // SAFETY: `hash` is a live slice for the whole call.
            let context = unsafe { CryptCATAdminEnumCatalogFromHash(self.0, hash, None, None) };
            if context == 0 {
                return None;
            }

            let mut info = CATALOG_INFO {
                cbStruct: std::mem::size_of::<CATALOG_INFO>() as u32,
                ..Default::default()
            };
            // SAFETY: `context` came from the enumeration above and `info` is a
            // valid, correctly sized out parameter.
            let read = unsafe { CryptCATCatalogInfoFromContext(context, &mut info, 0) };
            // SAFETY: releasing the context the enumeration handed out.
            unsafe {
                let _ = CryptCATAdminReleaseCatalogContext(self.0, context, 0);
            }
            read.ok()?;

            let end = info
                .wszCatalogFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(info.wszCatalogFile.len());
            let path = String::from_utf16_lossy(&info.wszCatalogFile[..end]);
            (!path.is_empty()).then(|| std::path::PathBuf::from(path))
        }
    }

    impl Drop for CatalogAdmin {
        fn drop(&mut self) {
            // SAFETY: the handle came from `CryptCATAdminAcquireContext2` and is
            // released exactly once.
            unsafe {
                let _ = CryptCATAdminReleaseContext(self.0, 0);
            }
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02X}");
            out
        })
    }

    /// Subject name of the signing certificate, or `None` when it cannot be read.
    ///
    /// Only called after a verify said the signature is good, so a failure here
    /// downgrades the display ("signed, publisher unreadable") rather than the
    /// verdict. `source` is the binary itself for an embedded signature and the
    /// `.cat` file for a catalog one — a catalog *is* a PKCS#7 message, so the
    /// content flag differs and nothing else does.
    fn publisher_of(source: &Path, content: CERT_QUERY_CONTENT_TYPE_FLAGS) -> Option<String> {
        let file = wide(source);
        let mut store = HCERTSTORE::default();
        let mut msg: *mut std::ffi::c_void = std::ptr::null_mut();

        // SAFETY: `file` is NUL-terminated and outlives the call; the two out
        // handles are valid pointers and are released below.
        let queried = unsafe {
            CryptQueryObject(
                CERT_QUERY_OBJECT_FILE,
                file.as_ptr().cast(),
                content,
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

    /// A catalog-signed Windows binary must not read as unsigned
    /// (security review M18).
    ///
    /// `notepad.exe` carries no embedded signature at all, so the old probe
    /// returned `TRUST_E_NOSIGNATURE` and this module reported "unsigned,
    /// unknown publisher" about a file Microsoft signed — which drops the
    /// startup manager's confidence and stops `KnownApps::lookup` matching its
    /// publisher-keyed rules.
    #[cfg(windows)]
    #[test]
    fn a_catalog_signed_windows_binary_is_not_reported_as_unsigned() {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if !notepad.is_file() {
            return;
        }

        let verdict = SystemSignatureProbe.check(notepad);
        println!("notepad.exe -> {verdict:?}");
        println!(
            "notepad.exe -> signed={}, {}",
            verdict.is_signed(),
            verdict.describe()
        );
        assert_ne!(
            verdict,
            Signature::Unsigned,
            "a catalog-signed binary is not an unsigned one"
        );
        assert!(
            verdict.is_signed(),
            "Windows' own binaries are catalog-signed and must verify: {verdict:?}"
        );
    }

    /// Nothing tampered with may ever get the badge. The catalog fallback added
    /// a second way to say "signed", and this is the guard that it did not
    /// become a way to say it about the wrong file.
    #[cfg(windows)]
    #[test]
    fn a_tampered_copy_of_a_signed_binary_never_verifies() {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if !notepad.is_file() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");

        let copy = dir.path().join("notepad.exe");
        let Ok(mut bytes) = std::fs::read(notepad) else {
            return;
        };
        // Flip a byte in the middle of the image; the hash no longer matches any
        // catalog entry and no embedded signature covers it either.
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xFF;
        std::fs::write(&copy, &bytes).expect("write");

        let verdict = SystemSignatureProbe.check(&copy);
        println!("tampered copy -> {verdict:?}");
        assert!(!verdict.is_signed(), "got {verdict:?}");

        // And a file that is not a PE at all.
        let nonsense = dir.path().join("nonsense.exe");
        std::fs::write(&nonsense, b"this is not a program").expect("write");
        assert!(!SystemSignatureProbe.check(&nonsense).is_signed());
    }

    /// The read-only promise, including last-access times (security review M19).
    ///
    /// Measured against a control that provably *can* move the clock on this
    /// machine, on this run: the old code path — `WinVerifyTrust` with a null
    /// `hFile`, so it opens the binary itself with no suppression — runs first
    /// against a backdated file, and the test says out loud whether that moved
    /// anything. A "the clock did not move" assertion with no such control is
    /// worth nothing on a volume where nothing moves clocks anyway.
    ///
    /// The subject is a byte-identical copy of a real signed binary, because the
    /// whole verification has to run: a text file fails at
    /// `TRUST_E_SUBJECT_FORM_UNKNOWN` before most of the file is ever touched.
    #[cfg(windows)]
    #[test]
    fn a_signature_check_does_not_move_the_access_time() {
        use std::time::{Duration, SystemTime};

        let source = Path::new(r"C:\Windows\System32\notepad.exe");
        if !source.is_file() {
            println!("skipped: no signed binary to copy");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("subject.exe");
        if std::fs::copy(source, &file).is_err() {
            println!("skipped: could not copy the subject");
            return;
        }

        let accessed = |path: &Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .accessed()
                .expect("an access time")
        };
        // A month ago: any update at all is unmissable.
        let backdated = SystemTime::now() - Duration::from_secs(60 * 60 * 24 * 30);
        let backdate = |path: &Path| {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("open for set_times")
                .set_times(std::fs::FileTimes::new().set_accessed(backdated))
                .expect("backdate the access time");
        };

        backdate(&file);
        let before_control = accessed(&file);
        let status = sys::verify_the_old_way(&file);
        let after_control = accessed(&file);
        let control_moved = after_control != before_control;
        println!(
            "control (null hFile, the old path) reported 0x{:08X}; clock {}",
            status as u32,
            if control_moved {
                "MOVED — this test is load-bearing on this machine"
            } else {
                "held — this volume does not update access times either way"
            }
        );
        println!("  backdated to      {before_control:?}");
        println!("  after control     {after_control:?}");

        backdate(&file);
        let before = accessed(&file);
        let verdict = SystemSignatureProbe.check(&file);
        let after = accessed(&file);
        println!("  verdict           {verdict:?}");
        println!("  after the probe   {after:?}");

        assert_eq!(
            before, after,
            "a signature check moved the last-access time, so a scan is not read-only"
        );
    }
}
