//! Test helper (not shipped): which step of the GPU renderer loads D3D10Warp.dll?
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::core::{Interface, w};

fn warp() -> bool {
    unsafe { GetModuleHandleW(w!("d3d10warp.dll")).is_ok() }
}

fn main() {
    unsafe {
        println!("start: {}", warp());
        let mut dev: Option<ID3D11Device> = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, None).unwrap();
        println!("after D3D11CreateDevice: {}", warp());
        let dev = dev.unwrap();
        let dxgi: IDXGIDevice = dev.cast().unwrap();
        let f: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
        println!("after D2D1CreateFactory: {}", warp());
        let d = f.CreateDevice(&dxgi).unwrap();
        println!("after ID2D1Factory1::CreateDevice: {}", warp());
        let _dc = d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE).unwrap();
        println!("after CreateDeviceContext: {}", warp());
        let _dc2 = d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_ENABLE_MULTITHREADED_OPTIMIZATIONS).unwrap();
        println!("after a second DC: {}", warp());
        use windows::Win32::Graphics::DirectComposition::*;
        use windows::Win32::Graphics::Dxgi::Common::*;
        use windows::Win32::Graphics::Dxgi::*;
        let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
        println!("after CreateDXGIFactory2: {}", warp());
        let desc = DXGI_SWAP_CHAIN_DESC1 { Width: 800, Height: 600, Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 2, Scaling: DXGI_SCALING_STRETCH, SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED, ..Default::default() };
        let swap = factory.CreateSwapChainForComposition(&dev, &desc, None).unwrap();
        println!("after CreateSwapChainForComposition: {}", warp());
        let _dc_: IDCompositionDevice = DCompositionCreateDevice(&dxgi).unwrap();
        println!("after DCompositionCreateDevice: {}", warp());
        let surf: IDXGISurface = swap.GetBuffer(0).unwrap();
        let props = D2D1_BITMAP_PROPERTIES1 { pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW, colorContext: std::mem::ManuallyDrop::new(None) };
        let bmp = _dc.CreateBitmapFromDxgiSurface(&surf, Some(&props)).unwrap();
        _dc.SetTarget(&bmp);
        _dc.BeginDraw();
        _dc.Clear(None);
        let b = _dc.CreateSolidColorBrush(&windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F { r: 1.0, g: 0.0, b: 0.0, a: 0.5 }, None).unwrap();
        _dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: 1.0, top: 1.0, right: 100.0, bottom: 50.0 }, radiusX: 8.0, radiusY: 8.0 }, &b);
        let _ = _dc.EndDraw(None, None);
        println!("after drawing: {}", warp());
        let _ = swap.Present(1, DXGI_PRESENT(0));
        println!("after Present: {}", warp());
        use windows::Win32::UI::WindowsAndMessaging::*;
        // a never-shown popup window, only to have an HWND for the composition target
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("x"), WS_POPUP, 0, 0, 10, 10, None, None, None, None).unwrap();
        let target = _dc_.CreateTargetForHwnd(hwnd, true).unwrap();
        println!("after CreateTargetForHwnd: {}", warp());
        let visual = _dc_.CreateVisual().unwrap();
        println!("after CreateVisual: {}", warp());
        visual.SetContent(&swap).unwrap();
        println!("after SetContent: {}", warp());
        target.SetRoot(&visual).unwrap();
        _dc_.Commit().unwrap();
        println!("after Commit: {}", warp());
        let _ = DestroyWindow(hwnd);
    }
}
