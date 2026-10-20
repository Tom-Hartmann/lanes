# Code signing

Lanes ships as a per-machine NSIS installer. Everything we build is Authenticode-signed with SHA-256 and an
RFC 3161 timestamp. There are two reasons:

- **Admin mode depends on it.** The elevated scheduled task (`\Lanes\Elevated`) is only registered for a
  `lanes.exe` that lives under Program Files *and* has a valid embedded signature
  (`lanes_platform::admin::verify_signature`). A task that starts a program elevated without a prompt is
  only safe when nobody but an administrator can swap that program.
- **Antivirus and SmartScreen.** A capture app reads the screen, records audio and may install a keyboard
  hook. Unsigned binaries that do this tend to get flagged.

## How it works

`app/tauri.conf.json` sets

```json
"signCommand": "powershell -NoProfile -ExecutionPolicy Bypass -File ../scripts/sign.ps1 %1"
```

Tauri runs this command for every binary it signs and passes the file path as `%1`. That covers `lanes.exe`,
the NSIS uninstaller and the installer. The Tauri CLI works from `app/` and turns relative arguments that
exist into absolute paths, so `../scripts/sign.ps1` also resolves when NSIS signs the uninstaller from its own
output directory.

`scripts/sign.ps1` finds the newest `signtool.exe` under `C:\Program Files (x86)\Windows Kits\10\bin\*\x64\`
(or uses `LANES_SIGNTOOL`). It runs

```
signtool sign /fd SHA256 /tr <timestamp url> /td SHA256 <certificate arguments> <file>
signtool verify /pa <file>
```

and exits non-zero if either step fails. The timestamp step is retried a few times because public timestamp
servers are sometimes flaky. Thanks to the timestamp, signatures stay valid after the certificate expires.

### Configuration

Settings are read in this order, and later sources override earlier ones: defaults, `scripts/signing.json`
(gitignored; copy `scripts/signing.example.json`), environment variables, then script parameters.

| Setting        | `signing.json` | Environment                | Default                         |
| -------------- | -------------- | -------------------------- | ------------------------------- |
| Mode           | `mode`         | `LANES_SIGN_MODE`          | `none`                          |
| Thumbprint     | `thumbprint`   | `LANES_SIGN_THUMBPRINT`    |                                 |
| Timestamp URL  | `timestampUrl` | `LANES_SIGN_TIMESTAMP_URL` | `http://timestamp.digicert.com` |
| Azure dlib     | `dlibPath`     | `LANES_SIGN_DLIB`          |                                 |
| Azure metadata | `metadataPath` | `LANES_SIGN_METADATA`      |                                 |

The modes are:

- **`none`**: skips signing with a warning, so `pnpm build` works without a certificate. The resulting build
  cannot use admin mode.
- **`thumbprint`**: uses a certificate from `CurrentUser\My` or `LocalMachine\My` (`/sha1 <thumbprint>`).
  This covers the dev certificate, an OV/EV certificate on a hardware token, and a cloud HSM that exposes its
  certificate through a CSP/KSP.
- **`azure`**: uses Azure Trusted Signing (`/dlib Azure.CodeSigning.Dlib.dll /dmdf metadata.json`).

### What gets signed

| File                                | Signed by                                            |
| ----------------------------------- | ---------------------------------------------------- |
| `lanes.exe`                         | Tauri via `signCommand`                              |
| NSIS uninstaller and installer      | Tauri via `signCommand`                              |
| FFmpeg DLLs (`avcodec-*.dll`, etc.) | `scripts/sign.ps1 -Path <folder>` before bundling    |

