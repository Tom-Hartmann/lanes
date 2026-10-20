//! Video capture sources. Each source delivers its newest frame as a GPU
//! texture into a [`FrameSlot`]; the encoder thread samples the slot at the
//! target frame rate. Sources never block on the encoder and never copy
//! frames to system memory.

pub mod duplication;
pub mod monitors;
pub mod thumbnail;
pub mod wgc;
pub mod windows_list;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Gdi::HMONITOR;

use crate::Timestamp;
use crate::config::CaptureApi;
use crate::error::Result;
use crate::gpu::Gpu;

/// A captured frame living on the GPU.
#[derive(Clone)]
pub struct Frame {
    pub texture: ID3D11Texture2D,
    /// Array slice of `texture` holding the frame.
    pub array_index: u32,
    /// The valid region (top-left aligned); may be smaller than the texture.
    pub width: u32,
    pub height: u32,
    pub time: Timestamp,
    /// Keeps the source's own frame object (e.g. a WGC frame) alive while in use.
    pub(crate) _owner: Option<Arc<dyn Send + Sync>>,
}

// SAFETY: D3D11 textures are free-threaded COM objects; the device's immediate
// context is multithread-protected (see gpu.rs).
unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}

/// Single-producer "latest frame" slot. Writing replaces the previous frame,
/// which returns it to the source's pool.
#[derive(Default)]
pub struct FrameSlot {
    frame: Mutex<Option<Frame>>,
    seq: AtomicU64,
    closed: AtomicBool,
}

impl FrameSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn put(&self, frame: Frame) {
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
        self.seq.fetch_add(1, Ordering::Release);
    }

    /// The newest frame and its sequence number (which increases with each new frame).
    pub fn latest(&self) -> Option<(Frame, u64)> {
        let seq = self.seq.load(Ordering::Acquire);
        self.frame.lock().unwrap_or_else(|e| e.into_inner()).clone().map(|f| (f, seq))
    }

    /// Marks the source as ended (window closed, monitor unplugged...).
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// What to capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Monitor { hmonitor: isize, name: String },
    Window { hwnd: isize, name: String },
}

impl Target {
    pub fn label(&self) -> &str {
        match self {
            Target::Monitor { name, .. } | Target::Window { name, .. } => name,
        }
    }

    pub(crate) fn hmonitor(&self) -> Option<HMONITOR> {
        match self {
            Target::Monitor { hmonitor, .. } => Some(HMONITOR(*hmonitor as _)),
            Target::Window { .. } => None,
        }
    }

    pub(crate) fn hwnd(&self) -> Option<HWND> {
        match self {
            Target::Window { hwnd, .. } => Some(HWND(*hwnd as _)),
            Target::Monitor { .. } => None,
        }
    }
}

/// Options shared by all capture back ends.
#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub cursor: bool,
    /// Frame-rate cap hint; WGC honours it on Windows 11 24H2+.
    pub max_fps: u32,
}

/// A running capture. Dropping it stops the capture.
pub enum Capture {
    Wgc(wgc::WgcCapture),
    Duplication(duplication::DuplicationCapture),
}

impl Capture {
    /// Starts capturing `target` into `slot` with the requested API, falling
    /// back from WGC to Desktop Duplication for monitors.
    pub fn start(gpu: &Gpu, target: &Target, api: CaptureApi, opts: &CaptureOptions, slot: Arc<FrameSlot>) -> Result<Self> {
        let try_wgc = matches!(api, CaptureApi::Auto | CaptureApi::Wgc);
        if try_wgc {
            match wgc::WgcCapture::start(gpu, target, opts, slot.clone()) {
                Ok(c) => return Ok(Capture::Wgc(c)),
                Err(e) if api == CaptureApi::Auto && target.hmonitor().is_some() => {
                    log::warn!("WGC capture of {} failed ({e}); using Desktop Duplication", target.label());
                }
                Err(e) => return Err(e),
            }
        }
        let Some(hmonitor) = target.hmonitor() else {
            return Err(crate::Error::Unsupported("Desktop Duplication can only capture monitors".into()));
        };
        Ok(Capture::Duplication(duplication::DuplicationCapture::start(gpu, hmonitor, opts, slot)?))
    }

    pub fn api_name(&self) -> &'static str {
        match self {
            Capture::Wgc(_) => "WGC",
            Capture::Duplication(_) => "Desktop Duplication",
        }
    }
}
