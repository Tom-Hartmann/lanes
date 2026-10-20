//! The shared D3D11 device. Capture, colour conversion and hardware encoding
//! all run on this one device, so frames never leave the GPU.

use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
    ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_DESC1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::Interface;

use crate::error::{Result, WinContext};
use crate::util::from_wide;

/// PCI vendor ids, used to pick the matching hardware encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl GpuVendor {
    fn from_id(id: u32) -> Self {
        match id {
            0x10DE => GpuVendor::Nvidia,
            0x1002 | 0x1022 => GpuVendor::Amd,
            0x8086 => GpuVendor::Intel,
            _ => GpuVendor::Other,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub name: String,
    pub vendor: GpuVendor,
    pub dedicated_vram_mb: u64,
    /// LUID, to match the adapter a monitor is attached to.
    #[serde(skip)]
    pub luid: (u32, i32),
}

/// A D3D11 device with video support, safe to share across threads.
#[derive(Clone)]
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// The same device as a WinRT object, for Windows.Graphics.Capture.
    pub winrt_device: IDirect3DDevice,
    pub adapter: AdapterInfo,
}

// SAFETY: the device is created free-threaded and the immediate context is
// protected by ID3D11Multithread::SetMultithreadProtected(TRUE) below, so all
// calls on it are serialised by the runtime.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

impl Gpu {
    /// Creates the device on the given adapter (by LUID) or the default adapter.
    pub fn new(adapter_luid: Option<(u32, i32)>) -> Result<Self> {
        let adapter = match adapter_luid {
            Some(luid) => find_adapter(luid)?,
            None => None,
        };
        let mut device = None;
        let mut context = None;
        let levels: [D3D_FEATURE_LEVEL; 2] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        // SAFETY: valid out-pointers; adapter is either None (default) or a live adapter.
        unsafe {
            D3D11CreateDevice(
                adapter.as_ref().map(|a: &IDXGIAdapter1| a.cast::<IDXGIAdapter>()).transpose().ctx("adapter cast")?.as_ref(),
                if adapter.is_some() { D3D_DRIVER_TYPE_UNKNOWN } else { windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE },
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .ctx("D3D11CreateDevice")?;
        }
        let device: ID3D11Device = device.ok_or_else(|| crate::Error::Unsupported("no D3D11 device".into()))?;
        let context: ID3D11DeviceContext = context.ok_or_else(|| crate::Error::Unsupported("no D3D11 context".into()))?;

        // Capture callbacks, the encoder and FFmpeg touch the immediate context
        // from different threads.
        let mt: ID3D11Multithread = context.cast().ctx("ID3D11Multithread")?;
        // SAFETY: plain flag setter on a live interface.
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }

        let dxgi: IDXGIDevice = device.cast().ctx("IDXGIDevice")?;
        // SAFETY: valid DXGI device; returns an owned WinRT wrapper.
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.ctx("CreateDirect3D11DeviceFromDXGIDevice")?;
        let winrt_device: IDirect3DDevice = inspectable.cast().ctx("IDirect3DDevice")?;

        // SAFETY: valid DXGI device.
        let adapter = unsafe { dxgi.GetAdapter() }.ctx("GetAdapter")?;
        let adapter: IDXGIAdapter1 = adapter.cast().ctx("IDXGIAdapter1")?;
        let info = adapter_info(&adapter)?;
        log::info!("GPU: {} ({:?}, {} MB)", info.name, info.vendor, info.dedicated_vram_mb);
        Ok(Self { device, context, winrt_device, adapter: info })
    }

    /// Fails with `DeviceLost` if the device was removed or reset.
    pub fn check_alive(&self) -> Result<()> {
        // SAFETY: simple query on a live device.
        unsafe { self.device.GetDeviceRemovedReason() }.ctx("device check")
    }
}

fn adapter_info(adapter: &IDXGIAdapter1) -> Result<AdapterInfo> {
    // SAFETY: simple query on a live adapter.
    let desc: DXGI_ADAPTER_DESC1 = unsafe { adapter.GetDesc1() }.ctx("GetDesc1")?;
    Ok(AdapterInfo {
        name: from_wide(&desc.Description),
        vendor: GpuVendor::from_id(desc.VendorId),
        dedicated_vram_mb: desc.DedicatedVideoMemory as u64 / (1024 * 1024),
        luid: (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart),
    })
}

fn find_adapter(luid: (u32, i32)) -> Result<Option<IDXGIAdapter1>> {
    // SAFETY: factory creation and enumeration with valid indices.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ctx("CreateDXGIFactory1")?;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            let info = adapter_info(&adapter)?;
            if info.luid == luid {
                return Ok(Some(adapter));
            }
            i += 1;
        }
    }
    Ok(None)
}

/// All hardware adapters, for encoder detection and diagnostics.
pub fn adapters() -> Vec<AdapterInfo> {
    let mut out = Vec::new();
    // SAFETY: factory creation and enumeration with valid indices.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            if let Ok(info) = adapter_info(&adapter) {
                // Skip the software "Microsoft Basic Render Driver".
                if info.vendor != GpuVendor::Other || !info.name.contains("Basic Render") {
                    out.push(info);
                }
            }
            i += 1;
        }
    }
    out
}
