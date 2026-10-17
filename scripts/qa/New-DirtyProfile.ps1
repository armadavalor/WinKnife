<#
.SYNOPSIS
  Fills a throwaway Windows profile with realistic junk for WinKnife QA.

.DESCRIPTION
  Builds the "dirty machine" fixture described in docs/QA-PLAN.md under
  -Root (a profile root, the folder that plays %USERPROFILE%):

    AppData\Local\Temp        -TempFiles small files (default 100 000) in
                              nested folders, aged 2-60 days, plus a few
                              large installers and crash dumps
    AppData\Local\...\Cache   Edge, Chrome and Firefox cache folders
    AppData\Local\CrashDumps  old .dmp files
    Downloads                 old installers/archives (needs review) and
                              recent files that must not be offered
    Documents, Pictures       -DuplicateSets sets of identical files, with
                              "(1)" / "- Copy" twins in Downloads
    Documents\Old projects    -EmptyFolders empty folder trees
    Temp\winknife-qa-locked   a file held open by a helper process
    Temp\winknife-qa-denied   a folder the current user is denied
    Temp\winknife-qa-long     a path well past 260 characters
    Temp\<unicode> ...        Turkish, CJK and emoji names
    Temp\winknife-qa-links    a junction loop back into Temp, a junction to
                              C:\Windows, and a file symlink when allowed

  It writes <Root>\winknife-fixture.json listing the sentinel paths and the
  helper PID; the E2E suite (scripts/qa/e2e.mjs) reads it.

  This only ever writes under -Root. It refuses the real profile of the
  account running it unless -DisposableMachine is passed, which only the
  Windows Sandbox bootstrap and CI do. Runs on Windows PowerShell 5.1
  (Windows Sandbox has no pwsh) and on PowerShell 7.

.PARAMETER Root
  Profile root to fill. Created if missing.

.PARAMETER TempFiles
  Number of small files under Temp. 100000 is the "huge" scenario.

.PARAMETER DuplicateSets
  Number of duplicate sets (each 2-4 copies).

.PARAMETER EmptyFolders
  Number of empty leaf folders (in nested trees).

.PARAMETER NoLockHelper
  Do not start the process that holds a file open.

.PARAMETER DisposableMachine
  Allow -Root to be the current account's real profile. Only for Windows
  Sandbox and CI runners, never a personal PC.

.EXAMPLE
  .\New-DirtyProfile.ps1 -Root C:\QA\Profile -TempFiles 5000
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)] [string] $Root,
  [int] $TempFiles = 100000,
  [int] $DuplicateSets = 200,
  [int] $EmptyFolders = 500,
  [switch] $NoLockHelper,
  [switch] $DisposableMachine
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$FILES_PER_DIR = 250
$SMALL_FILE_MAX_BYTES = 4096
$LONG_PATH_TARGET_CHARS = 400
$MANIFEST_NAME = 'winknife-fixture.json'

$Root = [IO.Path]::GetFullPath($Root)
$realProfile = [IO.Path]::GetFullPath([Environment]::GetFolderPath('UserProfile'))
if (($Root.TrimEnd('\') -ieq $realProfile.TrimEnd('\')) -and -not $DisposableMachine) {
  throw "Refusing to fill the real profile $realProfile. Use a separate -Root, or pass -DisposableMachine inside Windows Sandbox / CI only."
}
if ($Root.Length -le 3) { throw "Refusing a drive root as -Root: $Root" }

$rng = New-Object System.Random 4242
$now = Get-Date
$stats = @{ files = 0; bytes = [int64]0; dirs = 0 }

function Join([string[]] $parts) { [IO.Path]::Combine($parts) }

function Ensure-Dir([string] $path) {
  if (-not [IO.Directory]::Exists($path)) {
    [void][IO.Directory]::CreateDirectory($path)
    $stats.dirs++
  }
  $path
}

function Set-Age([string] $path, [double] $days) {
  $when = $now.AddDays(-$days)
  [IO.File]::SetCreationTime($path, $when)
  [IO.File]::SetLastWriteTime($path, $when)
  [IO.File]::SetLastAccessTime($path, $when)
}

# Random content so duplicate detection cannot pair unrelated files.
function New-Junk([string] $path, [int] $bytes, [double] $days) {
  $buf = New-Object byte[] $bytes
  $rng.NextBytes($buf)
  [IO.File]::WriteAllBytes($path, $buf)
  Set-Age $path $days
  $stats.files++
  $stats.bytes += $bytes
}

# Large zero-filled files are instant with fsutil and cost no RAM.
function New-Big([string] $path, [int64] $mb, [double] $days) {
  & fsutil file createnew $path ($mb * 1MB) | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "fsutil failed for $path" }
  Set-Age $path $days
  $stats.files++
  $stats.bytes += $mb * 1MB
}

function Write-Step([string] $text) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $text) }

