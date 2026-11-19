//! Display diagnostics (Help > Copy display diagnostics): what egui believes
//! about the window and the screen, next to what Windows reports for the
//! same window. A plain text block for a bug report; it holds no user name,
//! no path and no computer name.

use std::fmt::Write as _;

/// What egui and the settings know. Plain values, so that a test can build one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Diag {
    pub app_version: String,
    pub renderer: String,
    pub os: String,
    pub ppp: f32,
    pub native_ppp: Option<f32>,
    pub zoom_factor: f32,
    /// egui screen_rect in points: min x, min y, width, height.
    pub screen_rect: [f32; 4],
    pub inner_rect: Option<[f32; 4]>,
    pub outer_rect: Option<[f32; 4]>,
    pub monitor_size: Option<[f32; 2]>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    pub minimized: Option<bool>,
    pub focused: Option<bool>,
    pub ui_scale: f32,
    pub window_width: f32,
    pub window_height: f32,
    pub window_pos: Option<[f32; 2]>,
    pub window_maximized: bool,
    pub win32: Option<Win32>,
}

/// What Windows reports for the app's own window (physical pixels).
#[derive(Clone, Debug, Default)]
pub(crate) struct Win32 {
    /// GetClientRect: width, height.
    pub client: [i32; 2],
    /// Rects as left, top, right, bottom.
    pub window_rect: [i32; 4],
    pub frame_bounds: Option<[i32; 4]>,
    pub dpi_window: u32,
    pub dpi_system: u32,
    /// DPI_AWARENESS of the window and of the thread (-1 invalid, 0 unaware,
    /// 1 system, 2 per monitor), and whether the context is per monitor v2.
    pub awareness_window: i32,
    pub awareness_thread: i32,
    pub per_monitor_v2: bool,
    pub monitor_rect: Option<[i32; 4]>,
    pub work_rect: Option<[i32; 4]>,
    pub monitor_dpi: Option<[u32; 2]>,
    pub zoomed: bool,
    pub style: u32,
    pub ex_style: u32,
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| v.to_string())
}

fn rect_f(r: Option<[f32; 4]>) -> String {
    r.map_or_else(|| "n/a".to_string(), |r| format!("x {:.3} y {:.3} w {:.3} h {:.3}", r[0], r[1], r[2], r[3]))
}

fn rect_i(r: Option<[i32; 4]>) -> String {
    r.map_or_else(|| "n/a".to_string(), |r| format!("{} {} {} {} ({} x {})", r[0], r[1], r[2], r[3], r[2] - r[0], r[3] - r[1]))
}

fn awareness(a: i32) -> &'static str {
    match a {
        0 => "unaware",
        1 => "system aware",
        2 => "per monitor aware",
        _ => "invalid",
    }
}

/// egui's size of the client area in physical pixels, unrounded.
pub(crate) fn physical(d: &Diag) -> [f32; 2] {
    [d.screen_rect[2] * d.ppp, d.screen_rect[3] * d.ppp]
}

/// Yes when egui's physical size (rounded) is not the Win32 client size, or
/// the window DPI / 96 is not egui's native pixels per point.
pub(crate) fn mismatch(d: &Diag) -> bool {
    let Some(w) = &d.win32 else { return false };
    let p = physical(d);
    let size = p[0].round() as i32 != w.client[0] || p[1].round() as i32 != w.client[1];
    let dpi = d.native_ppp.is_some_and(|n| (w.dpi_window as f32 / 96.0 - n).abs() > 1e-4);
    size || dpi
}

