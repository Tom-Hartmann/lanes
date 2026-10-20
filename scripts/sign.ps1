<#
.SYNOPSIS
    Authenticode-signs Lanes binaries with signtool (SHA-256 + RFC 3161 timestamp).

.DESCRIPTION
    Called by Tauri for every binary it bundles (app/tauri.conf.json
    bundle.windows.signCommand passes the file as the first argument), and usable
    by hand to sign a whole folder, e.g. the FFmpeg DLLs shipped next to lanes.exe:

        powershell -File scripts/sign.ps1 -Path target/release

    Every signed file is checked afterwards with `signtool verify /pa`.

    Configuration, lowest to highest precedence:
      1. defaults (mode "none", timestamp http://timestamp.digicert.com)
      2. scripts/signing.json (gitignored; see scripts/signing.example.json)
      3. environment variables
           LANES_SIGN_MODE           none | thumbprint | azure
           LANES_SIGN_THUMBPRINT     SHA-1 thumbprint of a cert in CurrentUser\My or LocalMachine\My
           LANES_SIGN_TIMESTAMP_URL  RFC 3161 timestamp server
           LANES_SIGN_DLIB           path to Azure.CodeSigning.Dlib.dll (Azure Trusted Signing)
           LANES_SIGN_METADATA       path to the Trusted Signing metadata.json
           LANES_SIGNTOOL            explicit path to signtool.exe
      4. parameters of this script

    Mode "none" skips signing with a warning so `pnpm build` works without a
    certificate. Any signing or verification failure exits non-zero.

    See docs/SIGNING.md.
#>
[CmdletBinding()]
param(
    # File to sign, or a directory whose .exe/.dll files (recursively) are signed.
    [Parameter(Mandatory = $true, Position = 0)]
    [string] $Path,
    [ValidateSet('none', 'thumbprint', 'azure')]
    [string] $Mode,
    [string] $Thumbprint,
    [string] $TimestampUrl,
    [string] $DlibPath,
    [string] $MetadataPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$DefaultTimestampUrl = 'http://timestamp.digicert.com'
$ConfigFile = Join-Path $PSScriptRoot 'signing.json'

function Get-JsonValue($Object, [string] $Name) {
    if ($null -ne $Object -and $Object.PSObject.Properties.Name -contains $Name) { return [string] $Object.$Name }
    return $null
}

# Picks the first non-empty value: parameter, then environment, then file, then default.
function Resolve-Setting([string] $Param, [string] $EnvName, $Json, [string] $JsonName, [string] $Default) {
    if ($Param) { return $Param }
    $fromEnv = [Environment]::GetEnvironmentVariable($EnvName)
    if ($fromEnv) { return $fromEnv }
    $fromJson = Get-JsonValue $Json $JsonName
    if ($fromJson) { return $fromJson }
    return $Default
}

function Find-SignTool {
    if ($env:LANES_SIGNTOOL) {
        if (-not (Test-Path -LiteralPath $env:LANES_SIGNTOOL)) { throw "LANES_SIGNTOOL points to a missing file: $env:LANES_SIGNTOOL" }
        return $env:LANES_SIGNTOOL
    }
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (Test-Path -LiteralPath $kits) {
        # Version folders look like 10.0.26100.0; pick the newest that has an x64 signtool.
        $newest = Get-ChildItem -LiteralPath $kits -Directory |
            Where-Object { $_.Name -match '^\d+(\.\d+){3}$' -and (Test-Path -LiteralPath (Join-Path $_.FullName 'x64\signtool.exe')) } |
            Sort-Object { [version] $_.Name } -Descending |
            Select-Object -First 1
        if ($newest) { return (Join-Path $newest.FullName 'x64\signtool.exe') }
    }
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    throw 'signtool.exe not found. Install the Windows SDK (Signing Tools) or set LANES_SIGNTOOL.'
}

# Runs signtool, retrying a few times because public timestamp servers are occasionally flaky.
function Invoke-SignTool([string] $Tool, [string[]] $Arguments, [int] $Attempts = 3) {
    for ($i = 1; $i -le $Attempts; $i++) {
        & $Tool @Arguments
        if ($LASTEXITCODE -eq 0) { return }
        if ($i -lt $Attempts) {
            Write-Warning "signtool exited with $LASTEXITCODE; retrying ($i/$Attempts)"
            Start-Sleep -Seconds (2 * $i)
        }
    }
    throw "signtool $($Arguments[0]) failed with exit code $LASTEXITCODE"
}

try {
    $json = $null
    if (Test-Path -LiteralPath $ConfigFile) {
        $json = Get-Content -LiteralPath $ConfigFile -Raw | ConvertFrom-Json
    }
    $Mode = Resolve-Setting $Mode 'LANES_SIGN_MODE' $json 'mode' 'none'
    $Thumbprint = Resolve-Setting $Thumbprint 'LANES_SIGN_THUMBPRINT' $json 'thumbprint' ''
    $TimestampUrl = Resolve-Setting $TimestampUrl 'LANES_SIGN_TIMESTAMP_URL' $json 'timestampUrl' $DefaultTimestampUrl
    $DlibPath = Resolve-Setting $DlibPath 'LANES_SIGN_DLIB' $json 'dlibPath' ''
    $MetadataPath = Resolve-Setting $MetadataPath 'LANES_SIGN_METADATA' $json 'metadataPath' ''

    if (-not (Test-Path -LiteralPath $Path)) { throw "Not found: $Path" }
    $item = Get-Item -LiteralPath $Path
    if ($item.PSIsContainer) {
        $files = @(Get-ChildItem -LiteralPath $item.FullName -Recurse -File |
                Where-Object { $_.Extension -in '.exe', '.dll' } |
                ForEach-Object { $_.FullName })
    } else {
        $files = @($item.FullName)
    }
    if ($files.Count -eq 0) {
        Write-Warning "No .exe or .dll files under $Path"
        exit 0
    }

    if ($Mode -eq 'none') {
        Write-Warning "Code signing is disabled (mode 'none'); $($files.Count) file(s) left unsigned. See docs/SIGNING.md."
        exit 0
    }

    # Mode-specific signtool arguments.
    switch ($Mode) {
        'thumbprint' {
            if (-not $Thumbprint) { throw "mode 'thumbprint' needs a thumbprint (signing.json or LANES_SIGN_THUMBPRINT)" }
            $Thumbprint = ($Thumbprint -replace '[^0-9A-Fa-f]', '').ToUpperInvariant()
            if (Test-Path -LiteralPath "Cert:\CurrentUser\My\$Thumbprint") {
                $certArgs = @('/s', 'My', '/sha1', $Thumbprint)
            } elseif (Test-Path -LiteralPath "Cert:\LocalMachine\My\$Thumbprint") {
                $certArgs = @('/sm', '/s', 'My', '/sha1', $Thumbprint)
            } else {
                throw "Certificate $Thumbprint not found in CurrentUser\My or LocalMachine\My"
            }
        }
        'azure' {
            if (-not $DlibPath -or -not (Test-Path -LiteralPath $DlibPath)) { throw "mode 'azure' needs dlibPath pointing at Azure.CodeSigning.Dlib.dll" }
            if (-not $MetadataPath -or -not (Test-Path -LiteralPath $MetadataPath)) { throw "mode 'azure' needs metadataPath pointing at the Trusted Signing metadata.json" }
            $certArgs = @('/dlib', (Resolve-Path -LiteralPath $DlibPath).Path, '/dmdf', (Resolve-Path -LiteralPath $MetadataPath).Path)
        }
        default { throw "Unknown signing mode '$Mode'" }
    }

    $signtool = Find-SignTool
    Write-Host "Signing $($files.Count) file(s) with $signtool (mode $Mode, timestamp $TimestampUrl)"
    foreach ($file in $files) {
        $signArgs = @('sign', '/fd', 'SHA256', '/tr', $TimestampUrl, '/td', 'SHA256') + $certArgs + @($file)
        Invoke-SignTool $signtool $signArgs
        Invoke-SignTool $signtool @('verify', '/pa', '/q', $file) -Attempts 1
        Write-Host "Signed and verified: $file"
    }
    exit 0
} catch {
    Write-Host "sign.ps1: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