$local = Ensure-Dir (Join @($Root, 'AppData', 'Local'))
[void](Ensure-Dir (Join @($Root, 'AppData', 'Roaming')))
$temp = Ensure-Dir (Join @($local, 'Temp'))
$downloads = Ensure-Dir (Join @($Root, 'Downloads'))
$documents = Ensure-Dir (Join @($Root, 'Documents'))
$pictures = Ensure-Dir (Join @($Root, 'Pictures'))
foreach ($name in 'Desktop', 'Music', 'Videos') { [void](Ensure-Dir (Join @($Root, $name))) }

# --- Temp at scale -----------------------------------------------------------
Write-Step "Temp: $TempFiles small files"
$extensions = '.tmp', '.log', '.dat', '.etl', '.chk', '.bak', '.cache', '.txt'
$dirCount = [Math]::Max(1, [Math]::Ceiling($TempFiles / $FILES_PER_DIR))
$written = 0
for ($d = 0; $d -lt $dirCount -and $written -lt $TempFiles; $d++) {
  # Two levels deep, like installers and updaters leave behind.
  $dir = Ensure-Dir (Join @($temp, ('{0:X8}-qa' -f (0x1A000 + [int]($d / 20))), ('tmp{0:x4}' -f $d)))
  for ($f = 0; $f -lt $FILES_PER_DIR -and $written -lt $TempFiles; $f++) {
    $name = 'wk{0:x6}{1}' -f $written, $extensions[$written % $extensions.Length]
    New-Junk (Join @($dir, $name)) ($rng.Next(0, $SMALL_FILE_MAX_BYTES)) (2 + $rng.Next(0, 58))
    $written++
  }
  if ($d % 40 -eq 0) { Write-Progress -Activity 'Temp files' -Status "$written / $TempFiles" -PercentComplete ([int](100 * $written / [Math]::Max(1, $TempFiles))) }
}
Write-Progress -Activity 'Temp files' -Completed
$tempSentinel = Join @($temp, 'winknife-qa-sentinel.tmp')
New-Junk $tempSentinel 2048 10
# A fresh file the cleaner must leave alone (younger than 24 hours).
$tempFresh = Join @($temp, 'winknife-qa-fresh.tmp')
New-Junk $tempFresh 1024 0.1
New-Big (Join @($temp, 'setup-7.2.1.msi')) 40 45
New-Big (Join @((Ensure-Dir (Join @($temp, '7zS4E21'))), 'payload.dat')) 30 16
New-Big (Join @($temp, 'explorer.exe.4312.dmp')) 20 9

# --- Browser and app caches ---------------------------------------------------
Write-Step 'Browser and app caches'
$caches = @(
  (Join @($local, 'Microsoft', 'Edge', 'User Data', 'Default', 'Cache', 'Cache_Data')),
  (Join @($local, 'Microsoft', 'Edge', 'User Data', 'Default', 'Code Cache', 'js')),
  (Join @($local, 'Google', 'Chrome', 'User Data', 'Default', 'Cache', 'Cache_Data')),
  (Join @($local, 'Google', 'Chrome', 'User Data', 'Profile 1', 'GPUCache')),
  (Join @($local, 'Mozilla', 'Firefox', 'Profiles', 'qa1234.default-release', 'cache2', 'entries')),
  (Join @($local, 'npm-cache', '_cacache', 'content-v2', 'sha512', 'ab')),
  (Join @($local, 'pip', 'Cache', 'http', '1', '2'))
)
foreach ($c in $caches) {
  [void](Ensure-Dir $c)
  for ($i = 0; $i -lt 300; $i++) { New-Junk (Join @($c, ('f_{0:x6}' -f $i))) ($rng.Next(512, 65536)) (2 + $rng.Next(0, 20)) }
}
$crash = Ensure-Dir (Join @($local, 'CrashDumps'))
New-Big (Join @($crash, 'SearchHost.exe.9020.dmp')) 25 11
$wer = Ensure-Dir (Join @($local, 'Microsoft', 'Windows', 'WER', 'ReportArchive', 'AppCrash_qa_1'))
New-Junk (Join @($wer, 'Report.wer')) 8192 30