/// The text block, one "name: value" per line.
pub(crate) fn format(d: &Diag) -> String {
    let mut s = String::new();
    let mut l = |k: &str, v: String| {
        let _ = writeln!(s, "{k}: {v}");
    };
    l("app_version", d.app_version.clone());
    l("renderer", d.renderer.clone());
    l("os", d.os.clone());
    l("pixels_per_point", format!("{:.4}", d.ppp));
    l("native_pixels_per_point", opt(d.native_ppp.map(|v| format!("{v:.4}"))));
    l("zoom_factor", format!("{:.4}", d.zoom_factor));
    l("screen_rect_points", rect_f(Some(d.screen_rect)));
    let p = physical(d);
    l("screen_rect_physical", format!("{:.3} x {:.3}", p[0], p[1]));
    l("inner_rect_points", rect_f(d.inner_rect));
    l("outer_rect_points", rect_f(d.outer_rect));
    l("monitor_size_points", opt(d.monitor_size.map(|m| format!("{:.3} x {:.3}", m[0], m[1]))));
    l("maximized", opt(d.maximized));
    l("fullscreen", opt(d.fullscreen));
    l("minimized", opt(d.minimized));
    l("focused", opt(d.focused));
    l("settings_ui_scale", format!("{:.3}", d.ui_scale));
    l("settings_window_width", format!("{:.3}", d.window_width));
    l("settings_window_height", format!("{:.3}", d.window_height));
    l("settings_window_pos", opt(d.window_pos.map(|p| format!("{:.3}, {:.3}", p[0], p[1]))));
    l("settings_window_maximized", d.window_maximized.to_string());
    match &d.win32 {
        None => l("win32", "not available".to_string()),
        Some(w) => {
            l("win32_client_size", format!("{} x {}", w.client[0], w.client[1]));
            l("win32_window_rect", rect_i(Some(w.window_rect)));
            l("win32_extended_frame_bounds", rect_i(w.frame_bounds));
            l("win32_dpi_window", w.dpi_window.to_string());
            l("win32_dpi_system", w.dpi_system.to_string());
            l("win32_dpi_awareness_window", format!("{} ({})", w.awareness_window, awareness(w.awareness_window)));
            l("win32_dpi_awareness_thread", format!("{} ({})", w.awareness_thread, awareness(w.awareness_thread)));
            l("win32_per_monitor_v2", w.per_monitor_v2.to_string());
            l("win32_monitor_rect", rect_i(w.monitor_rect));
            l("win32_monitor_work_rect", rect_i(w.work_rect));
            l("win32_monitor_effective_dpi", opt(w.monitor_dpi.map(|m| format!("{} x {}", m[0], m[1]))));
            l("win32_is_zoomed", w.zoomed.to_string());
            l("win32_style", format!("0x{:08X}", w.style));
            l("win32_ex_style", format!("0x{:08X}", w.ex_style));
        }
    }
    l("mismatch", if mismatch(d) { "yes" } else { "no" }.to_string());
    s
}

/// The OpenGL vendor, renderer and version, and the viewport of the last
/// paint: "glow, <vendor> / <renderer> / <version>, viewport x y w h". On a
/// laptop with two GPUs this names the GPU that draws the window.
#[allow(unsafe_code)]
pub(crate) fn gl_info(gl: &eframe::glow::Context) -> String {
    use eframe::glow::{self, HasContext as _};
    // SAFETY: plain state queries on the current context of this thread
    // (eframe calls `update` with its GL context current).
    unsafe {
        let mut vp = [0i32; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut vp);
        format!(
            "glow, {} / {} / {}, viewport {} {} {} {}",
            gl.get_parameter_string(glow::VENDOR),
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION),
            vp[0],
            vp[1],
            vp[2],
            vp[3]
        )
    }
}

/// The Windows version (major.minor.build), from RtlGetVersion.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn os_version() -> String {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    // SAFETY: a zeroed OSVERSIONINFOW with its size set is what RtlGetVersion expects.
    unsafe {
        let mut v: OSVERSIONINFOW = std::mem::zeroed();
        v.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        if RtlGetVersion(&mut v) == 0 {
            return format!("Windows {}.{}.{}", v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber);
        }
    }
    "Windows".to_string()
}

#[cfg(not(windows))]
pub(crate) fn os_version() -> String {
    std::env::consts::OS.to_string()
}

