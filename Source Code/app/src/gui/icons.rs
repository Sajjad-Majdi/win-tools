//! Exe icons as premultiplied BGRA pixels (runs on the worker thread; the UI turns them into D2D bitmaps).
use crate::sys::wide;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

pub fn icon_bgra(path: &str, px: u32) -> Option<Vec<u8>> {
    let wp = wide(path);
    unsafe {
        let mut large = HICON::default();
        let mut small = HICON::default();
        if SHDefExtractIconW(PCWSTR(wp.as_ptr()), 0, 0, Some(&mut large), Some(&mut small), px).is_err() || large.is_invalid() {
            return None;
        }
        if !small.is_invalid() {
            let _ = DestroyIcon(small);
        }
        let out = hicon_pixels(large, px);
        let _ = DestroyIcon(large);
        out
    }
}

unsafe fn hicon_pixels(icon: HICON, px: u32) -> Option<Vec<u8>> {
    unsafe {
        // draw into a 32-bit DIB twice (black and white background) to recover alpha even for icons without it
        let dc = CreateCompatibleDC(None);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: px as i32, biHeight: -(px as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, bmp.into());
        let n = (px * px * 4) as usize;
        std::ptr::write_bytes(bits as *mut u8, 0, n);
        let _ = DrawIconEx(dc, 0, 0, icon, px as i32, px as i32, 0, None, DI_NORMAL);
        let _ = GdiFlush();
        let mut out = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
        let has_alpha = out.chunks_exact(4).any(|p| p[3] != 0);
        if !has_alpha {
            // legacy icon without alpha: draw on white too, alpha = 255 - (white - black)
            std::ptr::write_bytes(bits as *mut u8, 255, n);
            let _ = DrawIconEx(dc, 0, 0, icon, px as i32, px as i32, 0, None, DI_NORMAL);
            let _ = GdiFlush();
            let white = std::slice::from_raw_parts(bits as *const u8, n);
            for i in (0..n).step_by(4) {
                let a = 255 - (white[i + 1] as i32 - out[i + 1] as i32).clamp(0, 255);
                out[i + 3] = a as u8;
            }
        }
        // drawn over transparent black, so colors are already premultiplied
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        Some(out)
    }
}
