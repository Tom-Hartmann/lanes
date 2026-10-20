//! Engine error type. Capture and encode threads never panic: they return
//! these, and the engine reports them to the UI and recovers where possible.

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A Win32/COM/WinRT call failed.
    #[error("{context}: {source}")]
    Windows { context: &'static str, source: windows::core::Error },

    /// The GPU was reset, removed or the driver crashed. Everything on the
    /// device must be recreated.
    #[error("GPU device lost ({0})")]
    DeviceLost(&'static str),

    /// An FFmpeg call failed.
    #[error("{context}: {message} (code {code})")]
    Ffmpeg { context: &'static str, code: i32, message: String },

    #[error("{0}")]
    Io(#[from] std::io::Error),

    /// Feature not available on this system (OS build, GPU, encoder...).
    #[error("{0}")]
    Unsupported(String),

    /// Invalid configuration or request.
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// True if recreating the pipeline is likely to fix it.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Error::DeviceLost(_) | Error::Windows { .. } | Error::Ffmpeg { .. } | Error::Io(_))
    }
}

/// Adds context to `windows::core::Result`, classifying device-removed errors.
pub trait WinContext<T> {
    fn ctx(self, context: &'static str) -> Result<T>;
}

impl<T> WinContext<T> for windows::core::Result<T> {
    fn ctx(self, context: &'static str) -> Result<T> {
        self.map_err(|source| {
            use windows::Win32::Graphics::Dxgi::{
                DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
                DXGI_ERROR_DRIVER_INTERNAL_ERROR,
            };
            let code = source.code();
            if code == DXGI_ERROR_DEVICE_REMOVED
                || code == DXGI_ERROR_DEVICE_RESET
                || code == DXGI_ERROR_DEVICE_HUNG
                || code == DXGI_ERROR_DRIVER_INTERNAL_ERROR
            {
                Error::DeviceLost(context)
            } else {
                Error::Windows { context, source }
            }
        })
    }
}

/// Converts a negative FFmpeg return code into an error.
pub fn av_check(code: i32, context: &'static str) -> Result<i32> {
    if code >= 0 { Ok(code) } else { Err(Error::Ffmpeg { context, code, message: av_error_string(code) }) }
}

pub fn av_error_string(code: i32) -> String {
    let mut buf = [0 as std::ffi::c_char; 256];
    // SAFETY: buf is writable for its full length; av_strerror always NUL-terminates.
    unsafe {
        ffmpeg_next::ffi::av_strerror(code, buf.as_mut_ptr(), buf.len());
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Where an error came from, for UI reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Capture,
    Audio,
    Encoder,
    Recorder,
    Replay,
    Clip,
    Engine,
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Component::Capture => "capture",
            Component::Audio => "audio",
            Component::Encoder => "encoder",
            Component::Recorder => "recorder",
            Component::Replay => "replay",
            Component::Clip => "clip",
            Component::Engine => "engine",
        })
    }
}
