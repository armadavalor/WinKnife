fn main() {
    embed_common_controls_manifest_in_test_binaries();
    tauri_build::build()
}

/// Give `cargo test` binaries the same ComCtl32 v6 manifest `winknife.exe` gets.
///
/// Tauri's default features pull in `muda`, which imports `SetWindowSubclass`,
/// `RemoveWindowSubclass`, `DefSubclassProc` and `TaskDialogIndirect`. Those are
/// exported **only** by ComCtl32 version 6, and Windows only hands version 6 to a
/// process whose application manifest asks for it by name. `tauri_build::build()`
/// puts that manifest in the app binary; nothing puts one in a test binary, so any
/// test target that links Tauri's runtime — `tests/command_wiring.rs`, which has to
/// — died at *load* time with `STATUS_ENTRYPOINT_NOT_FOUND` (0xC0000139) before a
/// single test function ran. The failure looks like a broken test and is really a
/// missing side-by-side assembly.
///
/// MSVC-only: `/MANIFEST:EMBED` is a link.exe flag and the GNU toolchain would
/// choke on it.
fn embed_common_controls_manifest_in_test_binaries() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !windows || !msvc {
        return;
    }

    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let manifest = std::path::Path::new(&out_dir).join("test-common-controls.manifest");
    if std::fs::write(&manifest, COMMON_CONTROLS_V6_MANIFEST).is_err() {
        // Not fatal: the app itself builds fine without this, and a test binary
        // that cannot start will say so loudly enough.
        return;
    }

    println!("cargo::rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo::rustc-link-arg-tests=/MANIFESTINPUT:{}",
        manifest.display()
    );
}

/// The one dependency that matters: version 6 of the common controls assembly.
const COMMON_CONTROLS_V6_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#;
