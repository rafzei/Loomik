mod camera;
mod microphone;
mod screen;
mod time;
pub use camera::{Camera, discover_cameras};
pub use microphone::{Microphone, discover_microphones};
pub use screen::{ScreenCapture, discover_sources, request_screen_permission, screen_permission};

use anyhow::{Result, bail};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaType};

/// Call only from a worker. macOS delivers its permission callback independently.
pub(crate) fn request_media_permission(media: &AVMediaType, label: &str) -> Result<()> {
    // SAFETY: media is one of Apple's video/audio constants and the completion
    // block captures only a thread-safe channel. The API copies the block.
    unsafe {
        let status = AVCaptureDevice::authorizationStatusForMediaType(media);
        if status == AVAuthorizationStatus::Authorized {
            return Ok(());
        }
        if status == AVAuthorizationStatus::NotDetermined {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let completion = block2::RcBlock::new(move |allowed: objc2::runtime::Bool| {
                let _ = tx.try_send(allowed.as_bool());
            });
            AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &completion);
            if rx
                .recv_timeout(std::time::Duration::from_secs(120))
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
    }
    bail!(
        "{label} access is not allowed. Enable Loomik in System Settings → Privacy & Security → {label}, then try again."
    )
}
