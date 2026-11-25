//! Picture decoding for the image viewer: Windows Imaging Component (the codecs Windows has: JPEG, PNG, GIF, BMP,
//! TIFF, ICO, WebP, JPEG XR, and HEIC / AVIF / RAW when their extensions are installed). EXIF orientation is applied,
//! huge pictures are scaled down to a size the GPU can hold, and the pixels come out as 32-bit BGRA rows.
use crate::util::*;
use std::path::Path;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToUInt32};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::PCWSTR;

/// File types the viewer opens (by extension, lower case, no dot).
pub const PICTURES: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "webp", "heic", "heif", "avif", "tif", "tiff", "ico", "jxr", "wdp", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2",
];

/// Is `ext` (with or without the dot) a picture the viewer opens?
pub fn is_picture(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    PICTURES.contains(&e.as_str())
}

/// Decoded pixels: `width` x `height` BGRA (premultiplied when asked), already turned upright. `full_w` x `full_h`
/// is the upright size of the original (bigger than width x height when it was scaled down to fit the limits).
#[derive(Clone, Debug)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub full_w: u32,
    pub full_h: u32,
    /// EXIF orientation found in the file (1 = upright, 2..8 = turned / mirrored)
    pub orientation: u32,
    /// frames in the file (an animated GIF has several; the first is shown)
    pub frames: u32,
}

/// Turn decoded pixels (4 bytes each, `w` x `h` as stored) upright for an EXIF orientation:
/// 2 mirror, 3 turned 180, 4 mirror vertical, 5 transpose, 6 turned 90 (shown turned right), 7 transverse,
/// 8 turned 270. Done here on the buffer: WIC's flip rotator re-reads a JPEG line by line (1.6 s for 1200 x 800).
/// Checked against PIL's exif_transpose for 1..8. Returns the new buffer and its size.
pub fn orient(px: &[u8], w: u32, h: u32, o: u32) -> (Vec<u8>, u32, u32) {
    if !(2..=8).contains(&o) || w == 0 || h == 0 {
        return (px.to_vec(), w, h);
    }
    let (w, h) = (w as usize, h as usize);
    let src: Vec<u32> = px.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
    let (nw, nh) = if orientation_swaps(o) { (h, w) } else { (w, h) };
    let mut dst = vec![0u32; nw * nh];
    for dy in 0..nh {
        let row = &mut dst[dy * nw..(dy + 1) * nw];
        for (dx, d) in row.iter_mut().enumerate() {
            let (sx, sy) = match o {
                2 => (w - 1 - dx, dy),
                3 => (w - 1 - dx, h - 1 - dy),
                4 => (dx, h - 1 - dy),
                5 => (dy, dx),
                6 => (dy, h - 1 - dx),
                7 => (w - 1 - dy, h - 1 - dx),
                _ => (w - 1 - dy, dx),
            };
            *d = src[sy * w + sx];
        }
    }
    let mut out = Vec::with_capacity(dst.len() * 4);
    for v in dst {
        out.extend_from_slice(&v.to_ne_bytes());
    }
    (out, nw as u32, nh as u32)
}

/// Orientations 5..8 swap width and height.
pub fn orientation_swaps(o: u32) -> bool {
    (5..=8).contains(&o)
}

/// The size to decode a w x h picture at so that no side passes `max_side` and the pixel count stays under `max_px`.
pub fn fit_limits(w: u32, h: u32, max_side: u32, max_px: u64) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (w, h);
    }
    let mut k = 1.0f64;
    let side = w.max(h) as f64;
    if side > max_side as f64 {
        k = k.min(max_side as f64 / side);
    }
    let px = w as f64 * h as f64;
    if px > max_px as f64 {
        k = k.min((max_px as f64 / px).sqrt());
    }
    if k >= 1.0 {
        return (w, h);
    }
    (((w as f64 * k).floor() as u32).max(1), ((h as f64 * k).floor() as u32).max(1))
}

fn orientation_of(frame: &IWICBitmapFrameDecode) -> u32 {
    unsafe {
        let Ok(q) = frame.GetMetadataQueryReader() else { return 1 };
        // JPEG keeps EXIF in APP1, TIFF / RAW in the first IFD; HEIF and others answer the photo property
        for name in ["/app1/ifd/{ushort=274}", "/ifd/{ushort=274}", "System.Photo.Orientation"] {
            let w = wide(name);
            let mut pv = PROPVARIANT::default();
            if q.GetMetadataByName(PCWSTR(w.as_ptr()), &mut pv).is_ok() {
                let v = PropVariantToUInt32(&pv).unwrap_or(1);
                let _ = PropVariantClear(&mut pv);
                if (1..=8).contains(&v) {
                    return v;
                }
            }
        }
        1
    }
}

