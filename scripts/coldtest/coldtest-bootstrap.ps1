<#
.SYNOPSIS
  Runs inside Windows Sandbox at logon: build the fixture profile, record
  its SHA-256 manifest, open Edge on the repository page.

.DESCRIPTION
  Started by the LogonCommand in cold-test.wsb. WinKnife is deliberately not
  installed: the participant finds, downloads and installs it. The fixture
  is a smaller New-DirtyProfile run plus a set of important-looking files
  that no WinKnife tool may ever offer. Every file the fixture creates is
  hashed into <results>\<Participant>-manifest.json so Test-ColdTestResult.ps1
  can prove afterwards what changed. Protocol: docs/launch/COLD-TEST.md.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)] [string] $Participant,
  [string] $QaDir = 'C:\coldtest\qa',
  [string] $ResultsDir = 'C:\coldtest\results',
  [string] $StartUrl = 'https://github.com/armadavalor/WinKnife'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
. (Join-Path $PSScriptRoot 'ColdTestLib.ps1')

$TEMP_FILES = 5000
$DUPLICATE_SETS = 30
$EMPTY_FOLDERS = 40
$KEEP_AGE_DAYS = 400
$FIXTURE_MANIFEST = 'winknife-fixture.json'

Start-Transcript -Path (Join-Path $ResultsDir "$Participant-bootstrap.log") | Out-Null

# Files a careful user would never want touched. Old on purpose: an
# age-based rule that leaked into personal folders would offer them.
function New-KeepFiles([string] $profileRoot) {
  $rng = New-Object System.Random 7
  $keep = @(
    'Documents\Tax return 2025.pdf',
    'Documents\Thesis\thesis-final.docx',
    'Documents\Thesis\sources.bib',
    'Desktop\Budget 2026.xlsx',
    'Pictures\Family 2024\IMG_0001.jpg',
    'Pictures\Family 2024\IMG_0002.jpg',
    'Pictures\Family 2024\IMG_0003.jpg',
    'Music\voice-memo-grandma.m4a'
  )
  foreach ($rel in $keep) {
    $path = Join-Path $profileRoot $rel
    [void](New-Item -ItemType Directory -Force -Path (Split-Path $path))
    $buf = New-Object byte[] ($rng.Next(20000, 400000))
    $rng.NextBytes($buf)
    [IO.File]::WriteAllBytes($path, $buf)
    $when = (Get-Date).AddDays(-$KEEP_AGE_DAYS)
    [IO.File]::SetCreationTime($path, $when)
    [IO.File]::SetLastWriteTime($path, $when)
    [IO.File]::SetLastAccessTime($path, $when)
  }
  $keep | ForEach-Object { Join-Path $profileRoot $_ }
}

# Windows 10's sandbox image has no working browser (the taskbar Edge pin is
# dead; Windows 11 ships Edge). Install it from Microsoft's online installer,
# refusing anything not validly signed by Microsoft. A missing browser must
# not fail the fixture: the operator can still type the URL.
$EDGE_SETUP_URL = 'https://go.microsoft.com/fwlink/?linkid=2109047&Channel=Stable&language=en&consent=1'
$EDGE_INSTALL_TIMEOUT_S = 300

function Get-EdgePath {
  @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
}

function Install-Edge {
  try {
    Write-Host 'Installing Microsoft Edge (not in this sandbox image)...'
    $setup = Join-Path $env:TEMP 'MicrosoftEdgeSetup.exe'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $EDGE_SETUP_URL -OutFile $setup -UseBasicParsing
    $sig = Get-AuthenticodeSignature $setup
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
      throw "Edge installer signature not trusted: $($sig.Status) $($sig.SignerCertificate.Subject)"
    }
    $proc = Start-Process -FilePath $setup -ArgumentList '/silent', '/install' -PassThru
    $deadline = (Get-Date).AddSeconds($EDGE_INSTALL_TIMEOUT_S)
    while (-not (Get-EdgePath) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 3 }
    if (-not $proc.HasExited) { Write-Host 'Edge installer still finishing in the background.' }
    return Get-EdgePath
  } catch {
    Write-Host "Edge install failed: $($_.Exception.Message)" -ForegroundColor Yellow
    return $null
  }
}

try {
  if ($env:USERNAME -ne 'WDAGUtilityAccount') {
    throw "This script only runs inside Windows Sandbox (user is $env:USERNAME)."
  }
  # Same normalization as New-DirtyProfile, so its sentinel paths match ours.
  $profileRoot = [IO.Path]::GetFullPath($env:USERPROFILE)
  $before = @{}
  foreach ($f in (Get-FileSet $profileRoot).Files) { $before[$f] = $true }

  & (Join-Path $QaDir 'New-DirtyProfile.ps1') -Root $profileRoot -TempFiles $TEMP_FILES `
    -DuplicateSets $DUPLICATE_SETS -EmptyFolders $EMPTY_FOLDERS -DisposableMachine
  $fixture = Get-Content -Raw -Encoding UTF8 (Join-Path $profileRoot $FIXTURE_MANIFEST) | ConvertFrom-Json
  $keepPaths = @(New-KeepFiles $profileRoot)
  # Sentinels New-DirtyProfile already defines as "must not be offered".
  $keepPaths += $fixture.sentinels.tempFresh, $fixture.sentinels.downloadsRecent
  $keepPaths += Join-Path $fixture.sentinels.emptyKeep 'main.rs'
  $keepSet = @{}
  foreach ($k in $keepPaths) { $keepSet[$k] = $true }

  $after = Get-FileSet $profileRoot
  $entries = New-Object System.Collections.Generic.List[object]
  foreach ($f in $after.Files) {
    if ($before.ContainsKey($f) -or $f.EndsWith($FIXTURE_MANIFEST)) { continue }
    $entries.Add([ordered]@{
      path = $f
      sha256 = Get-Sha256OrNull $f
      class = $(if ($keepSet.ContainsKey($f)) { 'keep' } else { 'offerable' })
    })
  }

  $manifest = [ordered]@{
    participant = $Participant
    createdAt = (Get-Date).ToString('o')
    profileRoot = $profileRoot
    unlistableFolders = $after.Unlistable
    files = $entries
  }
  $manifestPath = Join-Path $ResultsDir "$Participant-manifest.json"
  [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), (New-Object System.Text.UTF8Encoding $false))
  $keepCount = @($entries | Where-Object { $_.class -eq 'keep' }).Count
  Write-Host ("Manifest: {0} files ({1} keep). {2}" -f $entries.Count, $keepCount, $manifestPath)

  $edge = Get-EdgePath
  if (-not $edge) { $edge = Install-Edge }
  if ($edge) {
    Start-Process -FilePath $edge -ArgumentList $StartUrl
  } else {
    Write-Host "No browser. Open $StartUrl by hand." -ForegroundColor Yellow
  }
  Write-Host 'Ready. Hand over to the participant.'
} catch {
  Write-Host "BOOTSTRAP FAILED: $($_.Exception.Message)" -ForegroundColor Red
  Read-Host 'Press Enter to close'
} finally {
  Stop-Transcript | Out-Null
}
