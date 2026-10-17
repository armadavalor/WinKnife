<#
.SYNOPSIS
  Runs inside Windows Sandbox at logon: install WinKnife, dirty the profile,
  launch the app.

.DESCRIPTION
  Started by the LogonCommand in winknife.wsb. The sandbox account's profile
  is disposable, so the fixture goes straight into it; WinKnife then scans
  what a real dirty machine looks like. Log: Desktop\qa-bootstrap.log.
#>
[CmdletBinding()]
param(
  [int] $TempFiles = 100000,
  [string] $InstallerDir = 'C:\qa\installer',
  [string] $ScriptsDir = 'C:\qa\scripts'
)

$ErrorActionPreference = 'Stop'
$desktop = [Environment]::GetFolderPath('Desktop')
Start-Transcript -Path (Join-Path $desktop 'qa-bootstrap.log') -Append | Out-Null

try {
  if ($env:USERNAME -ne 'WDAGUtilityAccount') {
    throw "This script only runs inside Windows Sandbox (user is $env:USERNAME)."
  }

  $installer = Get-ChildItem -Path $InstallerDir -Filter '*-setup.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if (-not $installer) { throw "No *-setup.exe in $InstallerDir. Put a WinKnife installer there and restart the sandbox." }
  Write-Host "Installing $($installer.Name)"
  $p = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
  if ($p.ExitCode -ne 0) { throw "Installer exited with $($p.ExitCode)" }
  $exe = Join-Path $env:LOCALAPPDATA 'WinKnife\winknife.exe'
  if (-not (Test-Path $exe)) { throw "Installed app not found at $exe" }

  Write-Host "Dirtying profile $env:USERPROFILE ($TempFiles temp files)"
  & (Join-Path $ScriptsDir 'New-DirtyProfile.ps1') -Root $env:USERPROFILE -TempFiles $TempFiles -DisposableMachine

  Write-Host 'Launching WinKnife'
  Start-Process -FilePath $exe
  Write-Host 'Ready. Work through the "manual in Sandbox" cells of docs/QA-PLAN.md.'
} catch {
  Write-Host "BOOTSTRAP FAILED: $($_.Exception.Message)" -ForegroundColor Red
  Read-Host 'Press Enter to close'
} finally {
  Stop-Transcript | Out-Null
}
