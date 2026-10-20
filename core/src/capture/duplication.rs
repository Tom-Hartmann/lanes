//! DXGI Desktop Duplication fallback for monitors (older Windows builds, or
//! when WGC is unavailable). Runs on its own capture thread; each new desktop
//! image is copied GPU-side into one of two textures so the duplication frame
//! can be released immediately. The mouse cursor is not drawn in this mode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIDevice, IDXGIOutput1,
    IDXGIOutputDuplication,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::core::Interface;

use super::{CaptureOptions, Frame, FrameSlot};
use crate::Timestamp;
use crate::error::{Error, Result, WinContext};
use crate::gpu::Gpu;
use crate::util::{Priority, spawn};

pub struct DuplicationCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Option<()>>>,
}

impl DuplicationCapture {
    pub fn start(gpu: &Gpu, monitor: HMONITOR, _opts: &CaptureOptions, slot: Arc<FrameSlot>) -> Result<Self> {
        // Fail fast (so the caller can report it) if this output cannot be duplicated.
        let first = open(gpu, monitor)?;
        let stop = Arc::new(AtomicBool::new(false));
        let gpu = gpu.clone();
        let monitor_raw = monitor.0 as isize;
        let stop_flag = stop.clone();
        let thread = spawn("lanes-dup-capture", Priority::AboveNormal, move || {
            let monitor = HMONITOR(monitor_raw as _);
            let mut dup = Some(first);
            let mut copies: Vec<ID3D11Texture2D> = Vec::new();
            let mut next_copy = 0usize;
            while !stop_flag.load(Ordering::Relaxed) {
                let d = match dup.take() {
                    Some(d) => d,
                    None => match open(&gpu, monitor) {
                        Ok(d) => d,
                        Err(e) => {
                            // Secure desktop (UAC, Ctrl+Alt+Del) or mode change: retry.
                            log::debug!("duplication reopen failed: {e}");
                            std::thread::sleep(Duration::from_millis(250));
                            continue;
                        }
                    },
                };
                match acquire(&gpu, &d, &mut copies, &mut next_copy) {
                    Ok(Some(frame)) => slot.put(frame),
                    Ok(None) => {}
                    Err(Error::DeviceLost(_)) => {
                        slot.close();
                        break;
                    }
                    Err(_) => {
                        // ACCESS_LOST and friends: recreate the duplication.
                        copies.clear();
                        continue;
                    }
                }
                dup = Some(d);
            }
        })?;
        Ok(Self { stop, thread: Some(thread) })
    }
}

impl Drop for DuplicationCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn open(gpu: &Gpu, monitor: HMONITOR) -> Result<IDXGIOutputDuplication> {
    let dxgi: IDXGIDevice = gpu.device.cast().ctx("IDXGIDevice")?;
    // SAFETY: enumeration with valid indices on a live adapter.
    unsafe {
        let adapter = dxgi.GetAdapter().ctx("GetAdapter")?;
        let mut i = 0;
        while let Ok(output) = adapter.EnumOutputs(i) {
            i += 1;
            let desc = output.GetDesc().ctx("output desc")?;
            if desc.Monitor == monitor {
                let output1: IDXGIOutput1 = output.cast().ctx("IDXGIOutput1")?;
                return output1.DuplicateOutput(&gpu.device).ctx("DuplicateOutput");
            }
        }
    }
    Err(Error::Unsupported("the monitor is not attached to the capture GPU".into()))
}

/// Waits up to 100 ms for a new desktop image. Returns `None` on timeout.
fn acquire(
    gpu: &Gpu,
    dup: &IDXGIOutputDuplication,
    copies: &mut Vec<ID3D11Texture2D>,
    next: &mut usize,
) -> Result<Option<Frame>> {
    let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource = None;
    // SAFETY: valid out-pointers; every successful acquire is paired with ReleaseFrame.
    unsafe {
        if let Err(e) = dup.AcquireNextFrame(100, &mut info, &mut resource) {
            if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                return Ok(None);
            }
            if e.code() == DXGI_ERROR_ACCESS_LOST {
                return Err(Error::Windows { context: "duplication access lost", source: e });
            }
            return Err(e).ctx("AcquireNextFrame");
        }
        let result = (|| -> Result<Option<Frame>> {
            // LastPresentTime == 0 means only the cursor moved.
            if info.LastPresentTime == 0 {
                return Ok(None);
            }
            let Some(resource) = resource else { return Ok(None) };
            let src: ID3D11Texture2D = resource.cast().ctx("duplication texture")?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            src.GetDesc(&mut desc);
            if copies.first().is_none_or(|t| {
                let mut d = D3D11_TEXTURE2D_DESC::default();
                t.GetDesc(&mut d);
                d.Width != desc.Width || d.Height != desc.Height
            }) {
                copies.clear();
                for _ in 0..2 {
                    copies.push(make_copy_texture(gpu, desc.Width, desc.Height)?);
                }
            }
            let dst = copies[*next % copies.len()].clone();
            *next += 1;
            gpu.context.CopyResource(&dst, &src);
            Ok(Some(Frame {
                texture: dst,
                array_index: 0,
                width: desc.Width,
                height: desc.Height,
                time: Timestamp::from_qpc(info.LastPresentTime),
                _owner: None,
            }))
        })();
        let _ = dup.ReleaseFrame();
        result
    }
}

fn make_copy_texture(gpu: &Gpu, width: u32, height: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        ..Default::default()
    };
    let mut tex = None;
    // SAFETY: valid descriptor and out-pointer.
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut tex)) }.ctx("CreateTexture2D")?;
    tex.ok_or_else(|| Error::Unsupported("texture creation returned nothing".into()))
}
