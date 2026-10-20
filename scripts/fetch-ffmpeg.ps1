<#
.SYNOPSIS
    Downloads the FFmpeg 9.0 shared build Lanes links against into third_party/ffmpeg.

.DESCRIPTION
    Uses BtbN's FFmpeg-Builds (https://github.com/BtbN/FFmpeg-Builds). The GPL
    variant includes x264 (the CPU fallback encoder); the LGPL variant does not,
    in which case Lanes only offers hardware encoders. NVENC, AMF and QSV are in
    both. The archive is extracted into a fresh temporary folder first and only
    the extracted tree is moved into place.

.EXAMPLE
    powershell -File scripts/fetch-ffmpeg.ps1            # GPL (default)
    powershell -File scripts/fetch-ffmpeg.ps1 -License lgpl
#>
param(
    [ValidateSet('gpl', 'lgpl')] [string] $License = 'gpl',
    [switch] $Force
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root 'third_party\ffmpeg'
if ((Test-Path $dest) -and -not $Force) {
    Write-Host "third_party\ffmpeg already exists (use -Force to replace it)."
    exit 0
}
$name = "ffmpeg-n9.0-latest-win64-$License-shared-9.0"
$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/$name.zip"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("lanes-ffmpeg-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "Downloading $url"
    Invoke-WebRequest -UseBasicParsing $url -OutFile "$tmp\ffmpeg.zip"
    Expand-Archive "$tmp\ffmpeg.zip" "$tmp\x"
    $inner = Get-ChildItem "$tmp\x" -Directory | Select-Object -First 1
    if (-not (Test-Path "$($inner.FullName)\include\libavcodec\avcodec.h")) { throw "unexpected archive layout" }
    if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
    New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
    Move-Item $inner.FullName $dest
    Write-Host "FFmpeg ($License) installed to $dest"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