Tauri does not sign bundled resources, so the FFmpeg DLLs that ship next to `lanes.exe` must be signed before
`tauri build` packs them. Run this on the folder they are copied from:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/sign.ps1 -Path <folder with the DLLs>
```

Given a folder, the script signs every `.exe` and `.dll` in it, recursively. Signing a file that is already
signed replaces its signature, so it is safe to run twice.

## Development certificate

Use this to test signed builds and admin mode on your own machine. Run it from an elevated PowerShell:

```powershell
scripts\new-dev-cert.ps1          # create (or reuse) "CN=Lanes Dev Signing" and trust it on this PC
scripts\new-dev-cert.ps1 -Remove  # remove it from all stores again
```

The script:

1. Creates an RSA-3072/SHA-256 code-signing certificate valid for 2 years in `CurrentUser\My`. The private
   key cannot be exported.
2. Exports the public certificate to `scripts/lanes-dev-signing.cer` (gitignored).
3. Imports it into `LocalMachine\Root` and `LocalMachine\TrustedPublisher` on this machine only, so that
   `WinVerifyTrust` (and with it `signtool verify /pa` and Lanes' admin-mode check) accepts it.
4. Writes `scripts/signing.json` with `"mode": "thumbprint"` and the new thumbprint.

> **Never trust the dev certificate on any other machine.** Anyone holding its private key could sign
> software that machine would trust. It exists only to exercise the signed code paths locally.

To test admin mode without any certificate, a **debug** build accepts unsigned files when
`LANES_ALLOW_UNSIGNED_ADMIN=1` is set (and logs a warning). Release builds ignore the variable.

## Release certificates

Switching to a real certificate only means changing `signing.json` (or the CI environment variables). No code
changes are needed.

- **OV certificate** (on a hardware token or cloud HSM): import or attach it so it appears in
  `CurrentUser\My`, then use `mode: thumbprint`.
- **EV certificate**: same as OV. In the past EV gave immediate SmartScreen reputation. Since 2024
  Microsoft treats EV and OV the same for SmartScreen, so EV mostly adds cost.
- **Azure Trusted Signing**: install the *Trusted Signing Client Tools*, then create `metadata.json`:

  ```json
  { "Endpoint": "https://<region>.codesigning.azure.net", "CodeSigningAccountName": "<account>", "CertificateProfileName": "<profile>" }
  ```

  Set `mode: azure`, `dlibPath` (…`\bin\x64\Azure.CodeSigning.Dlib.dll`) and `metadataPath`. Authentication
  goes through `DefaultAzureCredential`: `az login` locally, or a service principal via `AZURE_CLIENT_ID` /
  `AZURE_TENANT_ID` / `AZURE_CLIENT_SECRET` in CI. The certificates are short-lived, which is why the RFC 3161
  timestamp is required.

Use the same signer identity for every release. Reputation is tied to it.

### Why admin mode skips revocation checks

`verify_signature` uses `WinVerifyTrust` with `WINTRUST_ACTION_GENERIC_VERIFY_V2`, `WTD_REVOKE_NONE` and
cache-only URL retrieval. Users must be able to turn admin mode on while offline. The check guards against a
swapped binary inside Program Files, and by the time someone could swap that binary they already have admin
rights. Revocation would protect against a stolen certificate, which is a different threat. The check reads
the embedded signature only. Catalog-signed files, such as most of `System32`, fail it, and that is fine
because `lanes.exe` always carries an embedded signature.

## Defender and SmartScreen hygiene

What we do, and what we never do:

- **No injection into games.** Capture uses Windows.Graphics.Capture/DXGI and WASAPI (process) loopback from
  our own process. We use no DLL injection, no graphics API hooks in other processes, no
  `CreateRemoteThread` and no `WriteProcessMemory`.
- **No process hollowing or self-modifying code.** The elevated instance is started by Task Scheduler as a
  normal process, with a separate path and arguments (`lanes.exe --from-task`).
- **No packers or UPX.** Binaries are plain MSVC/LTO output. Compressed or obfuscated executables are a
  common false-positive trigger.
- **The keyboard hook is a fallback only.** Hotkeys use `RegisterHotKey` first. A `WH_KEYBOARD_LL` hook is
  installed only while a binding needs it, never swallows keys, and is documented in the README.
- **Honest metadata.** The exe version resource carries product name, version, publisher and copyright from
  `tauri.conf.json` (`productName`, `version`, `bundle.publisher`, `bundle.copyright`). Keep these filled in.
- **Everything is signed**, including third-party DLLs we redistribute.
- **Admin rights are opt-in.** Lanes never asks for elevation on its own. The single UAC prompt comes from
  the user turning on admin mode.

### Reporting a false positive

If Microsoft Defender flags a release:

1. Compute the hash: `Get-FileHash -Algorithm SHA256 <file>`.
2. Submit the file at <https://www.microsoft.com/en-us/wdsi/filesubmission>. Choose **Software developer**,
   select "Incorrectly detected as malware/malicious", and include the SHA-256, the detection name, the
   download URL and a short description of what Lanes does (screen/audio capture, optional keyboard hook for
   hotkeys).
3. Once the verdict changes, users can update definitions (`Update-MpSignature`) and retry.

Report to other vendors through their own portals the same way, always with the SHA-256.

### SmartScreen reputation

SmartScreen warns about downloads ("Windows protected your PC") until the file or its signer has built up
reputation. Reputation grows with clean downloads and installs over time, and it is tracked per certificate
(signer identity) as well as per file hash. A new certificate or an unsigned file starts at zero. Keeping the
same certificate across releases and signing every file both help. Microsoft no longer grants EV certificates
instant reputation, so expect warnings on the first releases whichever certificate type you use.