/// Read what Windows reports for the window `hwnd`.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn win32(hwnd: isize) -> Win32 {
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::HiDpi::{
        AreDpiAwarenessContextsEqual, GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor, GetDpiForSystem, GetDpiForWindow, GetThreadDpiAwarenessContext, GetWindowDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowLongW, GetWindowRect, IsZoomed, GWL_EXSTYLE, GWL_STYLE};
    let r4 = |r: RECT| [r.left, r.top, r.right, r.bottom];
    let h = hwnd as HWND;
    // SAFETY: plain Win32 queries on the app's own live window; every out
    // value is a local of the right type and size.
    unsafe {
        let mut w = Win32::default();
        let mut r: RECT = std::mem::zeroed();
        if GetClientRect(h, &mut r) != 0 {
            w.client = [r.right - r.left, r.bottom - r.top];
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(h, &mut r) != 0 {
            w.window_rect = r4(r);
        }
        let mut r: RECT = std::mem::zeroed();
        if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32) == 0 {
            w.frame_bounds = Some(r4(r));
        }
        w.dpi_window = GetDpiForWindow(h);
        w.dpi_system = GetDpiForSystem();
        let wc = GetWindowDpiAwarenessContext(h);
        w.awareness_window = GetAwarenessFromDpiAwarenessContext(wc);
        w.awareness_thread = GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext());
        w.per_monitor_v2 = AreDpiAwarenessContextsEqual(wc, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0;
        let m = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        if !m.is_null() {
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(m, &mut mi) != 0 {
                w.monitor_rect = Some(r4(mi.rcMonitor));
                w.work_rect = Some(r4(mi.rcWork));
            }
            let (mut x, mut y) = (0u32, 0u32);
            if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y) == 0 {
                w.monitor_dpi = Some([x, y]);
            }
        }
        w.zoomed = IsZoomed(h) != 0;
        w.style = GetWindowLongW(h, GWL_STYLE) as u32;
        w.ex_style = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Diag {
        Diag {
            app_version: "0.0.0".into(),
            renderer: "glow".into(),
            os: "Windows 10.0.26200".into(),
            ppp: 1.0,
            native_ppp: Some(1.0),
            zoom_factor: 1.0,
            screen_rect: [0.0, 0.0, 1280.0, 800.0],
            ui_scale: 1.0,
            window_width: 1280.0,
            window_height: 800.0,
            ..Diag::default()
        }
    }

    fn w32(cw: i32, ch: i32, dpi: u32) -> Win32 {
        Win32 { client: [cw, ch], dpi_window: dpi, dpi_system: 96, awareness_window: 2, awareness_thread: 2, per_monitor_v2: true, style: 0x14CF_0000, ex_style: 0x100, ..Win32::default() }
    }

    /// The Win32 queries run against the desktop window (it exists without
    /// opening any window): the client size and the DPI are filled in.
    #[cfg(windows)]
    #[test]
    #[allow(unsafe_code)]
    fn win32_queries_on_the_desktop_window() {
        // SAFETY: GetDesktopWindow has no arguments and no preconditions.
        let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow() } as isize;
        let w = win32(hwnd);
        assert!(w.client[0] > 0 && w.client[1] > 0, "{w:?}");
        assert!(w.dpi_system >= 96 && w.monitor_rect.is_some(), "{w:?}");
        let d = Diag { win32: Some(w), ..base() };
        println!("{}", format(&d));
    }

    #[test]
    fn format_and_mismatch_rule() {
        let mut d = base();
        let t = format(&d);
        assert!(t.contains("win32: not available\n"), "{t}");
        assert!(t.ends_with("mismatch: no\n"), "{t}");
        assert!(t.contains("screen_rect_physical: 1280.000 x 800.000\n"), "{t}");
        assert!(t.lines().all(|l| l.contains(": ")), "one name: value per line\n{t}");
        // Sizes and DPI match: no.
        d.win32 = Some(w32(1280, 800, 96));
        let t = format(&d);
        assert!(t.contains("win32_client_size: 1280 x 800\n") && t.contains("win32_style: 0x14CF0000\n"), "{t}");
        assert!(t.contains("win32_dpi_awareness_window: 2 (per monitor aware)\n"), "{t}");
        assert!(!mismatch(&d));
        // One pixel row more in the real client area: yes.
        d.win32 = Some(w32(1280, 801, 96));
        assert!(mismatch(&d) && format(&d).ends_with("mismatch: yes\n"));
        // A fractional egui size rounds: 1023.75 x 1.25 = 1279.6875 -> 1280.
        d.ppp = 1.25;
        d.native_ppp = Some(1.25);
        d.screen_rect = [0.0, 0.0, 1023.75, 640.0];
        d.win32 = Some(w32(1280, 800, 120));
        assert!(!mismatch(&d), "{}", format(&d));
        // The DPI does not fit native_pixels_per_point: yes.
        d.win32 = Some(w32(1280, 800, 96));
        assert!(mismatch(&d));
    }
}
