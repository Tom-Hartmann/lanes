//! Authenticode verification with `WinVerifyTrust`.

use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL,
    WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::core::{GUID, HRESULT, PCWSTR};

use super::wide;

/// Verifies the file's *embedded* Authenticode signature and returns the
/// signer's display name (the certificate subject CN).
///
/// Policy: `WINTRUST_ACTION_GENERIC_VERIFY_V2` (chain must end in a trusted
/// root, certificate valid for code signing, file hash must match), no
/// revocation checks (`WTD_REVOKE_NONE`) and no network fetches
/// (`WTD_CACHE_ONLY_URL_RETRIEVAL`). Lanes must be able to set up admin mode
/// offline, and the check guards against a *swapped* binary in an
/// admin-only directory rather than against a stolen certificate, which
/// revocation would cover. Catalog-signed files (most of `System32`) have no
/// embedded signature and fail this check.
pub fn verify_embedded(path: &Path) -> windows::core::Result<String> {
    let file_path = wide(path);
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(file_path.as_ptr()),
        hFile: HANDLE::default(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;

    // SAFETY: `data`, `file`, `file_path` and `action` outlive both calls; a
    // VERIFY call is always followed by a CLOSE call below to free the state.
    let status = unsafe { WinVerifyTrust(HWND(INVALID_HANDLE_VALUE.0), &mut action, (&raw mut data).cast()) };
    let signer = if status == 0 { signer_name(data.hWVTStateData) } else { None };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: as above; releases hWVTStateData.
    let _ = unsafe { WinVerifyTrust(HWND(INVALID_HANDLE_VALUE.0), &mut action, (&raw mut data).cast()) };

    HRESULT(status).ok()?;
    Ok(signer.unwrap_or_else(|| "<unknown signer>".to_owned()))
}

/// Reads the leaf certificate's display name from WinVerifyTrust state data.
fn signer_name(state: HANDLE) -> Option<String> {
    // SAFETY: `state` is live WinVerifyTrust state (not yet closed); each
    // returned pointer is checked for null before use and stays valid until
    // the state is closed, which happens after this function returns.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return None;
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
        if signer.is_null() {
            return None;
        }
        let cert = WTHelperGetProvCertFromChain(signer, 0);
        if cert.is_null() || (*cert).pCert.is_null() {
            return None;
        }
        let pcert = (*cert).pCert;
        let len = CertGetNameStringW(pcert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None);
        if len <= 1 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        let written = CertGetNameStringW(pcert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf));
        let text = buf.get(..(written as usize).saturating_sub(1))?;
        Some(String::from_utf16_lossy(text))
    }
}
