//! Platform-selected native capture. File sources never enter these backends.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod linux_pixels;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod unavailable;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub use unavailable::*;

pub fn desktop_supported() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    ))
}
