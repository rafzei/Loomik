use super::{portal, pw_stream, x11};
use crate::{
    model::{Bounds, Source, SourceKind},
    recording::frame::LatestFrame,
};
use anyhow::{Result, ensure};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub fn screen_permission() -> bool {
    true
} // Discovery never opens a portal dialog.
pub fn request_screen_permission() -> bool {
    !portal::wayland() || portal::request()
}
pub fn discover_sources() -> Result<Vec<Source>> {
    if portal::wayland() {
        portal::discover()
    } else {
        x11::discover()
    }
}
pub struct ScreenCapture {
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    bounds: Bounds,
}
impl ScreenCapture {
    pub fn start(
        source: &Source,
        width: u32,
        height: u32,
        fps: u32,
        _: bool,
        frames: LatestFrame,
    ) -> Result<Self> {
        ensure!(
            source.kind == SourceKind::Window,
            "Linux supports isolated window recording; choose a window or media file"
        );
        let portal = if portal::wayland() {
            Some(portal::selected(source.id)?)
        } else {
            None
        };
        let (id, cancel, error) = (
            source.id,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        );
        let (stop, failed) = (cancel.clone(), error.clone());
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                if let Some(selection) = portal {
                    pw_stream::run(
                        selection.remote()?,
                        selection.node,
                        Some((width, height)),
                        fps,
                        frames,
                        stop,
                        false,
                    )
                } else {
                    x11::run(u32::try_from(id)?, width, height, fps, frames, stop)
                }
            })();
            if let Err(e) = result {
                *failed.lock().unwrap() = Some(format!("{e:#}"));
            }
        });
        Ok(Self {
            stop: cancel,
            error,
            bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: width as f64,
                height: height as f64,
            },
        })
    }
    pub fn check(&self) -> Result<()> {
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            anyhow::bail!("{error}");
        }
        Ok(())
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
}
impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
