<#
Samples one studio app once a second and writes a CSV plus a summary.

  pwsh -File bench.ps1 -App clutch -Label record -Seconds 300 -Password <pw>
  pwsh -File bench.ps1 -App obs    -Label record -Seconds 300 -Password <pw>

Per sample: obs-websocket GetStats (fps, render time, skipped frames) and, measured from
outside the app, CPU / private memory / GPU 3D / GPU encode for the app's whole process
tree (main process plus browser-source helpers and other children).
#>
param(
    [Parameter(Mandatory)][ValidateSet('clutch', 'obs')][string]$App,
    [string]$Label = 'run',
    [int]$Seconds = 300,
    [int]$Port = 4455,
    [string]$Password = '',
    # Start recording before sampling and stop it afterwards.
    [switch]$Record,
    [string]$OutDir = (Join-Path $PSScriptRoot 'results')
)

$ErrorActionPreference = 'Stop'
$exe = @{ clutch = 'clutch'; obs = 'obs64' }[$App]
$cores = [Environment]::ProcessorCount
$none = [Threading.CancellationToken]::None

# --- obs-websocket v5 client -------------------------------------------------------

function Send-Ws($ws, $obj) {
    $bytes = [Text.Encoding]::UTF8.GetBytes(($obj | ConvertTo-Json -Depth 10 -Compress))
    $ws.SendAsync([ArraySegment[byte]]$bytes, 'Text', $true, $none).GetAwaiter().GetResult()
}

function Receive-Ws($ws) {
    $buf = [byte[]]::new(65536)
    $ms = [IO.MemoryStream]::new()
    do {
        $r = $ws.ReceiveAsync([ArraySegment[byte]]$buf, $none).GetAwaiter().GetResult()
        if ($r.MessageType -eq 'Close') { throw "server closed the connection: $($r.CloseStatusDescription)" }
        $ms.Write($buf, 0, $r.Count)
    } until ($r.EndOfMessage)
    [Text.Encoding]::UTF8.GetString($ms.ToArray()) | ConvertFrom-Json
}

function Get-B64Sha([string]$s) {
    [Convert]::ToBase64String([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($s)))
}

$ws = [Net.WebSockets.ClientWebSocket]::new()
$ws.Options.AddSubProtocol('obswebsocket.json')
$ws.ConnectAsync([Uri]"ws://127.0.0.1:$Port", $none).GetAwaiter().GetResult()

$hello = Receive-Ws $ws
$identify = @{ rpcVersion = 1; eventSubscriptions = 0 }
if ($hello.d.authentication) {
    if (-not $Password) { throw "$App asks for a password; pass -Password" }
    $a = $hello.d.authentication
    $identify.authentication = Get-B64Sha ((Get-B64Sha ($Password + $a.salt)) + $a.challenge)
}
Send-Ws $ws @{ op = 1; d = $identify }
$identified = Receive-Ws $ws
if ($identified.op -ne 2) { throw "identify failed: $($identified | ConvertTo-Json -Compress)" }

$reqId = 0
function Invoke-Req([string]$type, $data = $null) {
    $script:reqId++
    $d = @{ requestType = $type; requestId = "$script:reqId" }
    if ($data) { $d.requestData = $data }
    Send-Ws $ws @{ op = 6; d = $d }
    while ($true) {
        $m = Receive-Ws $ws
        if ($m.op -eq 7 -and $m.d.requestId -eq "$script:reqId") {
            if (-not $m.d.requestStatus.result) { throw "$type failed: $($m.d.requestStatus.comment)" }
            return $m.d.responseData
        }
    }
}
function Get-Stats { Invoke-Req 'GetStats' }

if ($Record) {
    if ((Invoke-Req 'GetRecordStatus').outputActive) { throw "$App is already recording" }
    [void](Invoke-Req 'StartRecord')
    $deadline = (Get-Date).AddSeconds(15)
    while (-not (Invoke-Req 'GetRecordStatus').outputActive) {
        if ((Get-Date) -gt $deadline) { throw "$App did not start recording" }
        Start-Sleep -Milliseconds 250
    }
    Start-Sleep -Seconds 3   # let the encoder settle before sampling
    Write-Host "recording started"
}

# --- process tree ------------------------------------------------------------------

$root = Get-Process $exe -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -First 1
if (-not $root) { throw "$exe.exe is not running" }

function Get-Tree([int]$rootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($rootId)
    do {
        $added = $false
        foreach ($p in $all) {
            if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added = $true }
        }
    } while ($added)
    , $ids
}

function Get-CpuTimes($ids) {
    $t = @{}
    foreach ($id in $ids) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($p) { $t[$id] = @{ cpu = $p.TotalProcessorTime.TotalMilliseconds; mem = $p.PrivateMemorySize64 } }
    }
    $t
}

# --- sampling ----------------------------------------------------------------------

[void](New-Item -ItemType Directory -Force $OutDir)
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$csv = Join-Path $OutDir "$App-$Label-$stamp.csv"
$counters = @(
    '\GPU Engine(*engtype_3D)\Utilization Percentage',
    '\GPU Engine(*engtype_VideoEncode)\Utilization Percentage',
    '\Processor(_Total)\% Processor Time'
)

