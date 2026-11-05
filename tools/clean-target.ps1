<#
.SYNOPSIS
  Frees disk space in target\ without throwing away the builds in use.

.DESCRIPTION
  Cargo never deletes old build output. Every dependency bump, feature change or toolchain
  update leaves the previous artifacts behind, so target\ grows without end. This script
  removes:
    - artifacts in debug\deps and release\deps (and their examples) that haven't been written
      for -Days days, when a newer build of the same crate exists;
    - incremental caches that haven't been written for -Days days;
    - installers and release folders in target\installer for versions older than the newest.
  A crate whose artifact was removed by mistake is just rebuilt on the next cargo run.

.PARAMETER Days
  Keep anything written in the last this-many days. Default 7. Some crates have two live
  builds (e.g. with and without build-script features); a short window rebuilds those more often.

.PARAMETER WhatIf
  List what would go without deleting.

.EXAMPLE
  pwsh tools\clean-target.ps1
  pwsh tools\clean-target.ps1 -Days 3 -WhatIf
#>
[CmdletBinding(SupportsShouldProcess)]
param([int]$Days = 7)

$ErrorActionPreference = 'Stop'
$target = Join-Path $PSScriptRoot '..\target' | Resolve-Path -ErrorAction SilentlyContinue
if (-not $target) { Write-Host 'No target folder.'; return }
$cutoff = (Get-Date).AddDays(-$Days)
$script:freed = 0L
$script:count = 0

function Remove-Stale([System.IO.FileSystemInfo]$item) {
    $size = if ($item.PSIsContainer) {
        (Get-ChildItem $item.FullName -Recurse -File -Force -ErrorAction SilentlyContinue |
            Measure-Object Length -Sum).Sum
    } else { $item.Length }
    if ($PSCmdlet.ShouldProcess($item.FullName, 'Remove')) {
        Remove-Item $item.FullName -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path $item.FullName) { return } # in use (cargo running?)
    }
    $script:freed += [long]$size
    $script:count++
}

foreach ($profile in 'debug', 'release') {
    foreach ($sub in 'deps', 'examples') {
        $dir = Join-Path $target "$profile\$sub"
        if (-not (Test-Path $dir)) { continue }
        # libfoo-0123456789abcdef.rlib -> group "libfoo|.rlib"; keep the newest of each group.
        Get-ChildItem $dir -File |
            Where-Object { $_.BaseName -match '^(.+)-[0-9a-f]{16}$' } |
            Group-Object { ($_.BaseName -replace '-[0-9a-f]{16}$', '') + '|' + $_.Extension } |
            ForEach-Object {
                $_.Group | Sort-Object LastWriteTime -Descending | Select-Object -Skip 1 |
                    Where-Object { $_.LastWriteTime -lt $cutoff } |
                    ForEach-Object { Remove-Stale $_ }
            }
    }
    $inc = Join-Path $target "$profile\incremental"
    if (Test-Path $inc) {
        Get-ChildItem $inc -Directory |
            Where-Object { $_.LastWriteTime -lt $cutoff } |
            ForEach-Object { Remove-Stale $_ }
    }
}

# Installers: keep the newest version only.
$installer = Join-Path $target 'installer'
if (Test-Path $installer) {
    $version = { param($name)
        if ($name -match '^(?:Clutch-|release-)(\d+)\.(\d+)\.(\d+)') {
            [version]::new([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
        }
    }
    $items = Get-ChildItem $installer | Where-Object { & $version $_.Name }
    $newest = $items | ForEach-Object { & $version $_.Name } | Sort-Object -Descending |
        Select-Object -First 1
    $items | Where-Object { (& $version $_.Name) -lt $newest } |
        ForEach-Object { Remove-Stale $_ }
}

$verb = if ($WhatIfPreference) { 'Would free' } else { 'Freed' }
'{0} {1:N2} GB ({2} items, kept anything newer than {3} days).' -f $verb, ($script:freed / 1GB), $script:count, $Days
