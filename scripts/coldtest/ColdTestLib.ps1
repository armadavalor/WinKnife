# Shared helpers for the cold-test bootstrap and verifier. Dot-source only.
# Windows PowerShell 5.1 compatible: Windows Sandbox has no pwsh.

$LONG_PREFIX = '\\?\'

function Add-LongPrefix([string] $path) {
  if ($path.StartsWith($LONG_PREFIX)) { return $path }
  $LONG_PREFIX + $path
}

function Remove-LongPrefix([string] $path) {
  if ($path.StartsWith($LONG_PREFIX)) { return $path.Substring($LONG_PREFIX.Length) }
  $path
}

# Every file under $Root, without following junctions or symlinks (the
# fixture plants a junction loop and a junction to C:\Windows). Folders that
# cannot be listed are counted, not fatal.
function Get-FileSet([string] $Root) {
  $files = New-Object System.Collections.Generic.List[string]
  $unlistable = 0
  $stack = New-Object System.Collections.Generic.Stack[string]
  $stack.Push((Add-LongPrefix $Root))
  while ($stack.Count -gt 0) {
    $dir = $stack.Pop()
    try {
      foreach ($f in [IO.Directory]::GetFiles($dir)) {
        $attr = [IO.File]::GetAttributes($f)
        if (-not ($attr -band [IO.FileAttributes]::ReparsePoint)) { $files.Add((Remove-LongPrefix $f)) }
      }
      foreach ($d in [IO.Directory]::GetDirectories($dir)) {
        $attr = [IO.File]::GetAttributes($d)
        if (-not ($attr -band [IO.FileAttributes]::ReparsePoint)) { $stack.Push($d) }
      }
    } catch {
      $unlistable++
    }
  }
  [pscustomobject]@{ Files = $files; Unlistable = $unlistable }
}

# SHA-256 of a file, or $null when it is missing, locked or denied.
function Get-Sha256OrNull([string] $path) {
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    $stream = [IO.File]::Open((Add-LongPrefix $path), 'Open', 'Read', 'ReadWrite')
    try {
      return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '')
    } finally {
      $stream.Dispose()
    }
  } catch {
    return $null
  } finally {
    $sha.Dispose()
  }
}

function Test-FileExists([string] $path) {
  [IO.File]::Exists((Add-LongPrefix $path))
}
