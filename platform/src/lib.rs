//! Windows integration for Lanes: autostart, the elevated logon task and
//! signature checks. No dependency on Tauri or the capture engine.
//!
//! The public modules hold the policy (what is allowed, which arguments, which
//! task settings) as plain, testable Rust. Every `unsafe` Win32 call lives in
//! the private `sys` module or in `admin::scheduler`, each in a small wrapper
//! with its own `// SAFETY:` note.

pub mod admin;
pub mod autostart;
pub mod os;

mod paths;
mod sys;
