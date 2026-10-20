//! Thin safe wrappers over the Win32 calls the hotkey thread needs. Every
//! `unsafe` block in the hotkey code lives here.

use std::cell::Cell;

use lanes_core::Timestamp;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use super::Combo;

/// Posted after a command is queued on the channel.
pub const WM_APP_COMMAND: u32 = WM_APP + 1;
/// Posted by the hook callback: wParam = native id, lParam = QPC timestamp.
const WM_APP_HOOK_FIRED: u32 = WM_APP + 2;

/// Returns the calling thread's id after making sure it has a message queue,
/// so `PostThreadMessageW` from other threads cannot fail.
pub fn current_thread_id_with_queue() -> u32 {
    let mut msg = MSG::default();
    // SAFETY: valid out-pointer; PM_NOREMOVE leaves the (empty) queue unchanged.
    unsafe {
        let _ = PeekMessageW(&mut msg, None, WM_APP, WM_APP, PM_NOREMOVE);
        GetCurrentThreadId()
    }
}

/// Posts a message to a thread's queue. Returns false if the thread is gone.
pub fn post(thread_id: u32, msg: u32, wparam: usize, lparam: isize) -> bool {
    // SAFETY: plain value arguments; fails cleanly if the thread does not exist.
    unsafe { PostThreadMessageW(thread_id, msg, WPARAM(wparam), LPARAM(lparam)).is_ok() }
}

pub fn post_hook_fired(thread_id: u32, native_id: i32, at: Timestamp) {
    post(thread_id, WM_APP_HOOK_FIRED, native_id as usize, at.0 as isize);
}

pub enum Message {
    Hotkey(i32),
    HookFired(i32, Timestamp),
    Command,
    Other,
}

/// Blocks until the next message arrives. Returns `None` on `WM_QUIT` or error.
/// Low-level hook callbacks run inside this call.
pub fn next_message() -> Option<Message> {
    let mut msg = MSG::default();
    // SAFETY: valid out-pointer; a null HWND retrieves thread messages.
    let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
    if ret.0 == 0 || ret.0 == -1 {
        return None;
    }
    Some(match msg.message {
        WM_HOTKEY => Message::Hotkey(msg.wParam.0 as i32),
        WM_APP_HOOK_FIRED => Message::HookFired(msg.wParam.0 as i32, Timestamp(msg.lParam.0 as i64)),
        WM_APP_COMMAND => Message::Command,
        _ => Message::Other,
    })
}

/// Registers a thread-level hotkey (null HWND, so `WM_HOTKEY` goes to this thread's queue).
pub fn register_hotkey(native_id: i32, combo: &Combo) -> windows::core::Result<()> {
    let mut mods = MOD_NOREPEAT;
    for (on, flag) in [(combo.ctrl, MOD_CONTROL), (combo.alt, MOD_ALT), (combo.shift, MOD_SHIFT), (combo.win, MOD_WIN)] {
        if on {
            mods |= flag;
        }
    }
    // SAFETY: plain value arguments; must be called on the thread that will pump WM_HOTKEY.
    unsafe { RegisterHotKey(None, native_id, HOT_KEY_MODIFIERS(mods.0), combo.vk as u32) }
}

pub fn unregister_hotkey(native_id: i32) {
    // SAFETY: plain value arguments; unregistering an unknown id is harmless.
    let _ = unsafe { UnregisterHotKey(None, native_id) };
}

/// Whether a key is physically down right now. Only used to seed modifier
/// state when the hook is installed: inside a low-level hook callback the
/// async key state lags behind and must not be relied on.
pub fn key_down(vk: u16) -> bool {
    // SAFETY: no pointers involved. The high bit (sign) means "currently down".
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// A key transition delivered to the hook callback.
pub struct KeyEvent {
    pub vk: u16,
    pub pressed: bool,
}

thread_local! {
    static HOOK_CALLBACK: Cell<Option<fn(KeyEvent)>> = const { Cell::new(None) };
}

/// An installed `WH_KEYBOARD_LL` hook, removed on drop. It must be installed
/// and dropped on a thread that pumps messages.
pub struct Hook(HHOOK);

impl Hook {
    pub fn install(callback: fn(KeyEvent)) -> windows::core::Result<Self> {
        HOOK_CALLBACK.set(Some(callback));
        // SAFETY: `hook_proc` matches HOOKPROC and lives for the whole program;
        // the module handle of our own exe is always valid.
        let hook = unsafe {
            let module = GetModuleHandleW(None)?;
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(HINSTANCE(module.0)), 0)?
        };
        Ok(Self(hook))
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: the handle came from SetWindowsHookExW and is unhooked exactly once.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
        HOOK_CALLBACK.set(None);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let pressed = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        if let (Some(pressed), Some(callback)) = (pressed, HOOK_CALLBACK.get()) {
            // SAFETY: for HC_ACTION, lParam points to a KBDLLHOOKSTRUCT valid for this call.
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            callback(KeyEvent { vk: info.vkCode as u16, pressed });
        }
    }
    // Never swallow the key: the game must still receive it.
    // SAFETY: forwards the exact arguments we were given.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
