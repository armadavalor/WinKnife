<#
.SYNOPSIS
  Opens Windows Sandbox with WinKnife installed on a freshly dirtied profile.

.DESCRIPTION
  Fills the host paths of winknife.wsb, writes the result to %TEMP% and
  opens it. Nothing on this PC is touched apart from that one .wsb file.

  Windows Sandbox must be enabled first (once, needs a reboot): Settings >
  Apps > Optional features > More Windows features > "Windows Sandbox".

.PARAMETER InstallerDir
  Folder holding a WinKnife_*_x64-setup.exe (a release download, the CI
  "winknife-installer" artifact, or src-tauri\target\release\bundle\nsis).

.PARAMETER TempFiles
  Size of the Temp fixture. 100000 reproduces the "huge Temp" scenario.

.EXAMPLE
  .\scripts\qa\Start-QASandbox.ps1 -InstallerDir $HOME\Downloads\winknife-installer
#>
[CmdletBinding()]
param(
  [string] $InstallerDir = (Join-Path $PSScriptRoot '..\..\src-tauri\target\release\bundle\nsis'),
  [int] $TempFiles = 100000
)

$ErrorActionPreference = 'Stop'

$sandboxExe = Join-Path $env:WINDIR 'System32\WindowsSandbox.exe'
if (-not (Test-Path $sandboxExe)) {
  throw 'Windows Sandbox is not enabled. Turn on the "Windows Sandbox" optional feature, reboot, and run this again.'
}
$InstallerDir = (Resolve-Path $InstallerDir).Path
if (-not (Get-ChildItem -Path $InstallerDir -Filter '*-setup.exe')) {
  throw "No *-setup.exe in $InstallerDir"
}

[xml] $wsb = Get-Content -Raw (Join-Path $PSScriptRoot 'winknife.wsb')
$folders = $wsb.Configuration.MappedFolders.MappedFolder
$folders[0].HostFolder = (Resolve-Path $PSScriptRoot).Path
$folders[1].HostFolder = $InstallerDir
$wsb.Configuration.LogonCommand.Command = $wsb.Configuration.LogonCommand.Command -replace '-TempFiles \d+', "-TempFiles $TempFiles"

$out = Join-Path $env:TEMP 'winknife-qa.wsb'
$wsb.Save($out)
Write-Host "Opening $out"
Start-Process -FilePath $out
