//! Platform-selected native capture. File sources never enter these backends.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unavailable;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use unavailable::*;

pub fn desktop_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}
