<#
.SYNOPSIS
    Packs the release build into a portable ZIP (no installer): Lanes.exe plus
    the FFmpeg DLLs, in target/release/bundle/portable/.

.DESCRIPTION
    Run after `pnpm build` (which builds target/release/lanes.exe with the UI
    embedded). The portable build keeps its settings in %APPDATA%\Lanes like
    the installed one. Admin mode is unavailable in the portable build, because
    the elevated task may only point to an exe under Program Files.
#>
param([string] $Version)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Version) {
    $Version = (Get-Content -Raw (Join-Path $root 'app\tauri.conf.json') | ConvertFrom-Json).version
}
$exe = Join-Path $root 'target\release\lanes.exe'
$ffmpeg = Join-Path $root 'third_party\ffmpeg'
if (-not (Test-Path $exe)) { throw "missing $exe; run 'pnpm build' first" }
if (-not (Test-Path "$ffmpeg\bin")) { throw "missing third_party\ffmpeg; run scripts\fetch-ffmpeg.ps1" }

$outDir = Join-Path $root 'target\release\bundle\portable'
New-Item -ItemType Directory -Force $outDir | Out-Null
$stage = Join-Path ([IO.Path]::GetTempPath()) ("lanes-portable-" + [Guid]::NewGuid())
New-Item -ItemType Directory "$stage\Lanes" | Out-Null
try {
    Copy-Item $exe "$stage\Lanes\Lanes.exe"
    # Only the libraries lanes.exe links (directly or through avcodec).
    foreach ($dll in 'avcodec-*.dll', 'avformat-*.dll', 'avutil-*.dll', 'swresample-*.dll') {
        Copy-Item "$ffmpeg\bin\$dll" "$stage\Lanes\"
    }
    Copy-Item "$ffmpeg\LICENSE.txt" "$stage\Lanes\FFmpeg-LICENSE.txt"
    @"
Lanes $Version (portable)

Start Lanes.exe. Settings: %APPDATA%\Lanes, recordings: %USERPROFILE%\Videos\Lanes.
Requires Windows 10/11 x64 and the Microsoft Edge WebView2 Runtime (preinstalled
on Windows 11 and current Windows 10; otherwise https://go.microsoft.com/fwlink/p/?LinkId=2124703).
"Run as administrator" mode needs the installer version (Program Files).

FFmpeg (avcodec, avformat, ...) is included as separate DLLs under its own
license, see FFmpeg-LICENSE.txt. Source: https://github.com/BtbN/FFmpeg-Builds
"@ | Set-Content -Encoding utf8 "$stage\Lanes\README.txt"
    $zip = Join-Path $outDir "Lanes_${Version}_x64-portable.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path "$stage\Lanes" -DestinationPath $zip
    Write-Host "Portable build: $zip"
} finally {
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}
