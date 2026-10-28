<#
Builds Clutch's Linux packages in an Ubuntu 24.04 container (Podman), so they run on every
supported distro, and copies them to target\installer. Needs the Linux FFmpeg and CEF in
third_party\linux\ffmpeg and third_party\linux\cef. See README.md.

  pwsh -File tools\linux-build\package.ps1
#>
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$image = 'clutch-linux-build'

foreach ($dir in 'third_party\linux\ffmpeg', 'third_party\linux\cef') {
    if (-not (Test-Path "$root\$dir")) { throw "$dir is missing (see README.md, Linux packages)" }
}

podman build -t $image -f "$PSScriptRoot\Containerfile" $PSScriptRoot
if ($LASTEXITCODE) { throw 'container image build failed' }

# The build and Cargo's downloads live in volumes: fast, and kept between runs.
podman run --rm `
    -v "${root}:/src" `
    -v clutch-linux-target:/target `
    -v clutch-linux-cargo:/root/.cargo/registry `
    -e CARGO_TARGET_DIR=/target `
    -e FFMPEG_DIR=/src/third_party/linux/ffmpeg `
    -e CEF_PATH=/src/third_party/linux/cef `
    -w /src $image `
    bash -c 'cargo xtask package && mkdir -p /src/target/installer && cp /target/package/Clutch-* /src/target/installer/'
if ($LASTEXITCODE) { throw 'Linux packaging failed' }
Get-ChildItem "$root\target\installer\Clutch-*" | Where-Object { $_.Name -notlike '*.msi' } |
    Sort-Object LastWriteTime | Select-Object -Last 3 Name, Length, LastWriteTime
