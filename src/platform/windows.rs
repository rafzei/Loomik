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
unsafe extern "system" fn before_window_message(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && l.0 != 0 {
        let message = unsafe { &*(l.0 as *const CWPSTRUCT) };
        if (message.message == WM_SHOWWINDOW && message.wParam.0 != 0)
            || message.message == WM_WINDOWPOSCHANGING
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

pub fn camera_bounds(inset: f64, size: f64) -> Option<crate::model::CameraPlacement> {
    use windows::{
        Win32::{Foundation::RECT, UI::HiDpi::GetDpiForWindow},
        core::w,
    };
    let hwnd = unsafe { FindWindowW(None, w!("Loomik camera")) }.ok()?;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    let scale = unsafe { GetDpiForWindow(hwnd) } as f64 / 96.0;
    Some(crate::model::CameraPlacement {
        x: rect.left as f64 + inset * scale,
        y: rect.top as f64 + inset * scale,
        diameter: size * scale,
        visible: true,
        mirror: true,
    })
}
