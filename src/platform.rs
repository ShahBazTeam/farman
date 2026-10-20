//! Small Windows-native integration points that winit does not expose:
//! borderless window polish and a fatal-error message box.

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Give the undecorated window a native dark shadow, rounded corners and a
/// border that matches the active theme. Best-effort: older Windows builds
/// simply ignore attributes they do not understand.
pub fn apply_borderless_chrome(window: &winit::window::Window, background: (u8, u8, u8)) {
    #[cfg(windows)]
    unsafe {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let hwnd = win32.hwnd.get() as isize as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return;
        }
        let enabled: i32 = 1;
        let _ = windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows_sys::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &enabled as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        let rounded: i32 = 2; // DWMWCP_ROUND
        let _ = windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows_sys::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &rounded as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        // COLORREF is 0x00BBGGRR.
        let color = (background.2 as u32) << 16 | (background.1 as u32) << 8 | background.0 as u32;
        let _ = windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows_sys::Win32::Graphics::Dwm::DWMWA_BORDER_COLOR as u32,
            &color as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
    #[cfg(not(windows))]
    {
        let _ = (window, background);
    }
}

/// Keep the window inside the primary monitor's work area (excluding the
/// taskbar). At 150% DPI a 1280x800 logical window is 1920x1200 physical, which
/// otherwise places the status bar underneath the taskbar.
pub fn fit_window_to_work_area(window: &winit::window::Window) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, SetWindowPos, SystemParametersInfoW, SPI_GETWORKAREA, SWP_NOACTIVATE,
            SWP_NOZORDER,
        };
        if window.is_maximized() {
            return;
        }
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let hwnd = win32.hwnd.get() as isize as windows_sys::Win32::Foundation::HWND;
        let mut work: RECT = std::mem::zeroed();
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            &mut work as *mut RECT as *mut core::ffi::c_void,
            0,
        ) == 0
        {
            return;
        }
        let mut current: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut current) == 0 {
            return;
        }
        let work_w = (work.right - work.left).max(320);
        let work_h = (work.bottom - work.top).max(240);
        let width = (current.right - current.left).max(320);
        let height = (current.bottom - current.top).max(240);
        let max_w = work_w - 24;
        let max_h = work_h - 24;
        let new_w = width.min(max_w);
        let new_h = height.min(max_h);
        let x = work.left + (work_w - new_w) / 2;
        let y = work.top + (work_h - new_h) / 2;

        if current.left == x
            && current.top == y
            && current.right - current.left == new_w
            && current.bottom - current.top == new_h
        {
            return;
        }
        // Center the startup window inside the work area. This also prevents
        // the initial cascade position from pushing the status bar beneath the
        // taskbar on high-DPI displays.
        let _ = SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            new_w,
            new_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    #[cfg(not(windows))]
    {
        let _ = window;
    }
}

pub fn message_box(title: &str, message: &str, error: bool) {
    #[cfg(windows)]
    unsafe {
        let title: Vec<u16> = OsStr::new(title)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let message: Vec<u16> = OsStr::new(message)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut flags =
            windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                | windows_sys::Win32::UI::WindowsAndMessaging::MB_SETFOREGROUND;
        if error {
            flags |= windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR;
        } else {
            flags |= windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION;
        }
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            flags,
        );
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}: {message}");
    }
}

