//! Apply capture exclusion before any top-level Loomik window is shown.
use anyhow::{Result, ensure};
use std::{
    sync::Mutex,
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::*,
    },
    core::BOOL,
};
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ERROR: Mutex<Option<String>> = Mutex::new(None);
pub struct WindowProtection(HHOOK);
impl Drop for WindowProtection {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
        INSTALLED.store(false, Ordering::Release);
    }
}
pub fn install() -> Result<WindowProtection> {
    ensure!(
        supported_version(),
        "Windows 10 version 2004 or newer is required for capture exclusion"
    );
    let hook = unsafe {
        SetWindowsHookExW(
            WH_CALLWNDPROC,
            Some(before_window_message),
            None,
            GetCurrentThreadId(),
        )?
    };
    INSTALLED.store(true, Ordering::Release);
    Ok(WindowProtection(hook))
}
pub fn supported_version() -> bool {
    use windows::{
        Wdk::System::SystemServices::RtlGetVersion,
        Win32::System::SystemInformation::OSVERSIONINFOW,
    };
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe { RtlGetVersion(&mut version) }.is_ok() && version.dwBuildNumber >= 19041
}
unsafe extern "system" fn before_window_message(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && l.0 != 0 {
        let message = unsafe { &*(l.0 as *const CWPSTRUCT) };
        if (message.message == WM_SHOWWINDOW && message.wParam.0 != 0)
            || (message.message == WM_WINDOWPOSCHANGING
                && message.lParam.0 != 0
                && unsafe { &*(message.lParam.0 as *const WINDOWPOS) }
                    .flags
                    .contains(SWP_SHOWWINDOW))
        {
            let hwnd = message.hwnd;
            if unsafe { GetAncestor(hwnd, GA_ROOT) } == hwnd {
                let mut affinity = 0;
                let already = unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.is_ok()
                    && affinity == WDA_EXCLUDEFROMCAPTURE.0;
                if !already
                    && let Err(error) =
                        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }
                {
                    *ERROR.lock().unwrap() = Some(format!(
                        "Cannot exclude Loomik controls from capture: {error}. Windows 10 version 2004 or later is required."
                    ));
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}
unsafe extern "system" fn check_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == std::process::id() && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let mut affinity = 0;
        if unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.is_err()
            || affinity != WDA_EXCLUDEFROMCAPTURE.0
        {
            unsafe {
                *(data.0 as *mut bool) = false;
            }
        }
    }
    true.into()
}
pub fn verify_exclusion() -> Result<()> {
    ensure!(
        INSTALLED.load(Ordering::Acquire),
        "Loomik window protection was not initialized. Restart the application."
    );
    if let Some(error) = ERROR.lock().unwrap().as_ref() {
        anyhow::bail!("{error}");
    }
    let mut protected = true;
    unsafe {
        EnumWindows(
            Some(check_window),
            LPARAM((&mut protected as *mut bool) as isize),
        )?;
    }
    ensure!(
        protected,
        "A Loomik window cannot be excluded. Recording stopped before publishing the frame. Restart Loomik or use a file background."
    );
    Ok(())
}

unsafe extern "system" fn find_camera(hwnd: HWND, data: LPARAM) -> BOOL {
    let mut title = [0u16; 64];
    let len = unsafe { GetWindowTextW(hwnd, &mut title) }.max(0) as usize;
    if String::from_utf16_lossy(&title[..len]) == "Loomik camera" {
        unsafe {
            *(data.0 as *mut HWND) = hwnd;
        }
        return false.into();
    }
    true.into()
}
pub fn camera_bounds(inset: f64, size: f64, scale: f64) -> Option<crate::model::CameraPlacement> {
    use windows::Win32::Foundation::RECT;
    let mut hwnd = HWND::default();
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(find_camera),
            LPARAM((&mut hwnd as *mut HWND) as isize),
        );
    }
    if hwnd.is_invalid() {
        return None;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(crate::model::CameraPlacement {
        x: rect.left as f64 + inset * scale,
        y: rect.top as f64 + inset * scale,
        diameter: size * scale,
        visible: true,
        mirror: true,
    })
}
