use crate::{
    model::{Bounds, Source, SourceKind},
    recording::frame::{LatestFrame, VideoFrame},
};
use anyhow::{Context, Result};
use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    string::CFString,
};
use screencapturekit::{cm::CMSampleBufferExt, cv::CVPixelBufferLockFlags, prelude::*};
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}
pub fn screen_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}
pub fn request_screen_permission() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

fn bounds(rect: screencapturekit::cg::CGRect) -> Bounds {
    Bounds {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    }
}

pub fn discover_sources() -> Result<Vec<Source>> {
    anyhow::ensure!(
        screen_permission(),
        "Allow Screen Recording to choose a display or window."
    );
    let content = SCShareableContent::get().context("Cannot list screen sources")?;
    let mut sources = Vec::new();
    for (index, display) in content.displays().iter().enumerate() {
        sources.push(Source {
            id: display.display_id() as u64,
            kind: SourceKind::Display,
            name: format!(
                "Display {} · {} × {}",
                index + 1,
                display.width(),
                display.height()
            ),
            bounds: bounds(display.frame()),
            pixel_width: display.width(),
            pixel_height: display.height(),
        });
    }
    let mut windows = Vec::new();
    for window in content.windows() {
        let Some(app) = window.owning_application() else {
            continue;
        };
        if app.process_id() == std::process::id() as i32 {
            continue;
        }
        if window.window_layer() != 0 || app.application_name().trim().is_empty() {
            continue;
        }
        let Some(title) = window.title().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let b = bounds(window.frame());
        if b.width < 100.0 || b.height < 60.0 {
            continue;
        }
        windows.push(Source {
            id: window.window_id() as u64,
            kind: SourceKind::Window,
            name: format!("{} · {}", app.application_name(), title),
            bounds: b,
            pixel_width: b.width as u32,
            pixel_height: b.height as u32,
        });
    }
    windows.sort_by(|a, b| a.name.cmp(&b.name));
    sources.extend(windows);
    Ok(sources)
}

struct Delegate(Arc<Mutex<Option<String>>>);
impl SCStreamDelegateTrait for Delegate {
    fn did_stop_with_error(&self, error: SCError) {
        *self.0.lock().unwrap() = Some(error.to_string());
    }
}

pub struct ScreenCapture {
    stream: SCStream,
    error: Arc<Mutex<Option<String>>>,
    bounds: Bounds,
    window_id: Option<u32>,
    current_bounds: Cell<Bounds>,
    bounds_updated: Cell<Instant>,
}
impl ScreenCapture {
    pub fn start(
        source: &Source,
        width: u32,
        height: u32,
        fps: u32,
        cursor: bool,
        latest: LatestFrame,
    ) -> Result<Self> {
        anyhow::ensure!(
            screen_permission(),
            "Screen Recording permission is required. Enable Loomik in System Settings → Privacy & Security → Screen & System Audio Recording."
        );
        let content = SCShareableContent::get()?;
        let (filter, source_bounds) = match source.kind {
            SourceKind::Display => {
                let display = content
                    .displays()
                    .into_iter()
                    .find(|d| d.display_id() as u64 == source.id)
                    .context("The selected display was disconnected. Choose another display.")?;
                let apps = content.applications();
                let own = apps.iter().find(|a| a.process_id()==std::process::id() as i32)
                    .context("Cannot identify Loomik's windows to exclude them. Keep the settings panel open and try again.")?;
                // Exclude the application, not a one-time list of window IDs:
                // new toolbar/camera/settings windows stay invisible to capture.
                (
                    SCContentFilter::create()
                        .with_display(&display)
                        .with_excluding_applications(&[own], &[])
                        .build(),
                    bounds(display.frame()),
                )
            }
            SourceKind::Window => {
                let window = content
                    .windows()
                    .into_iter()
                    .find(|w| w.window_id() as u64 == source.id)
                    .context("The selected window closed. Choose another window.")?;
                anyhow::ensure!(
                    !window
                        .owning_application()
                        .is_some_and(|a| a.process_id() == std::process::id() as i32),
                    "Loomik cannot record its own controls"
                );
                (
                    SCContentFilter::create().with_window(&window).build(),
                    bounds(window.frame()),
                )
            }
        };
        let config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(cursor)
            .with_queue_depth(3)
            .with_scales_to_fit(true)
            .with_minimum_frame_interval(&CMTime::from_seconds(1.0 / fps as f64, 600));
        let error = Arc::new(Mutex::new(None));
        let mut stream = SCStream::new_with_delegate(&filter, &config, Delegate(error.clone()));
        stream.add_output_handler(
            move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
                if kind != SCStreamOutputType::Screen {
                    return;
                }
                let Some(captured_at) = sample
                    .presentation_timestamp()
                    .as_seconds()
                    .and_then(super::time::host_instant)
                else {
                    return;
                };
                let Some(pixel) = sample.pixel_buffer() else {
                    return;
                };
                let Ok(lock) = pixel.lock(CVPixelBufferLockFlags::READ_ONLY) else {
                    return;
                };
                // SAFETY: the read-only lock keeps the native pixel storage valid for
                // this scope; from_strided copies it before the lock is released.
                if let Some(bytes) = unsafe { lock.as_slice() }
                    && let Some(frame) = VideoFrame::from_strided_at(
                        pixel.width() as u32,
                        pixel.height() as u32,
                        pixel.bytes_per_row(),
                        bytes,
                        captured_at,
                    )
                {
                    latest.set(frame);
                }
            },
            SCStreamOutputType::Screen,
        );
        stream
            .start_capture()
            .context("Cannot start screen capture")?;
        Ok(Self {
            stream,
            error,
            bounds: source_bounds,
            window_id: (source.kind == SourceKind::Window).then_some(source.id as u32),
            current_bounds: Cell::new(source_bounds),
            bounds_updated: Cell::new(Instant::now()),
        })
    }
    pub fn check(&self) -> Result<()> {
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            anyhow::bail!("Screen capture stopped: {error}");
        }
        Ok(())
    }
    pub fn bounds(&self) -> Bounds {
        let Some(window) = self.window_id else {
            return self.bounds;
        };
        // Query window geometry without fetching the expensive full SCK content
        // list. This keeps a moving/resized target aligned with the camera circle.
        if self.bounds_updated.get().elapsed() > Duration::from_millis(100) {
            if let Some(bounds) = window_bounds(window) {
                self.current_bounds.set(bounds);
            }
            self.bounds_updated.set(Instant::now());
        }
        self.current_bounds.get()
    }
}

fn window_bounds(window: u32) -> Option<Bounds> {
    use core_graphics::window::{CGWindowListCopyWindowInfo, kCGWindowListOptionIncludingWindow};
    unsafe {
        let raw = CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, window);
        if raw.is_null() {
            return None;
        }
        let array: CFArray<CFDictionary<CFString, CFType>> = TCFType::wrap_under_create_rule(raw);
        let info = array.get(0)?;
        let value = info.find(CFString::new("kCGWindowBounds"))?;
        let dict = value.downcast::<CFDictionary>()?;
        let rect = core_graphics::geometry::CGRect::from_dict_representation(&dict)?;
        Some(Bounds {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }
}
impl Drop for ScreenCapture {
    fn drop(&mut self) {
        let _ = self.stream.stop_capture();
    }
}
