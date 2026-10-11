param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?$')]
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $repoRoot 'release-artifacts'
$stage = Join-Path ([IO.Path]::GetTempPath()) ('winknife-package-' + [guid]::NewGuid())
$exe = Join-Path $repoRoot 'src-tauri/target/release/winknife.exe'
$installers = @(Get-ChildItem -LiteralPath (Join-Path $repoRoot 'src-tauri/target/release/bundle/nsis') -Filter '*-setup.exe' -File)
if ($installers.Count -ne 1) { throw 'Expected exactly one NSIS installer; review stale build outputs first.' }
if (!(Test-Path -LiteralPath $exe)) { throw 'Build the production executable first.' }
if (Test-Path -LiteralPath $outDir) { throw 'release-artifacts already exists; archive or review it before packaging again.' }
New-Item -ItemType Directory -Path $stage, $outDir | Out-Null
Copy-Item -LiteralPath $exe -Destination $stage
Copy-Item -LiteralPath (Join-Path $repoRoot 'rules') -Destination $stage -Recurse
foreach ($name in @('LICENSE', 'README.md', 'README.tr.md')) {
    Copy-Item -LiteralPath (Join-Path $repoRoot $name) -Destination $stage
}
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath (Join-Path $outDir "WinKnife_${Version}_x64-portable.zip")
Copy-Item -LiteralPath $installers[0].FullName -Destination (Join-Path $outDir "WinKnife_${Version}_x64-setup.exe")
$checksums = Get-ChildItem -LiteralPath $outDir -File | Sort-Object Name | ForEach-Object {
    $hash = Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256
    '{0}  {1}' -f $hash.Hash.ToLowerInvariant(), $_.Name
}
$checksums | Set-Content -LiteralPath (Join-Path $outDir 'SHA256SUMS.txt') -Encoding ascii
# Remove only this invocation's generated temporary staging directory.
$resolvedStage = (Resolve-Path -LiteralPath $stage).Path
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
if (!$resolvedStage.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or !(Split-Path -Leaf $resolvedStage).StartsWith('winknife-package-')) {
    throw 'Refusing to clean a staging directory outside the expected temporary root.'
}
Remove-Item -LiteralPath $resolvedStage -Recurse
Write-Output $outDir