# --- Downloads ----------------------------------------------------------------
Write-Step 'Downloads'
New-Big (Join @($downloads, 'driver-pack-2025.zip')) 60 210
New-Big (Join @($downloads, 'tool-installer-4.0.exe')) 15 240
$downloadsRecent = Join @($downloads, 'this-week-report.pdf')
New-Junk $downloadsRecent 50000 2

# --- Duplicates ---------------------------------------------------------------
Write-Step "Duplicates: $DuplicateSets sets"
$photos = Ensure-Dir (Join @($pictures, 'Camera Roll 2024'))
$docsDup = Ensure-Dir (Join @($documents, 'Invoices'))
$dupFirst = $null
for ($s = 0; $s -lt $DuplicateSets; $s++) {
  $bytes = $rng.Next(64KB, 2MB)
  $buf = New-Object byte[] $bytes
  $rng.NextBytes($buf)
  $setDir = if ($s % 2) { $photos } else { $docsDup }
  $ext = if ($s % 2) { '.jpg' } else { '.pdf' }
  $original = Join @($setDir,('item-{0:d4}{1}' -f $s, $ext))
  $copies = @($original, (Join @($downloads, ('item-{0:d4} (1){1}' -f $s, $ext))))
  if ($s % 3 -eq 0) { $copies += Join @($downloads, ('item-{0:d4} - Copy{1}' -f $s, $ext)) }
  if ($s % 7 -eq 0) { $copies += Join @((Ensure-Dir (Join @($Root, 'Desktop', 'sort later'))), ('item-{0:d4}{1}' -f $s, $ext)) }
  foreach ($p in $copies) {
    [IO.File]::WriteAllBytes($p, $buf)
    Set-Age $p (5 + $rng.Next(0, 300))
    $stats.files++
    $stats.bytes += $bytes
  }
  if ($null -eq $dupFirst) { $dupFirst = $copies }
}

# --- Empty folder trees -------------------------------------------------------
Write-Step "Empty folders: $EmptyFolders leaves"
$emptyRoot = Ensure-Dir (Join @($documents, 'Old projects'))
for ($e = 0; $e -lt $EmptyFolders; $e++) {
  [void](Ensure-Dir (Join @($emptyRoot, ('proj{0:d3}' -f [int]($e / 10)), 'build', ('out{0}' -f ($e % 10)), 'empty')))
}
# A non-empty folder inside the tree that must survive.
$keepDir = Ensure-Dir (Join @($emptyRoot, 'proj000', 'src'))
New-Junk (Join @($keepDir, 'main.rs')) 300 100

# --- Hostile cases ------------------------------------------------------------
Write-Step 'Unicode, long paths, links, denied, locked'
# Built from code points: Windows PowerShell 5.1 reads a BOM-less script as ANSI.
function U([int[]] $codes) { -join ($codes | ForEach-Object { [char]::ConvertFromUtf32($_) }) }
$uniDirName = (U 0xFC, 0x6E, 0xEF, 0x63, 0xF6, 0x64, 0xE9) + ' ' + (U 0x15F, 0x11F, 0x131, 0x130) + ' ' + (U 0x65E5, 0x672C, 0x8A9E) + ' ' + (U 0x2713)
$uniFileName = 'r' + (U 0xE9) + 'sum' + (U 0xE9) + ' ' + (U 0x1F600) + ' ' + (U 0xFB01) + 'le.tmp'
$uni = Ensure-Dir (Join @($temp, $uniDirName))
$uniFile = Join @($uni, $uniFileName)
New-Junk $uniFile 1500 12

