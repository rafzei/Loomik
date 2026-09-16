//! Capability boundary for other systems without a native capture backend.
//! Portable media recording remains usable; unavailable devices are never listed.
use crate::{
    model::{Bounds, Device, RecordingClock, Source},
    recording::frame::LatestFrame,
};
use anyhow::{Result, bail};
use crossbeam_channel::Receiver;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub fn screen_permission() -> bool {
    false
}
pub fn request_screen_permission() -> bool {
    false
}
pub fn discover_sources() -> Result<Vec<Source>> {
    Ok(vec![])
}
pub fn discover_cameras() -> Result<Vec<Device>> {
    Ok(vec![])
}
pub fn discover_microphones() -> Result<Vec<Device>> {
    Ok(vec![])
}

pub struct ScreenCapture;
impl ScreenCapture {
    pub fn start(_: &Source, _: u32, _: u32, _: u32, _: bool, _: LatestFrame) -> Result<Self> {
        bail!(
            "Native desktop capture is not available on this OS yet. Choose an image or video background."
        )
    }
    pub fn check(&self) -> Result<()> {
        bail!("Desktop capture unavailable")
    }
    pub fn bounds(&self) -> Bounds {
        Bounds::default()
    }
}
pub struct Camera {
    pub frames: LatestFrame,
    pub events: Receiver<Result<(), String>>,
}
impl Camera {
    pub fn start(_: String) -> Self {
        let (tx, events) = crossbeam_channel::bounded(1);
        let _ = tx.send(Err(
            "Native camera capture is not available on this OS yet.".into(),
        ));
        Self {
            frames: LatestFrame::default(),
            events,
        }
    }
}
pub struct Microphone;
impl Microphone {
    pub fn start(_: &str, _: &Path, _: Arc<Mutex<RecordingClock>>) -> Result<Self> {
        bail!("Native microphone capture is not available on this OS yet.")
    }
    pub fn check(&mut self) -> Result<()> {
        bail!("Microphone capture unavailable")
    }
    pub fn sample_rate(&self) -> u32 {
        48_000
    }
    pub fn stop(&mut self) -> Result<serde_json::Value> {
        bail!("Microphone capture unavailable")
    }
}
