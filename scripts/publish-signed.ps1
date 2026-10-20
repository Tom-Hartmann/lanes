<#
.SYNOPSIS
    Builds a signed installer + portable ZIP on this PC and replaces the
    unsigned files of a GitHub release with them.

.DESCRIPTION
    GitHub's runners cannot sign with your local certificate (its private key
    never leaves this machine), so this script signs locally and uploads.
    Uses the signing configuration in scripts/signing.json (see
    scripts/new-dev-cert.ps1 and docs/SIGNING.md) and the GitHub login git
    already has (Git Credential Manager).

    A self-signed dev certificate is trusted only on PCs where it was
    imported. On other PCs the files show as signed by an unknown publisher,
    and admin mode refuses them. For signatures every PC trusts, use a real
    certificate (Azure Trusted Signing in the release workflow).

.EXAMPLE
    powershell -File scripts/publish-signed.ps1                # latest tag
    powershell -File scripts/publish-signed.ps1 -Tag v0.1.1
#>
param(
    [string] $Tag,
    [string] $Repo = 'Tom-Hartmann/lanes'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$config = Join-Path $PSScriptRoot 'signing.json'
if (-not (Test-Path $config) -or ((Get-Content -Raw $config | ConvertFrom-Json).mode -eq 'none')) {
    throw "No signing configured. Run scripts\new-dev-cert.ps1 (as admin) or set up scripts\signing.json first."
}
if (-not $Tag) { $Tag = (git describe --tags --abbrev=0).Trim() }
$version = $Tag.TrimStart('v')
Write-Host "Building signed Lanes $version for release $Tag"

# Same version stamping as the release workflow; restored afterwards.
$conf = 'app\tauri.conf.json'
$backup = @{ $conf = Get-Content -Raw $conf; 'Cargo.toml' = Get-Content -Raw 'Cargo.toml' }
try {
    $json = $backup[$conf] | ConvertFrom-Json
    $json.version = $version
    [IO.File]::WriteAllText((Resolve-Path $conf), ($json | ConvertTo-Json -Depth 32), (New-Object Text.UTF8Encoding($false)))
    [IO.File]::WriteAllText((Resolve-Path 'Cargo.toml'), ($backup['Cargo.toml'] -replace '(?m)^version = ".*"$', "version = `"$version`""), (New-Object Text.UTF8Encoding($false)))
    pnpm tauri build
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
    & (Join-Path $PSScriptRoot 'package-portable.ps1') -Version $version
} finally {
    foreach ($f in $backup.Keys) { [IO.File]::WriteAllText((Resolve-Path $f), $backup[$f], (New-Object Text.UTF8Encoding($false))) }
}

$files = @(
    "target\release\bundle\nsis\Lanes_${version}_x64-setup.exe",
    "target\release\bundle\portable\Lanes_${version}_x64-portable.zip"
)
foreach ($f in $files) {
    if (-not (Test-Path $f)) { throw "missing $f" }
}
$sig = Get-AuthenticodeSignature $files[0]
Write-Host "Installer signature: $($sig.Status) ($($sig.SignerCertificate.Subject))"

# GitHub token from Git Credential Manager (never printed).
$token = ("protocol=https`nhost=github.com`n`n" | git credential fill | Select-String '^password=').Line.Substring(9)
$headers = @{ Authorization = "Bearer $token"; Accept = 'application/vnd.github+json' }
$release = Invoke-RestMethod -Headers $headers "https://api.github.com/repos/$Repo/releases/tags/$Tag"

foreach ($f in $files) {
    $name = Split-Path $f -Leaf
    foreach ($old in @($release.assets | Where-Object name -eq $name)) {
        Invoke-RestMethod -Method Delete -Headers $headers "https://api.github.com/repos/$Repo/releases/assets/$($old.id)" | Out-Null
    }
    $upload = "https://uploads.github.com/repos/$Repo/releases/$($release.id)/assets?name=$([uri]::EscapeDataString($name))"
    Invoke-RestMethod -Method Post -Headers $headers -ContentType 'application/octet-stream' -InFile $f $upload | Out-Null
    Write-Host "Uploaded $name"
}
Write-Host "Done: $($release.html_url)"