# Long path: build with the \\?\ prefix so it works without the LongPathsEnabled policy.
$longDir = Join @($temp, 'winknife-qa-long')
$segment = 'a-very-long-folder-name-to-push-past-max-path-0123456789'
while ($longDir.Length -lt $LONG_PATH_TARGET_CHARS) { $longDir = Join @($longDir, $segment) }
[void][IO.Directory]::CreateDirectory('\\?\' + $longDir)
$longFile = Join @($longDir, 'deep-junk.tmp')
[IO.File]::WriteAllBytes('\\?\' + $longFile, (New-Object byte[] 777))
foreach ($setter in 'SetCreationTime', 'SetLastWriteTime', 'SetLastAccessTime') {
  [IO.File]::$setter.Invoke('\\?\' + $longFile, $now.AddDays(-20))
}

$links = Ensure-Dir (Join @($temp, 'winknife-qa-links'))
$loop = Join @($links, 'loop-to-temp')
$toWindows = Join @($links, 'junction-to-windows')
$symlink = Join @($links, 'symlink-to-sentinel.tmp')
foreach ($pair in @(@($loop, $temp), @($toWindows, $env:WINDIR))) {
  if (-not (Test-Path -LiteralPath $pair[0])) { & cmd /c mklink /J "$($pair[0])" "$($pair[1])" | Out-Null }
}
$symlinkMade = $false
try {
  if (-not (Test-Path -LiteralPath $symlink)) { New-Item -ItemType SymbolicLink -Path $symlink -Target $tempSentinel | Out-Null }
  $symlinkMade = $true
} catch {
  Write-Warning "File symlink not created (needs Developer Mode or elevation): $($_.Exception.Message)"
}

$denied = Ensure-Dir (Join @($temp, 'winknife-qa-denied'))
New-Junk (Join @($denied, 'secret.tmp')) 1000 30
$who = [Security.Principal.WindowsIdentity]::GetCurrent().Name
& icacls $denied /deny "${who}:(OI)(CI)(RD,X)" | Out-Null

$lockedDir = Ensure-Dir (Join @($temp, 'winknife-qa-locked'))
$locked = Join @($lockedDir, 'held-open.tmp')
New-Junk $locked 4096 30
$lockPid = $null
if (-not $NoLockHelper) {
  $escaped = $locked.Replace("'", "''")
  $holder = "`$f=[IO.File]::Open('$escaped','Open','ReadWrite','None'); while(`$true){Start-Sleep 3600}"
  $p = Start-Process powershell.exe -WindowStyle Hidden -PassThru -ArgumentList '-NoProfile', '-Command', $holder
  $lockPid = $p.Id
  # Re-age after the open; opening does not change times, but be explicit.
  Start-Sleep -Milliseconds 800
}

# --- Manifest -----------------------------------------------------------------
$manifest = [ordered]@{
  root = $Root
  createdAt = $now.ToString('o')
  counts = [ordered]@{ tempFiles = $TempFiles; duplicateSets = $DuplicateSets; emptyFolders = $EmptyFolders; files = $stats.files; dirs = $stats.dirs; bytes = $stats.bytes }
  sentinels = [ordered]@{
    tempOld = $tempSentinel
    tempFresh = $tempFresh
    downloadsRecent = $downloadsRecent
    unicode = $uniFile
    longPath = $longFile
    locked = $locked
    deniedDir = $denied
    junctionLoop = $loop
    junctionToWindows = $toWindows
    symlink = $(if ($symlinkMade) { $symlink } else { $null })
    emptyRoot = $emptyRoot
    emptyKeep = $keepDir
    duplicateSet = $dupFirst
  }
  lockHelperPid = $lockPid
}
$manifestPath = Join @($Root, $MANIFEST_NAME)
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 5), (New-Object System.Text.UTF8Encoding $false))
Write-Step ("Done: {0} files, {1} folders, {2:N0} MB. Manifest: {3}" -f $stats.files, $stats.dirs, ($stats.bytes / 1MB), $manifestPath)
