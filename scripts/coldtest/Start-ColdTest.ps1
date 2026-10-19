<#
.SYNOPSIS
  Opens a fresh Windows Sandbox for one cold-test participant.

.DESCRIPTION
  Fills the host paths of cold-test.wsb, writes the result to %TEMP% and
  opens it. Inside the sandbox the bootstrap builds the fixture profile,
  records a SHA-256 manifest and opens Edge on the repository page. WinKnife
  is NOT preinstalled: finding, downloading and installing it is part of the
  test. Nothing on this PC is touched apart from the .wsb file and the
  results folder. Protocol: docs/launch/COLD-TEST.md.

.PARAMETER Participant
  Participant code, P1 to P5. Used for the results file names; never a name.

.PARAMETER ResultsDir
  Host folder that receives the manifest and verification report.

.EXAMPLE
  .\scripts\coldtest\Start-ColdTest.ps1 -Participant P1
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)] [ValidatePattern('^P[1-9]$')] [string] $Participant,
  [string] $ResultsDir = (Join-Path $env:USERPROFILE 'WinKnife-coldtest-results')
)

$ErrorActionPreference = 'Stop'

$sandboxExe = Join-Path $env:WINDIR 'System32\WindowsSandbox.exe'
if (-not (Test-Path $sandboxExe)) {
  throw 'Windows Sandbox is not enabled. Turn on the "Windows Sandbox" optional feature, reboot, and run this again.'
}
if (Get-Process -Name 'WindowsSandbox*' -ErrorAction SilentlyContinue) {
  throw 'A Windows Sandbox is already running. Close it first: only one can run at a time.'
}
[void](New-Item -ItemType Directory -Force -Path $ResultsDir)

[xml] $wsb = Get-Content -Raw (Join-Path $PSScriptRoot 'cold-test.wsb')
$folders = $wsb.Configuration.MappedFolders.MappedFolder
$folders[0].HostFolder = (Resolve-Path (Join-Path $PSScriptRoot '..\qa')).Path
$folders[1].HostFolder = (Resolve-Path $PSScriptRoot).Path
$folders[2].HostFolder = (Resolve-Path $ResultsDir).Path
$wsb.Configuration.LogonCommand.Command = $wsb.Configuration.LogonCommand.Command -replace '-Participant P\d', "-Participant $Participant"

$out = Join-Path $env:TEMP "winknife-coldtest-$Participant.wsb"
$wsb.Save($out)
Write-Host "Opening $out. Results go to $ResultsDir"
Start-Process -FilePath $out
