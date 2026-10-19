<#
.SYNOPSIS
  Run inside the sandbox at the end of a session: compare the profile with
  the manifest written by the bootstrap.

.DESCRIPTION
  For every file the fixture created:
    unchanged   still at its path with the same SHA-256 (also what a correct
                restore looks like)
    missing     gone from its path (quarantined, or deleted)
    changed     present with different content
    unreadable  could not be hashed now or at setup (locked/denied)

  FAIL when any "keep" file is missing or changed, or any file is changed.
  Missing "offerable" files are listed for the moderator to match against
  the recording: each one must correspond to something the participant
  approved. Writes <Participant>-verify.json and -verify.txt next to the
  manifest and exits 1 on FAIL.

.EXAMPLE
  C:\coldtest\scripts\Test-ColdTestResult.ps1 -Participant P1
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)] [string] $Participant,
  [string] $ResultsDir = 'C:\coldtest\results'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'ColdTestLib.ps1')

function Get-State($entry) {
  if (-not (Test-FileExists $entry.path)) { return 'missing' }
  $now = Get-Sha256OrNull $entry.path
  if ($null -eq $now -or $null -eq $entry.sha256) { return 'unreadable' }
  if ($now -eq $entry.sha256) { return 'unchanged' }
  'changed'
}

$manifest = Get-Content -Raw -Encoding UTF8 (Join-Path $ResultsDir "$Participant-manifest.json") | ConvertFrom-Json
$rows = foreach ($e in $manifest.files) {
  [pscustomobject]@{ path = $e.path; class = $e.class; state = (Get-State $e) }
}

$keepBad = @($rows | Where-Object { $_.class -eq 'keep' -and $_.state -in 'missing', 'changed' })
$changed = @($rows | Where-Object { $_.state -eq 'changed' })
$missingOfferable = @($rows | Where-Object { $_.class -eq 'offerable' -and $_.state -eq 'missing' })
$verdict = if ($keepBad.Count -or $changed.Count) { 'FAIL' } else { 'PASS' }

$summary = [ordered]@{
  participant = $Participant
  checkedAt = (Get-Date).ToString('o')
  verdict = $verdict
  counts = [ordered]@{
    total = @($rows).Count
    unchanged = @($rows | Where-Object state -eq 'unchanged').Count
    missing = @($rows | Where-Object state -eq 'missing').Count
    changed = $changed.Count
    unreadable = @($rows | Where-Object state -eq 'unreadable').Count
  }
  keepViolations = $keepBad
  changedFiles = $changed
  missingOfferable = $missingOfferable
}
$base = Join-Path $ResultsDir "$Participant-verify"
[IO.File]::WriteAllText("$base.json", ($summary | ConvertTo-Json -Depth 4), (New-Object System.Text.UTF8Encoding $false))

$lines = @(
  "$Participant  verdict: $verdict",
  ("files {0}: unchanged {1}, missing {2}, changed {3}, unreadable {4}" -f $summary.counts.total, $summary.counts.unchanged, $summary.counts.missing, $summary.counts.changed, $summary.counts.unreadable),
  '',
  "Keep files missing or changed ($($keepBad.Count)):"
) + ($keepBad | ForEach-Object { "  [$($_.state)] $($_.path)" }) + @(
  '',
  "Changed content ($($changed.Count)):"
) + ($changed | ForEach-Object { "  $($_.path)" }) + @(
  '',
  "Missing offerable files: match each against an approval in the recording ($($missingOfferable.Count)):"
) + ($missingOfferable | Group-Object { Split-Path $_.path } | Sort-Object Count -Descending |
  ForEach-Object { "  {0,6}  {1}" -f $_.Count, $_.Name })
[IO.File]::WriteAllLines("$base.txt", $lines, (New-Object System.Text.UTF8Encoding $false))
$lines | ForEach-Object { Write-Host $_ }
if ($verdict -eq 'FAIL') { exit 1 }
