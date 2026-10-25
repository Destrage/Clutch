<#
Side-by-side table of the newest clutch and obs run for each label that bench.ps1 wrote.

  pwsh -File compare.ps1            # every label with both apps
  pwsh -File compare.ps1 -Label record
#>
param([string]$Label = '*', [string]$Dir = (Join-Path $PSScriptRoot 'results'))

$newest = @{}
foreach ($f in Get-ChildItem $Dir -Filter "*-$Label-*-summary.csv" | Sort-Object LastWriteTime) {
    if ($f.Name -match '^(clutch|obs)-(.+)-\d{8}-\d{6}-summary\.csv$') {
        $newest["$($Matches[2])|$($Matches[1])"] = $f.FullName
    }
}

$labels = $newest.Keys | ForEach-Object { ($_ -split '\|')[0] } | Sort-Object -Unique
foreach ($l in $labels) {
    $c = $newest["$l|clutch"]; $o = $newest["$l|obs"]
    if (-not ($c -and $o)) { Write-Host "$l : need both a clutch and an obs run"; continue }
    $obs = @{}; Import-Csv $o | ForEach-Object { $obs[$_.metric] = $_ }
    Write-Host "`n== $l ==" -ForegroundColor Cyan
    Import-Csv $c | ForEach-Object {
        $b = $obs[$_.metric]
        [pscustomobject]@{
            metric       = $_.metric
            'clutch avg' = [double]$_.avg
            'obs avg'    = [double]$b.avg
            'clutch p95' = [double]$_.p95
            'obs p95'    = [double]$b.p95
            'avg diff %' = if ([double]$b.avg) { [math]::Round(([double]$_.avg / [double]$b.avg - 1) * 100, 1) } else { '' }
        }
    } | Format-Table -AutoSize
}
