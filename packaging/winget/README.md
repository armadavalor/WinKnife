# winget manifest

Manifests for submitting WinKnife to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).
The folder layout under `manifests/` mirrors that repo, so a version folder can be copied over as is.

## New version

1. Copy the last version folder to `manifests/a/armadavalor/WinKnife/<version>/`.
2. Update `PackageVersion`, `ReleaseDate`, `InstallerUrl` and `InstallerSha256`
   (the setup.exe line of the release's `SHA256SUMS.txt`, upper case).
3. `winget validate --manifest packaging/winget/manifests/a/armadavalor/WinKnife/<version>`
4. Open a PR to microsoft/winget-pkgs with the folder (or `wingetcreate submit <folder>`).

Notes: the installer is Tauri's NSIS build, per-user (`Scope: user`); its uninstall
key is `HKCU\...\Uninstall\WinKnife` with publisher `winknife`, hence `ProductCode` and
`AppsAndFeaturesEntries`.
