//! Getting clips out of Lanes: native drag & drop (into Discord, Explorer,
//! browser uploads...) and copying the file to the clipboard (Ctrl+V in
//! Discord uploads it).

use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::{
    BHID_DataObject, DROPFILES, IShellItem, IShellItemArray, SHCreateItemFromParsingName, SHCreateShellItemArrayFromShellItem,
};
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
use windows::core::{BOOL, HRESULT, HSTRING, implement};

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Starts a native drag of `path` (copy). Must run on the UI (main) thread
/// while the left mouse button is held; returns when the user drops or cancels.
pub fn drag_file(path: &Path) -> windows::core::Result<bool> {
    // SAFETY: called on the main STA thread; the data object and drop source
    // are live COM objects for the whole (modal) DoDragDrop call.
    unsafe {
        let _ = OleInitialize(None);
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
        let array: IShellItemArray = SHCreateShellItemArrayFromShellItem(&item)?;
        let data: IDataObject = array.BindToHandler(None, &BHID_DataObject)?;
        let source: IDropSource = DropSource.into();
        let mut effect = DROPEFFECT::default();
        let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
        Ok(hr == DRAGDROP_S_DROP)
    }
}

/// Puts `path` on the clipboard as a file (like Ctrl+C in Explorer).
pub fn copy_file(path: &Path) -> Result<(), String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide_with_nul();
    let header = size_of::<DROPFILES>();
    let bytes = header + (wide.len() + 1) * 2; // list ends with an extra NUL
    // SAFETY: the global block is sized for the header plus the
    // double-NUL-terminated path; ownership passes to the clipboard on success.
    unsafe {
        let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            return Err("GlobalLock failed".into());
        }
        std::ptr::write_bytes(ptr, 0, bytes);
        let drop = ptr as *mut DROPFILES;
        (*drop).pFiles = header as u32;
        (*drop).fWide = true.into();
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr.add(header) as *mut u16, wide.len());
        let _ = GlobalUnlock(mem);
        OpenClipboard(None).map_err(|e| format!("clipboard busy: {e}"))?;
        let _ = EmptyClipboard();
        let set = SetClipboardData(u32::from(CF_HDROP.0), Some(HANDLE(mem.0)));
        let _ = CloseClipboard();
        set.map(|_| ()).map_err(|e| e.to_string())
    }
}

trait EncodeWide {
    fn encode_wide_with_nul(&self) -> Vec<u16>;
}

impl EncodeWide for std::ffi::OsStr {
    fn encode_wide_with_nul(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}