Write-Host "Sampling $App ($exe.exe, pid $($root.Id)) for $Seconds s -> $csv"
$ids = Get-Tree $root.Id
$prev = Get-CpuTimes $ids
$prevAt = [Diagnostics.Stopwatch]::StartNew()
$rows = [Collections.Generic.List[object]]::new()

for ($i = 1; $i -le $Seconds; $i++) {
    if ($i % 10 -eq 0) { $ids = Get-Tree $root.Id }

    # Takes about one second, so it also paces the loop.
    $samples = (Get-Counter -Counter $counters -SampleInterval 1 -MaxSamples 1 -ErrorAction SilentlyContinue).CounterSamples

    $now = Get-CpuTimes $ids
    $elapsedMs = $prevAt.Elapsed.TotalMilliseconds
    $prevAt.Restart()
    $cpuMs = 0
    foreach ($id in $now.Keys) { if ($prev[$id]) { $cpuMs += $now[$id].cpu - $prev[$id].cpu } }
    $prev = $now
    $memMb = ($now.Values | Measure-Object -Property mem -Sum).Sum / 1MB

    $gpu3d = 0; $gpuEnc = 0; $sys3d = 0; $sysCpu = 0
    foreach ($s in $samples) {
        if ($s.Path -like '*\processor(_total)\*') { $sysCpu = $s.CookedValue; continue }
        $mine = $s.InstanceName -match 'pid_(\d+)_' -and $ids.Contains([int]$Matches[1])
        if ($s.InstanceName -like '*engtype_3d') {
            $sys3d += $s.CookedValue
            if ($mine) { $gpu3d += $s.CookedValue }
        } elseif ($mine) { $gpuEnc += $s.CookedValue }
    }

    $st = Get-Stats
    $row = [pscustomobject]@{
        t                = $i
        app_cpu_pct      = [math]::Round($cpuMs / $elapsedMs / $cores * 100, 2)
        app_mem_mb       = [math]::Round($memMb, 1)
        app_gpu3d_pct    = [math]::Round($gpu3d, 2)
        app_gpu_enc_pct  = [math]::Round($gpuEnc, 2)
        sys_cpu_pct      = [math]::Round($sysCpu, 2)
        sys_gpu3d_pct    = [math]::Round([math]::Min($sys3d, 100), 2)
        procs            = $ids.Count
        ws_cpu_pct       = [math]::Round($st.cpuUsage, 2)
        ws_mem_mb        = [math]::Round($st.memoryUsage, 1)
        fps              = [math]::Round($st.activeFps, 2)
        render_ms        = [math]::Round($st.averageFrameRenderTime, 3)
        render_skipped   = $st.renderSkippedFrames
        render_total     = $st.renderTotalFrames
        output_skipped   = $st.outputSkippedFrames
        output_total     = $st.outputTotalFrames
    }
    # Clutch also reports its p95 and CPU time per video-thread stage.
    if ($null -ne $st.clutchFrameRenderP95) {
        $row | Add-Member render_p95_ms ([math]::Round($st.clutchFrameRenderP95, 3))
        foreach ($p in $st.clutchVideoStages.PSObject.Properties) {
            $row | Add-Member "stage_$($p.Name)_ms" ([math]::Round($p.Value.averageMs, 3))
        }
    }
    $rows.Add($row)
    if ($i % 30 -eq 0) { Write-Host "  $i / $Seconds s" }
}

$rows | Export-Csv $csv -NoTypeInformation
if ($Record) {
    try {
        $out = Invoke-Req 'StopRecord'
        # StopRecord returns before the file is finalized; closing the app now would interrupt it.
        $deadline = (Get-Date).AddSeconds(30)
        while ((Invoke-Req 'GetRecordStatus').outputActive -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
        Write-Host "recording stopped: $($out.outputPath)"
    }
    catch { Write-Host "stop: $($_.Exception.Message)" }
}
# Save first: a server that drops the socket instead of answering the close must not lose the run.
try { $ws.CloseAsync('NormalClosure', 'done', $none).GetAwaiter().GetResult() } catch { Write-Host "close: $($_.Exception.Message)" }

# --- summary -----------------------------------------------------------------------

function P95($values) {
    $v = @($values | Sort-Object)
    if (-not $v.Count) { return 0 }
    $v[[math]::Min($v.Count - 1, [math]::Ceiling($v.Count * 0.95) - 1)]
}
$first = $rows[0]; $last = $rows[-1]
$summary = foreach ($col in 'app_cpu_pct', 'app_mem_mb', 'app_gpu3d_pct', 'app_gpu_enc_pct', 'sys_cpu_pct', 'sys_gpu3d_pct', 'fps', 'render_ms') {
    $v = $rows.$col
    [pscustomobject]@{
        metric = $col
        avg    = [math]::Round(($v | Measure-Object -Average).Average, 2)
        p95    = P95 $v
        max    = ($v | Measure-Object -Maximum).Maximum
    }
}
$summary | Format-Table -AutoSize
"render skipped: {0} of {1}   output skipped: {2} of {3}" -f
    ($last.render_skipped - $first.render_skipped), ($last.render_total - $first.render_total),
    ($last.output_skipped - $first.output_skipped), ($last.output_total - $first.output_total)
$summary | Export-Csv ($csv -replace '\.csv$', '-summary.csv') -NoTypeInformation
if ($Record -and ($last.output_total - $first.output_total) -le 0) {
    Write-Warning "$App reported no output frames: this run did not record"
}
