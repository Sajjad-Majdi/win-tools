//! Thumbnails and icons from the shell (IShellItemImageFactory: the same cache and providers Explorer uses),
//! as straight-alpha BGRA pixels for the GUI, or written to a PNG file with WIC.
use crate::util::*;
use std::path::Path;
use windows::Win32::Foundation::{GENERIC_WRITE, SIZE};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW, HGDIOBJ};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICBitmapFrameEncode, IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY};
use windows::core::PCWSTR;

/// Image pixels: BGRA, 4 bytes per pixel, rows top to bottom, straight (not premultiplied) alpha.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbKind {
    /// A real thumbnail when one exists, else the icon (Explorer's normal view).
    Auto,
    /// Only a real thumbnail (error when the type has none).
    ThumbnailOnly,
    /// Only the icon.
    IconOnly,
}

/// Thumbnail (or icon) of a file or folder, fitted into `size` x `size`. Needs COM; it starts it when needed.
pub fn thumbnail(path: &Path, size: u32, kind: ThumbKind) -> Res<Image> {
    let _c = com();
    let path = abs(path);
    let w = wide(path.as_os_str());
    unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).map_err(|e| win_err(&format!("cannot open {}", path.display()), &e))?;
        let flags = match kind {
            ThumbKind::Auto => SIIGBF_RESIZETOFIT,
            ThumbKind::ThumbnailOnly => SIIGBF(SIIGBF_RESIZETOFIT.0 | SIIGBF_THUMBNAILONLY.0),
            ThumbKind::IconOnly => SIIGBF(SIIGBF_RESIZETOFIT.0 | SIIGBF_ICONONLY.0),
        };
        let s = size.clamp(16, 2560) as i32;
        let hbm = f.GetImage(SIZE { cx: s, cy: s }, flags).map_err(|e| win_err("no image for this item", &e))?;
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(hbm.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let (wd, ht) = (bm.bmWidth.max(0) as u32, bm.bmHeight.unsigned_abs());
        let mut px = vec![0u8; (wd * ht * 4) as usize];
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: wd as i32,
                biHeight: -(ht as i32), // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, hbm, 0, ht, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        if lines == 0 || wd == 0 {
            return Err("could not read the image".into());
        }
        // Shell bitmaps are premultiplied; photos often come with alpha 0 everywhere (= opaque).
        if px.chunks_exact(4).all(|p| p[3] == 0) {
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
        } else {
            for p in px.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
        }
        Ok(Image { width: wd, height: ht, bgra: px })
    }
}

/// Encode BGRA pixels as a PNG file (WIC). Needs COM; it starts it when needed.
pub fn save_png(img: &Image, out: &Path) -> Res<()> {
    let _c = com();
    let fail = |what: &str, e: windows::core::Error| win_err(&format!("PNG {what}"), &e);
    unsafe {
        let fac: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(|e| fail("factory", e))?;
        let bmp = fac.CreateBitmapFromMemory(img.width, img.height, &GUID_WICPixelFormat32bppBGRA, img.width * 4, &img.bgra).map_err(|e| fail("bitmap", e))?;
        let stream = fac.CreateStream().map_err(|e| fail("stream", e))?;
        let w = wide(out.as_os_str());
        stream.InitializeFromFilename(PCWSTR(w.as_ptr()), GENERIC_WRITE.0).map_err(|e| win_err(&format!("cannot write {}", out.display()), &e))?;
        let enc = fac.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).map_err(|e| fail("encoder", e))?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache).map_err(|e| fail("encoder", e))?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut bag: Option<IPropertyBag2> = None;
        enc.CreateNewFrame(&mut frame, &mut bag).map_err(|e| fail("frame", e))?;
        let frame = frame.ok_or("PNG frame missing")?;
        frame.Initialize(bag.as_ref()).map_err(|e| fail("frame", e))?;
        frame.SetSize(img.width, img.height).map_err(|e| fail("size", e))?;
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt).map_err(|e| fail("format", e))?;
        frame.WriteSource(&bmp, std::ptr::null()).map_err(|e| fail("write", e))?;
        frame.Commit().map_err(|e| fail("commit", e))?;
        enc.Commit().map_err(|e| fail("commit", e))?;
    }
    Ok(())
}

/// Thumbnail straight to a PNG file. Returns the image size.
pub fn thumbnail_png(path: &Path, out: &Path, size: u32, kind: ThumbKind) -> Res<(u32, u32)> {
    let img = thumbnail(path, size, kind)?;
    save_png(&img, out)?;
    Ok((img.width, img.height))
}