/// Decode a picture with WIC. `max_side` / `max_px` bound the result (bigger pictures are scaled down with a good
/// filter); `premultiplied` gives the pixel format Direct2D draws. Needs COM; it starts it when needed.
pub fn decode(path: &Path, max_side: u32, max_px: u64, premultiplied: bool) -> Res<Decoded> {
    let _c = com();
    let fail = |what: &str, e: windows::core::Error| win_err(what, &e);
    let w = wide(abs(path).as_os_str());
    unsafe {
        let fac: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(|e| fail("imaging factory", e))?;
        let dec = fac.CreateDecoderFromFilename(PCWSTR(w.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).map_err(|e| {
            // no decoder took the file: for the types Windows always reads, the file is damaged (or not a picture)
            let built_in = matches!(ext_of(&path.to_string_lossy()).as_str(), "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "gif" | "bmp" | "dib" | "tif" | "tiff" | "ico" | "jxr" | "wdp");
            match e.code().0 as u32 {
                0x88982F50 | 0x88982F61 | 0x88982F60 if built_in => "The file is damaged or is not really a picture".to_string(),
                0x88982F50 => "Windows has no decoder for this kind of picture".to_string(),
                _ => win_err("cannot open this picture", &e),
            }
        })?;
        let frames = dec.GetFrameCount().unwrap_or(1).max(1);
        // an icon file holds several sizes: the biggest one
        let mut best = 0u32;
        if frames > 1 && ext_of(&path.to_string_lossy()) == "ico" {
            let mut area = 0u64;
            for i in 0..frames.min(64) {
                if let Ok(f) = dec.GetFrame(i) {
                    let (mut fw, mut fh) = (0u32, 0u32);
                    if f.GetSize(&mut fw, &mut fh).is_ok() && fw as u64 * fh as u64 > area {
                        area = fw as u64 * fh as u64;
                        best = i;
                    }
                }
            }
        }
        let frame = dec.GetFrame(best).map_err(|e| fail("cannot read the picture", e))?;
        let (mut fw, mut fh) = (0u32, 0u32);
        frame.GetSize(&mut fw, &mut fh).map_err(|e| fail("picture size", e))?;
        if fw == 0 || fh == 0 {
            return Err("the picture is empty".into());
        }
        let o = orientation_of(&frame);
        let (tw, th) = fit_limits(fw, fh, max_side, max_px);
        let mut src: IWICBitmapSource = frame.cast().map_err(|e| fail("picture", e))?;
        if (tw, th) != (fw, fh) {
            let sc = fac.CreateBitmapScaler().map_err(|e| fail("scaler", e))?;
            sc.Initialize(&src, tw, th, WICBitmapInterpolationModeFant).map_err(|e| fail("scaler", e))?;
            src = sc.cast().map_err(|e| fail("scaler", e))?;
        }
        let conv = fac.CreateFormatConverter().map_err(|e| fail("converter", e))?;
        let fmt = if premultiplied { GUID_WICPixelFormat32bppPBGRA } else { GUID_WICPixelFormat32bppBGRA };
        conv.Initialize(&src, &fmt, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).map_err(|e| fail("converter", e))?;
        let (mut ow, mut oh) = (0u32, 0u32);
        conv.GetSize(&mut ow, &mut oh).map_err(|e| fail("picture size", e))?;
        let stride = ow * 4;
        let mut px = vec![0u8; stride as usize * oh as usize];
        conv.CopyPixels(std::ptr::null(), stride, &mut px).map_err(|e| fail("cannot decode the picture", e))?;
        let (px, ow, oh) = if o != 1 { orient(&px, ow, oh, o) } else { (px, ow, oh) };
        let (full_w, full_h) = if orientation_swaps(o) { (fh, fw) } else { (fw, fh) };
        Ok(Decoded { width: ow, height: oh, bgra: px, full_w, full_h, orientation: o, frames })
    }
}

use windows::core::Interface;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_keep_small_pictures_and_shrink_big_ones() {
        assert_eq!(fit_limits(6000, 4000, 16384, 80_000_000), (6000, 4000));
        assert_eq!(fit_limits(20000, 10000, 16384, 1_000_000_000), (16384, 8192));
        let (w, h) = fit_limits(12000, 9000, 16384, 64_000_000);
        assert!(w as u64 * h as u64 <= 64_000_000 && (w as f32 / h as f32 - 12.0 / 9.0).abs() < 0.01);
        assert_eq!(fit_limits(0, 10, 100, 100), (0, 10));
    }

    #[test]
    fn orientation_swaps_only_for_quarter_turns() {
        assert!(!orientation_swaps(1) && !orientation_swaps(3) && orientation_swaps(6) && orientation_swaps(8));
        // 2 x 1 pixels [a b]: turned right (6) gives a column a over b; turned left (8) b over a; mirrored b a
        let px = [1u8, 1, 1, 1, 2, 2, 2, 2];
        assert_eq!(orient(&px, 2, 1, 6), (px.to_vec(), 1, 2));
        assert_eq!(orient(&px, 2, 1, 8), (vec![2, 2, 2, 2, 1, 1, 1, 1], 1, 2));
        assert_eq!(orient(&px, 2, 1, 2), (vec![2, 2, 2, 2, 1, 1, 1, 1], 2, 1));
        assert_eq!(orient(&px, 2, 1, 1), (px.to_vec(), 2, 1));
    }

    #[test]
    fn picture_types() {
        assert!(is_picture("JPG") && is_picture(".png") && is_picture("heic") && !is_picture("mp4") && !is_picture("txt"));
    }

    /// A PNG written with WIC decodes back to the same size and pixels.
    #[test]
    fn decodes_a_png() {
        let dir = std::env::temp_dir().join(format!("fm-picture-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.png");
        let mut px = vec![0u8; 5 * 3 * 4];
        for (i, c) in px.chunks_exact_mut(4).enumerate() {
            c.copy_from_slice(&[(i * 10) as u8, 20, 200, 255]);
        }
        crate::thumb::save_png(&crate::thumb::Image { width: 5, height: 3, bgra: px.clone() }, &p).unwrap();
        let d = decode(&p, 16384, 80_000_000, false).unwrap();
        assert_eq!((d.width, d.height, d.full_w, d.full_h, d.orientation), (5, 3, 5, 3, 1));
        assert_eq!(d.bgra, px);
        // scaled down to the limit
        let s = decode(&p, 2, 80_000_000, true).unwrap();
        assert_eq!((s.width, s.height, s.full_w), (2, 1, 5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
