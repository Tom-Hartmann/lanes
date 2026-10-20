//! Small monitor previews for the Sources page. Only used while that page is
//! open; this is the one place where a captured frame is read back to the CPU.

use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use super::{Capture, CaptureOptions, FrameSlot, Target};
use crate::config::CaptureApi;
use crate::convert::{Converter, Layer, OutputFormat, Rect, make_texture, scaled_size};
use crate::error::{Error, Result, WinContext};
use crate::gpu::Gpu;

/// Captures one frame of `target` and returns it as a BMP data URL, `width` pixels wide.
pub fn grab(gpu: &Gpu, target: &Target, width: u32) -> Result<String> {
    let slot = FrameSlot::new();
    let opts = CaptureOptions { cursor: false, max_fps: 0 };
    let capture = Capture::start(gpu, target, CaptureApi::Auto, &opts, Arc::clone(&slot))?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    let frame = loop {
        if let Some((f, _)) = slot.latest() {
            break f;
        }
        if Instant::now() > deadline {
            return Err(Error::Unsupported("no frame arrived for the thumbnail".into()));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    drop(capture);

    let (w, h) = scaled_size(frame.width, frame.height, Some((u64::from(width) * u64::from(frame.height) / u64::from(frame.width.max(1))) as u32));
    let target_tex = make_texture(gpu, w, h)?;
    let mut conv = Converter::new(gpu, w, h, OutputFormat::Bgra)?;
    conv.convert(&[Layer { frame: &frame, dest: Rect::full(w, h) }], &target_tex, 0)?;
    let pixels = read_bgra(gpu, &target_tex, w, h)?;
    Ok(format!("data:image/bmp;base64,{}", base64(&bmp(w, h, &pixels))))
}

fn read_bgra(gpu: &Gpu, tex: &ID3D11Texture2D, w: u32, h: u32) -> Result<Vec<u8>> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..Default::default()
    };
    let mut staging = None;
    // SAFETY: valid descriptor/out-pointers; the mapping is read within its
    // row pitch and unmapped before returning.
    unsafe {
        gpu.device.CreateTexture2D(&desc, None, Some(&mut staging)).ctx("staging texture")?;
        let staging = staging.ok_or_else(|| Error::Unsupported("no staging texture".into()))?;
        gpu.context.CopyResource(&staging, tex);
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).ctx("Map")?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts((map.pData as *const u8).add(y * map.RowPitch as usize), w as usize * 4);
            out.extend_from_slice(row);
        }
        gpu.context.Unmap(&staging, 0);
        Ok(out)
    }
}

/// Encodes top-down BGRA pixels as a 32-bit BMP.
pub fn bmp(w: u32, h: u32, bgra: &[u8]) -> Vec<u8> {
    let data_len = w * h * 4;
    let mut out = Vec::with_capacity(54 + data_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // negative: top-down rows
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    out.extend_from_slice(&bgra[..data_len as usize]);
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn bmp_header_is_well_formed() {
        let px = vec![0u8; 2 * 2 * 4];
        let b = bmp(2, 2, &px);
        assert_eq!(&b[..2], b"BM");
        assert_eq!(b.len(), 54 + 16);
        assert_eq!(u32::from_le_bytes(b[2..6].try_into().unwrap()), 70);
    }
}
