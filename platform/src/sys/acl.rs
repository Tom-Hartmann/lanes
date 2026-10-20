//! Reads who owns a file or folder and who may modify it.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION,
    GetAce, GetAclInformation, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows::core::PWSTR;

use super::wide;

/// Access bits that let someone change or replace the file or folder content.
const WRITE_BITS: u32 = 0x0000_0002 // FILE_WRITE_DATA / FILE_ADD_FILE
    | 0x0000_0004 // FILE_APPEND_DATA / FILE_ADD_SUBDIRECTORY
    | 0x0000_0040 // FILE_DELETE_CHILD
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x1000_0000 // GENERIC_ALL
    | 0x4000_0000; // GENERIC_WRITE

const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

/// Owner SID and the SIDs granted any write-like access (as SID strings).
/// A missing (NULL) DACL means everyone has full access: reported as "S-1-1-0".
pub fn owner_and_writers(path: &Path) -> windows::core::Result<(String, Vec<String>)> {
    let name = wide(path);
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: valid NUL-terminated path and out-pointers; the security
    // descriptor (which owner and dacl point into) is freed with LocalFree below.
    unsafe {
        let rc = GetNamedSecurityInfoW(
            windows::core::PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        );
        if rc != ERROR_SUCCESS {
            return Err(windows::core::Error::from_hresult(rc.to_hresult()));
        }
        let result = (|| {
            let owner_sid = sid_string(owner)?;
            let mut writers = Vec::new();
            if dacl.is_null() {
                writers.push("S-1-1-0".to_owned());
                return Ok((owner_sid, writers));
            }
            let mut info = ACL_SIZE_INFORMATION::default();
            GetAclInformation(dacl, &mut info as *mut _ as *mut _, size_of::<ACL_SIZE_INFORMATION>() as u32, AclSizeInformation)?;
            for i in 0..info.AceCount {
                let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
                GetAce(dacl, i, &mut ace)?;
                let header = &*(ace as *const ACE_HEADER);
                // Inherit-only entries apply to children, not to this object.
                if header.AceType != ACCESS_ALLOWED_ACE_TYPE || header.AceFlags & INHERIT_ONLY_ACE.0 as u8 != 0 {
                    continue;
                }
                let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
                if allowed.Mask & WRITE_BITS != 0 {
                    writers.push(sid_string(PSID(&allowed.SidStart as *const u32 as *mut _))?);
                }
            }
            Ok((owner_sid, writers))
        })();
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        result
    }
}

fn sid_string(sid: PSID) -> windows::core::Result<String> {
    let mut out = PWSTR::null();
    // SAFETY: `sid` points into a live security descriptor; the string is freed with LocalFree.
    unsafe {
        ConvertSidToStringSidW(sid, &mut out)?;
        let s = out.to_string().unwrap_or_default();
        let _ = LocalFree(Some(HLOCAL(out.0 as _)));
        Ok(s)
    }
}
