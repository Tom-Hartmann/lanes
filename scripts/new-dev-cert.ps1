<#
.SYNOPSIS
    Creates (or removes) a self-signed code-signing certificate for local Lanes builds.

.DESCRIPTION
    ##########################################################################
    #  WARNING: RUN THIS ON YOUR OWN DEVELOPMENT MACHINE ONLY.               #
    #                                                                        #
    #  It adds a self-made certificate to LocalMachine\Root and              #
    #  LocalMachine\TrustedPublisher, which makes THIS machine trust         #
    #  anything signed with its private key. Never install the certificate  #
    #  (or the exported .cer) on other machines, never ship it, and never   #
    #  ask users to trust it. Releases are signed with a real certificate;  #
    #  see docs/SIGNING.md.                                                  #
    ##########################################################################

    Without -Remove:
      1. creates "CN=Lanes Dev Signing" (RSA 3072, SHA-256, 2 years) in
         CurrentUser\My, or reuses a still-valid one,
      2. exports its public part to scripts/lanes-dev-signing.cer (gitignored),
      3. imports that into LocalMachine\Root and LocalMachine\TrustedPublisher,
      4. writes the thumbprint to scripts/signing.json (mode "thumbprint").

    The private key never leaves CurrentUser\My and is not exportable.

    With -Remove: deletes every "CN=Lanes Dev Signing" certificate from the
    three stores, the exported .cer, and scripts/signing.json if it points at
    one of them.

    Requires an elevated PowerShell (LocalMachine stores are admin-only).
#>
[CmdletBinding()]
param(
    [switch] $Remove,
    [ValidateRange(1, 5)]
    [int] $Years = 2
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Subject = 'CN=Lanes Dev Signing'
$CerFile = Join-Path $PSScriptRoot 'lanes-dev-signing.cer'
$ConfigFile = Join-Path $PSScriptRoot 'signing.json'
$Stores = @('Cert:\CurrentUser\My', 'Cert:\LocalMachine\Root', 'Cert:\LocalMachine\TrustedPublisher')

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host @'
new-dev-cert.ps1 must run elevated: it writes to the LocalMachine\Root and
LocalMachine\TrustedPublisher certificate stores, which only administrators may change.
Open "Windows PowerShell" with "Run as administrator" and run it again.
'@ -ForegroundColor Red
    exit 1
}

function Write-Utf8NoBom([string] $File, [string] $Text) {
    [IO.File]::WriteAllText($File, $Text, (New-Object Text.UTF8Encoding($false)))
}

if ($Remove) {
    $removed = @()
    foreach ($store in $Stores) {
        foreach ($cert in @(Get-ChildItem -LiteralPath $store | Where-Object { $_.Subject -eq $Subject })) {
            Remove-Item -LiteralPath $cert.PSPath -DeleteKey:($store -like '*\My') -ErrorAction Continue
            Write-Host "Removed $($cert.Thumbprint) from $store"
            $removed += $cert.Thumbprint
        }
    }
    if (Test-Path -LiteralPath $CerFile) { Remove-Item -LiteralPath $CerFile }
    if (Test-Path -LiteralPath $ConfigFile) {
        $config = Get-Content -LiteralPath $ConfigFile -Raw | ConvertFrom-Json
        if ($config.PSObject.Properties.Name -contains 'thumbprint' -and $removed -contains ([string] $config.thumbprint).ToUpperInvariant()) {
            Remove-Item -LiteralPath $ConfigFile
            Write-Host "Removed $ConfigFile"
        }
    }
    if ($removed.Count -eq 0) { Write-Host "No '$Subject' certificates found." }
    exit 0
}

# Reuse a valid existing certificate so repeated runs don't pile up keys.
$cert = Get-ChildItem -LiteralPath 'Cert:\CurrentUser\My' |
    Where-Object { $_.Subject -eq $Subject -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date).AddDays(30) } |
    Sort-Object NotAfter -Descending |
    Select-Object -First 1

if ($cert) {
    Write-Host "Reusing $Subject ($($cert.Thumbprint), expires $($cert.NotAfter.ToString('yyyy-MM-dd')))"
} else {
    $cert = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject $Subject `
        -FriendlyName 'Lanes Dev Signing (local only)' `
        -KeyAlgorithm RSA `
        -KeyLength 3072 `
        -HashAlgorithm SHA256 `
        -KeyExportPolicy NonExportable `
        -CertStoreLocation 'Cert:\CurrentUser\My' `
        -NotAfter (Get-Date).AddYears($Years)
    Write-Host "Created $Subject ($($cert.Thumbprint))"
}

Export-Certificate -Cert $cert -FilePath $CerFile -Type CERT | Out-Null
foreach ($store in 'Cert:\LocalMachine\Root', 'Cert:\LocalMachine\TrustedPublisher') {
    Import-Certificate -FilePath $CerFile -CertStoreLocation $store | Out-Null
    Write-Host "Trusted on this machine: $store"
}

# Keep an existing timestamp URL; switch the mode to this certificate.
$timestampUrl = 'http://timestamp.digicert.com'
if (Test-Path -LiteralPath $ConfigFile) {
    $old = Get-Content -LiteralPath $ConfigFile -Raw | ConvertFrom-Json
    if ($old.PSObject.Properties.Name -contains 'timestampUrl' -and $old.timestampUrl) { $timestampUrl = [string] $old.timestampUrl }
}
$config = [ordered] @{ mode = 'thumbprint'; thumbprint = $cert.Thumbprint; timestampUrl = $timestampUrl }
Write-Utf8NoBom $ConfigFile ($config | ConvertTo-Json)

Write-Host ''
Write-Host "Thumbprint: $($cert.Thumbprint)"
Write-Host "Wrote $ConfigFile; 'pnpm build' now signs with this certificate."
Write-Host 'Undo with: scripts\new-dev-cert.ps1 -Remove'
