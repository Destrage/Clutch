<#
Back-to-back recording benchmark of Clutch and obs-studio with matched setups:
one game capture, 1920x1080 at 60 fps, desktop audio + mic, NVENC H.264 CBR 12 Mbps,
AAC 192 kbps, mkv, no replay/clip buffer. Each app's settings are backed up before the run
and restored after it. See README.md.

  pwsh -File run-pair.ps1 -ClutchPassword <pw>
#>
param(
    [Parameter(Mandatory)][string]$ClutchPassword,
    [int]$Seconds = 300,
    # Clutch's window id for the game: exe, class and title separated by U+001F.
    [string]$Window = "r5apex_dx12.exe$([char]0x1f)Respawn001$([char]0x1f)Apex Legends"
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$clutchExe = 'C:\Program Files\Clutch\clutch.exe'
$obsDir = 'C:\Program Files\obs-studio\bin\64bit'
$clutchDir = "$env:APPDATA\Clutch"
$obsProfile = "$env:APPDATA\obs-studio\basic\profiles\Untitled\basic.ini"
$obsPassword = (Get-Content "$env:APPDATA\obs-studio\plugin_config\obs-websocket\config.json" -Raw | ConvertFrom-Json).server_password

function Log($msg) { Write-Host "[$(Get-Date -Format 'h:mm:ss tt')] $msg" }

function Wait-Port([int]$port, [int]$timeoutS = 60) {
    $deadline = (Get-Date).AddSeconds($timeoutS)
    while ((Get-Date) -lt $deadline) {
        $c = [Net.Sockets.TcpClient]::new()
        try { $c.Connect('127.0.0.1', $port); return } catch { Start-Sleep -Milliseconds 500 } finally { $c.Dispose() }
    }
    throw "nothing listening on $port after $timeoutS s"
}

function Close-App([string]$name) {
    $p = Get-Process $name -ErrorAction SilentlyContinue
    if (-not $p) { return }
    # Without /F taskkill posts WM_CLOSE, so the app shuts down cleanly.
    taskkill /IM "$name.exe" | Out-Null
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Process $name -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
    if (Get-Process $name -ErrorAction SilentlyContinue) {
        Log "$name did not close; ending it"
        Stop-Process -Name $name -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
}

function New-Id { [guid]::NewGuid().ToString() }

# One canvas, one scene: a full-canvas game capture of $Window, plus desktop audio and mic.
function New-BenchScene([string]$path) {
    $canvas = New-Id; $scene = New-Id; $game = New-Id
    $audio = { param($name, $kind) @{
            id = (New-Id); name = $name; kind = $kind; settings = @{ device = '' }
            audio = @{ volume_db = 0.0; muted = $false; balance = 0.5; mono = $false; sync_offset_ms = 0
                monitoring = 'off'; tracks = 63; global = $true; filters = @() }
        } }
    @{
        schema     = 1
        canvases   = @(@{ id = $canvas; name = 'Main'; width = 1920; height = 1080; fps = @{ num = 60; den = 1 }
                background = @{ r = 0; g = 0; b = 0; a = 255 }; active_scene = $scene })
        scenes     = @(@{ id = $scene; name = 'Bench'; canvas = $canvas; items = @(@{
                        id = (New-Id); content = @{ type = 'source'; source = $game }
                        transform = @{ position = @(0.0, 0.0); rotation = 0.0; scale = @(1.0, 1.0); bounds_mode = 'fit'
                            bounds = @(1920.0, 1080.0); bounds_alignment = 'center' }
                        crop = @{ left = 0; top = 0; right = 0; bottom = 0 }
                        visible = $true; locked = $false; opacity = 1.0; blend = 'normal'
                    }) })
        sources    = @(
            (& $audio 'Desktop Audio' 'audio_output'),
            (& $audio 'Microphone' 'audio_input'),
            @{ id = $game; name = 'Game capture'; kind = 'game_capture'
                settings = @{ audio = $false; cursor = $false; mode = 'window'; window = $Window } }
        )
        audio      = @{ monitor_device = '' }
        transition = @{ kind = @{ type = 'fade' }; duration_ms = 300 }
    } | ConvertTo-Json -Depth 20 | Set-Content $path -Encoding utf8NoBOM
}

if (Get-Process clutch, obs64 -ErrorAction SilentlyContinue) { throw 'close Clutch and OBS first' }
$gameExe = ($Window -split [char]0x1f)[0] -replace '\.exe$', ''
while (-not (Get-Process $gameExe -ErrorAction SilentlyContinue)) { Log "waiting for $gameExe"; Start-Sleep 10 }

# --- Clutch --------------------------------------------------------------------------

$settings = "$clutchDir\settings.json"
$backup = "$clutchDir\settings.bench-backup.json"
$bench = "$clutchDir\scenes\Bench.json"
Copy-Item $settings $backup -Force
try {
    New-BenchScene $bench

    $text = Get-Content $settings -Raw
    $text = $text -replace '"last_collection": "[^"]*"', ('"last_collection": "' + ($bench -replace '\\', '\\') + '"')
    $text = $text -replace '"mode": true,', '"mode": false,'                       # clip mode off
    $text = $text -replace '"rate_control": "[a-z_]+"', '"rate_control": "cbr"'
    $text = $text -replace '"game_audio": "ask"', '"game_audio": "skip"'          # no prompt mid-game
    Set-Content $settings $text -Encoding utf8NoBOM -NoNewline
    $r = Get-Content $settings -Raw | ConvertFrom-Json
    if ($r.clips.mode -or $r.recording.rate_control -ne 'cbr' -or $r.recording.bitrate_kbps -ne 12000) { throw 'Clutch settings edit did not apply' }

    Log 'starting Clutch'
    Start-Process $clutchExe -WindowStyle Minimized
    Wait-Port 4455
    Start-Sleep -Seconds 20   # game capture hook and audio devices
    Log 'benchmarking Clutch'
    pwsh -NoProfile -File "$here\bench.ps1" -App clutch -Label record -Seconds $Seconds -Password $ClutchPassword -Record
    Close-App clutch
} finally {
    if (Get-Process clutch -ErrorAction SilentlyContinue) { Close-App clutch }
    Copy-Item $backup $settings -Force
    Remove-Item $backup, $bench -ErrorAction SilentlyContinue
    Log 'Clutch settings restored'
}

Start-Sleep -Seconds 15

# --- OBS -----------------------------------------------------------------------------

$obsBackup = "$obsProfile.bench-backup"
Copy-Item $obsProfile $obsBackup -Force
try {
    @'
[General]
Name=Untitled

[Video]
BaseCX=1920
BaseCY=1080
OutputCX=1920
OutputCY=1080
FPSType=0
FPSCommon=60

[Output]
Mode=Simple

[AdvOut]
Encoder=obs_nvenc_h264_tex

[SimpleOutput]
StreamEncoder=nvenc
RecQuality=Stream
VBitrate=12000
ABitrate=192
RecFormat2=mkv
'@ | Set-Content $obsProfile -Encoding utf8NoBOM

    Log 'starting OBS'
    Start-Process "$obsDir\obs64.exe" -WorkingDirectory $obsDir -WindowStyle Minimized `
        -ArgumentList '--disable-shutdown-check', '--disable-updater'
    Wait-Port 4455
    Start-Sleep -Seconds 20
    Log 'benchmarking OBS'
    pwsh -NoProfile -File "$here\bench.ps1" -App obs -Label record -Seconds $Seconds -Password $obsPassword -Record
    Close-App obs64
} finally {
    if (Get-Process obs64 -ErrorAction SilentlyContinue) { Close-App obs64 }
    Copy-Item $obsBackup $obsProfile -Force
    Remove-Item $obsBackup -ErrorAction SilentlyContinue
    Log 'OBS profile restored'
}

pwsh -NoProfile -File "$here\compare.ps1" -Label record
