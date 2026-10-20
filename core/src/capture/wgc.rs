//! Windows.Graphics.Capture source. Uses a free-threaded frame pool, so frames
//! arrive on a system thread-pool thread without a dispatcher queue, and are
//! handed to the encoder as GPU textures.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface};

use super::{CaptureOptions, Frame, FrameSlot, Target};
use crate::Timestamp;
use crate::error::{Result, WinContext};
use crate::gpu::Gpu;

const POOL_BUFFERS: i32 = 2;

pub struct WgcCapture {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    item: GraphicsCaptureItem,
    frame_token: i64,
    closed_token: i64,
}

// SAFETY: WinRT capture objects are agile (free-threaded pool); they are only
// closed from Drop.
unsafe impl Send for WgcCapture {}

/// The WinRT device, moved into the frame callback.
struct AgileDevice(windows::Graphics::DirectX::Direct3D11::IDirect3DDevice);
// SAFETY: IDirect3DDevice wrappers are agile WinRT objects, callable from any thread.
unsafe impl Send for AgileDevice {}

impl AgileDevice {
    fn get(&self) -> &windows::Graphics::DirectX::Direct3D11::IDirect3DDevice {
        &self.0
    }
}

/// Keeps a WGC frame alive (and out of the pool) while the encoder uses its texture.
struct FrameHolder(#[allow(dead_code)] Direct3D11CaptureFrame);
// SAFETY: capture frames are agile WinRT objects.
unsafe impl Send for FrameHolder {}
unsafe impl Sync for FrameHolder {}

impl WgcCapture {
    pub fn is_supported() -> bool {
        GraphicsCaptureSession::IsSupported().unwrap_or(false)
    }

    pub fn start(gpu: &Gpu, target: &Target, opts: &CaptureOptions, slot: Arc<FrameSlot>) -> Result<Self> {
        if !Self::is_supported() {
            return Err(crate::Error::Unsupported("Windows.Graphics.Capture is not supported".into()));
        }
        let item = create_item(target)?;
        let size = item.Size().ctx("item size")?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &gpu.winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            POOL_BUFFERS,
            size,
        )
        .ctx("CreateFreeThreaded")?;

        let pool_w = Arc::new(AtomicU32::new(size.Width as u32));
        let pool_h = Arc::new(AtomicU32::new(size.Height as u32));
        let device = AgileDevice(gpu.winrt_device.clone());
        let frame_slot = slot.clone();
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |pool, _| {
            let Some(pool) = pool.as_ref() else { return Ok(()) };
            let frame = pool.TryGetNextFrame()?;
            let content = frame.ContentSize()?;
            // The window/monitor changed size: recreate the pool so following
            // frames are not cropped. This frame is still delivered as is.
            let resized = content.Width as u32 != pool_w.load(Ordering::Relaxed)
                || content.Height as u32 != pool_h.load(Ordering::Relaxed);
            if resized && content.Width > 0 && content.Height > 0 {
                pool.Recreate(device.get(), DirectXPixelFormat::B8G8R8A8UIntNormalized, POOL_BUFFERS, content)?;
                pool_w.store(content.Width as u32, Ordering::Relaxed);
                pool_h.store(content.Height as u32, Ordering::Relaxed);
            }
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            // SAFETY: the surface wraps a D3D11 texture on our device.
            let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
            // SystemRelativeTime is QPC-based, in 100 ns units: the shared clock.
            let time = Timestamp(frame.SystemRelativeTime()?.Duration);
            frame_slot.put(Frame {
                texture,
                array_index: 0,
                width: content.Width.max(1) as u32,
                height: content.Height.max(1) as u32,
                time,
                _owner: Some(Arc::new(FrameHolder(frame))),
            });
            Ok(())
        });
        let frame_token = pool.FrameArrived(&handler).ctx("FrameArrived")?;

        let closed_slot = slot.clone();
        let closed_token = item
            .Closed(&TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                closed_slot.close();
                Ok(())
            }))
            .ctx("Closed")?;

        let session = pool.CreateCaptureSession(&item).ctx("CreateCaptureSession")?;
        let _ = session.SetIsCursorCaptureEnabled(opts.cursor);
        // Windows 11: hide the yellow capture border. Older builds lack the property.
        let _ = session.SetIsBorderRequired(false);
        // Windows 11 24H2: let WGC itself drop frames above our rate.
        if opts.max_fps > 0 {
            let _ = session.SetMinUpdateInterval(TimeSpan { Duration: 10_000_000 / i64::from(opts.max_fps) });
        }
        session.StartCapture().ctx("StartCapture")?;
        log::info!("WGC capture started: {} ({}x{})", target.label(), size.Width, size.Height);
        Ok(Self { session, pool, item, frame_token, closed_token })
    }

    pub fn size(&self) -> (u32, u32) {
        self.item.Size().map(|s: SizeInt32| (s.Width as u32, s.Height as u32)).unwrap_or((0, 0))
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn create_item(target: &Target) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ctx("capture interop")?;
    // SAFETY: handles come from enumeration; invalid ones make the call fail cleanly.
    unsafe {
        match (target.hmonitor(), target.hwnd()) {
            (Some(m), _) => interop.CreateForMonitor(m).ctx("CreateForMonitor"),
            (_, Some(w)) => interop.CreateForWindow(w).ctx("CreateForWindow"),
            _ => unreachable!("a target is either a monitor or a window"),
        }
    }
}
