use super::time::HostClock;
use crate::{
    model::{Bounds, Source, SourceKind},
    recording::frame::{LatestFrame, VideoFrame},
};
use anyhow::{Context as _, Result, ensure};
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use windows::{
    Graphics::Capture::GraphicsCaptureSession,
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
            Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
        },
        UI::WindowsAndMessaging::IsIconic,
    },
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::*,
    window::Window,
};

pub fn screen_permission() -> bool {
    let _apartment = super::time::Apartment::new().ok();
    crate::platform::windows::supported_version()
        && GraphicsCaptureSession::IsSupported().unwrap_or(false)
}
pub fn request_screen_permission() -> bool {
    screen_permission()
}
fn rect_bounds(r: RECT) -> Bounds {
    Bounds {
        x: r.left as f64,
        y: r.top as f64,
        width: (r.right - r.left) as f64,
        height: (r.bottom - r.top) as f64,
    }
}
fn window_bounds(id: u64) -> Result<Bounds> {
    let window = Window::from_raw_hwnd(id as usize as *mut _);
    ensure!(
        window.is_valid() && !unsafe { IsIconic(HWND(window.as_raw_hwnd())) }.as_bool(),
        "The selected window was closed or minimized"
    );
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            HWND(window.as_raw_hwnd()),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )?;
    }
    Ok(rect_bounds(rect))
}
pub fn discover_sources() -> Result<Vec<Source>> {
    ensure!(
        screen_permission(),
        "Windows Graphics Capture is unavailable. Loomik requires Windows 11."
    );
    let mut sources = Vec::new();
    for (i, monitor) in Monitor::enumerate()?.into_iter().enumerate() {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        ensure!(
            unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &mut info) }.as_bool(),
            "Cannot read monitor geometry"
        );
        let b = rect_bounds(info.rcMonitor);
        sources.push(Source {
            id: monitor.as_raw_hmonitor() as usize as u64,
            kind: SourceKind::Display,
            name: format!("Display {} · {} × {}", i + 1, b.width, b.height),
            bounds: b,
            pixel_width: b.width as u32,
            pixel_height: b.height as u32,
        });
    }
    for window in Window::enumerate()? {
        if window.process_id().ok() == Some(std::process::id()) {
            continue;
        }
        let title = window.title().unwrap_or_default();
        let id = window.as_raw_hwnd() as usize as u64;
        if title.trim().is_empty() {
            continue;
        }
        if let Ok(b) = window_bounds(id)
            && b.width >= 100.0
            && b.height >= 60.0
        {
            sources.push(Source {
                id,
                kind: SourceKind::Window,
                name: format!("{} · {title}", window.process_name().unwrap_or_default()),
                bounds: b,
                pixel_width: b.width as u32,
                pixel_height: b.height as u32,
            });
        }
    }
    Ok(sources)
}
struct Flags {
    latest: LatestFrame,
    width: u32,
    height: u32,
    fps: u32,
    error: Arc<Mutex<Option<String>>>,
}
struct NativeItem {
    kind: SourceKind,
    id: u64,
}
impl TryInto<GraphicsCaptureItemType> for NativeItem {
    type Error = anyhow::Error;
    fn try_into(self) -> Result<GraphicsCaptureItemType> {
        // The library performs this conversion on its initialized WinRT thread.
        Ok(match self.kind {
            SourceKind::Display => {
                Monitor::from_raw_hmonitor(self.id as usize as *mut _).try_into()?
            }
            SourceKind::Window => Window::from_raw_hwnd(self.id as usize as *mut _).try_into()?,
        })
    }
}
struct Handler {
    flags: Flags,
    clock: HostClock,
    last: Option<Instant>,
    resizer: fast_image_resize::Resizer,
    packed: Vec<u8>,
}
impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = anyhow::Error;
    fn new(ctx: Context<Flags>) -> Result<Self> {
        Ok(Self {
            flags: ctx.flags,
            clock: HostClock::new()?,
            last: None,
            resizer: fast_image_resize::Resizer::new(),
            packed: Vec::new(),
        })
    }
    fn on_frame_arrived(&mut self, frame: &mut Frame, _: InternalCaptureControl) -> Result<()> {
        let result = (|| -> Result<()> {
            // Reject unprotected windows before publishing any pixels.
            crate::platform::windows::verify_exclusion()?;
            let captured_at = self.clock.instant(frame.timestamp()?.Duration)?;
            if self.last.is_some_and(|last| {
                captured_at.saturating_duration_since(last)
                    < Duration::from_secs_f64(0.95 / self.flags.fps as f64)
            }) {
                return Ok(());
            }
            let mut buffer = frame.buffer()?;
            let (width, height, stride) =
                (buffer.width(), buffer.height(), buffer.row_pitch() as usize);
            let raw = buffer.as_raw_buffer();
            let bytes = if stride == width as usize * 4 {
                &raw[..width as usize * height as usize * 4]
            } else {
                self.packed.clear();
                for row in raw.chunks(stride).take(height as usize) {
                    self.packed.extend_from_slice(&row[..width as usize * 4]);
                }
                &self.packed
            };
            let (w, h) = (self.flags.width, self.flags.height);
            let bgra = if (width, height) == (w, h) {
                bytes.to_vec()
            } else {
                use fast_image_resize::{
                    FilterType, PixelType, ResizeAlg, ResizeOptions,
                    images::{Image, ImageRef},
                };
                let source = ImageRef::new(width, height, bytes, PixelType::U8x4)?;
                let mut output = Image::new(w, h, PixelType::U8x4);
                self.resizer.resize(
                    &source,
                    &mut output,
                    &ResizeOptions::new()
                        .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                        .use_alpha(false),
                )?;
                output.into_vec()
            };
            self.flags.latest.set(VideoFrame {
                width: w,
                height: h,
                bgra,
                captured_at,
            });
            self.last = Some(captured_at);
            Ok(())
        })();
        if let Err(e) = &result {
            *self.flags.error.lock().unwrap() = Some(format!("{e:#}"));
        }
        result
    }
    fn on_closed(&mut self) -> Result<()> {
        *self.flags.error.lock().unwrap() =
            Some("The captured display or window was closed".into());
        Ok(())
    }
}
pub struct ScreenCapture {
    control: Option<CaptureControl<Handler, anyhow::Error>>,
    source: Source,
    error: Arc<Mutex<Option<String>>>,
    bounds: Cell<Bounds>,
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
        crate::platform::windows::verify_exclusion()?;
        let error = Arc::new(Mutex::new(None));
        let flags = Flags {
            latest,
            width,
            height,
            fps,
            error: error.clone(),
        };
        let item = NativeItem {
            kind: source.kind,
            id: source.id,
        };
        let settings = Settings::new(
            item,
            if cursor {
                CursorCaptureSettings::WithCursor
            } else {
                CursorCaptureSettings::WithoutCursor
            },
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            flags,
        );
        let control = Handler::start_free_threaded(settings)
            .context("Cannot start Windows Graphics Capture")?;
        Ok(Self {
            control: Some(control),
            source: source.clone(),
            error,
            bounds: Cell::new(source.bounds),
        })
    }
    pub fn check(&self) -> Result<()> {
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            anyhow::bail!("{error}");
        }
        ensure!(
            !self.control.as_ref().is_none_or(|c| c.is_finished()),
            "Windows screen capture stopped unexpectedly"
        );
        if self.source.kind == SourceKind::Window {
            self.bounds.set(window_bounds(self.source.id)?);
        }
        Ok(())
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds.get()
    }
}
impl Drop for ScreenCapture {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
    }
}
