//! The on-screen "clip saved" notice: a tiny native popup in a corner of the
//! game's monitor. It is topmost, click-through and never activated, so the
//! game keeps focus and input; it fades out after a few seconds. Drawn with
//! GDI on its own short-lived thread (no WebView, ~0 cost).
//!
//! Like any overlay that does not inject into the game, it shows over windowed
//! and borderless games, not over exclusive fullscreen.

use std::cell::{Cell, RefCell};
use std::sync::Once;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetTextExtentPoint32W, HGDIOBJ,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow, PAINTSTRUCT, ReleaseDC, SelectObject,
    SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT, FW_SEMIBOLD, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
    CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, FF_DONTCARE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HTTRANSPARENT, KillTimer, LWA_ALPHA,
    MSG, PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE, SetLayeredWindowAttributes, SetTimer, ShowWindow,
    TranslateMessage, WM_DESTROY, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::settings::OverlayCorner;

const CLASS: PCWSTR = w!("LanesClipNotice");
const SHOW_MS: u32 = 2600;
const FADE_STEP_MS: u32 = 30;
const MAX_ALPHA: u8 = 235;

thread_local! {
    static TEXT: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    static FONT: Cell<isize> = const { Cell::new(0) };
    static ALPHA: Cell<u8> = const { Cell::new(MAX_ALPHA) };
    static PAD: Cell<i32> = const { Cell::new(12) };
}

/// Shows `text` in `corner` of the monitor holding `near` (a window, e.g. the
/// game) or of the primary monitor. Returns immediately.
pub fn show(text: String, corner: OverlayCorner, near: Option<isize>) {
    let spawned = std::thread::Builder::new().name("lanes-overlay".into()).spawn(move || {
        // SAFETY: all Win32 calls below happen on this thread, which owns the
        // window and its message loop; handles are released before returning.
        unsafe { run(&text, corner, near) };
    });
    if let Err(e) = spawned {
        log::warn!("overlay thread: {e}");
    }
}

unsafe fn run(text: &str, corner: OverlayCorner, near: Option<isize>) {
    // SAFETY (whole fn): see `show`.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module.into(), lpszClassName: CLASS, ..Default::default() };
            RegisterClassW(&class);
        });

        // Target monitor, its work area and DPI scale.
        let monitor = match near {
            Some(h) => MonitorFromWindow(HWND(h as _), MONITOR_DEFAULTTOPRIMARY),
            None => MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
        };
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
            return;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let px = |v: f32| (v * scale).round() as i32;

        let font = CreateFontW(
            -px(17.0), 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY, FF_DONTCARE.0 as u32, w!("Segoe UI"),
        );
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        let dc = GetDC(None);
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
        SelectObject(dc, old);
        ReleaseDC(None, dc);

        let pad = px(14.0);
        let accent = px(4.0);
        let (w, h) = (size.cx + pad * 2 + accent, size.cy + pad * 2 - px(4.0));
        let margin = px(24.0);
        let work: RECT = mi.rcWork;
        let x = match corner {
            OverlayCorner::TopLeft | OverlayCorner::BottomLeft => work.left + margin,
            OverlayCorner::TopRight | OverlayCorner::BottomRight => work.right - margin - w,
        };
        let y = match corner {
            OverlayCorner::TopLeft | OverlayCorner::TopRight => work.top + margin,
            OverlayCorner::BottomLeft | OverlayCorner::BottomRight => work.bottom - margin - h,
        };

        TEXT.with_borrow_mut(|t| *t = wide);
        FONT.set(font.0 as isize);
        ALPHA.set(MAX_ALPHA);
        PAD.set(pad + accent);

        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Lanes"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            let _ = DeleteObject(HGDIOBJ(font.0));
            return;
        };
        let _ = SetWindowRgn(hwnd, Some(CreateRoundRectRgn(0, 0, w + 1, h + 1, px(10.0), px(10.0))), true);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), MAX_ALPHA, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), 1, SHOW_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: called by the system on the overlay thread with a valid window.
    unsafe {
        match msg {
            // Click-through: the mouse goes to the game underneath.
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
                let bg = CreateSolidBrush(COLORREF(0x001B_1818)); // zinc-900-ish (BGR)
                FillRect(dc, &rc, bg);
                let _ = DeleteObject(HGDIOBJ(bg.0));
                let accent = CreateSolidBrush(COLORREF(0x00C7_8402)); // sky-600 (BGR)
                let bar = RECT { left: 0, top: 0, right: PAD.get() / 4, bottom: rc.bottom };
                FillRect(dc, &bar, accent);
                let _ = DeleteObject(HGDIOBJ(accent.0));
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00FA_FAFA));
                let old = SelectObject(dc, HGDIOBJ(FONT.get() as _));
                let mut text_rc = RECT { left: PAD.get(), ..rc };
                TEXT.with_borrow_mut(|t| {
                    DrawTextW(dc, t, &mut text_rc, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX);
                });
                SelectObject(dc, old);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == 1 {
                    let _ = KillTimer(Some(hwnd), 1);
                    SetTimer(Some(hwnd), 2, FADE_STEP_MS, None);
                } else {
                    let a = ALPHA.get().saturating_sub(24);
                    ALPHA.set(a);
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), a, LWA_ALPHA);
                    if a == 0 {
                        let _ = KillTimer(Some(hwnd), 2);
                        let _ = DestroyWindow(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
