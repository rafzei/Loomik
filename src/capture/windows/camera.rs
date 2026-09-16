use super::time::{Apartment, HostClock};
use crate::{
    model::Device,
    recording::frame::{LatestFrame, VideoFrame},
};
use anyhow::{Context, Result, ensure};
use crossbeam_channel::{Receiver, bounded};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Devices::Enumeration::{DeviceClass, DeviceInformation},
    Foundation::{IClosable, TypedEventHandler},
    Graphics::Imaging::{BitmapBufferAccessMode, BitmapPixelFormat, BitmapSize},
    Media::Capture::{
        Frames::{
            MediaFrameArrivedEventArgs, MediaFrameReader, MediaFrameReaderAcquisitionMode,
            MediaFrameReaderStartStatus, MediaFrameSourceKind,
        },
        MediaCapture, MediaCaptureInitializationSettings, MediaCaptureMemoryPreference,
        StreamingCaptureMode,
    },
    Win32::System::WinRT::IMemoryBufferByteAccess,
    core::Interface,
};

struct Close(IClosable);
impl Close {
    fn new<T: Interface>(object: &T) -> Result<Self> {
        Ok(Self(object.cast()?))
    }
}
impl Drop for Close {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
pub fn discover_cameras() -> Result<Vec<Device>> {
    let _apartment = Apartment::new()?;
    let list = DeviceInformation::FindAllAsyncDeviceClass(DeviceClass::VideoCapture)?.join()?;
    list.into_iter()
        .map(|d| {
            Ok(Device {
                id: d.Id()?.to_string(),
                name: d.Name()?.to_string(),
            })
        })
        .collect()
}
pub struct Camera {
    pub frames: LatestFrame,
    pub events: Receiver<Result<(), String>>,
    stop: Arc<AtomicBool>,
}
impl Camera {
    pub fn start(id: String) -> Self {
        let frames = LatestFrame::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (events_tx, events) = bounded(4);
        let (out, cancel) = (frames.clone(), stop.clone());
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let _apartment = Apartment::new()?;
                let settings = MediaCaptureInitializationSettings::new()?;
                settings.SetVideoDeviceId(&id.into())?;
                settings.SetStreamingCaptureMode(StreamingCaptureMode::Video)?;
                settings.SetMemoryPreference(MediaCaptureMemoryPreference::Cpu)?;
                let capture = MediaCapture::new()?;
                let _close_capture = Close::new(&capture)?;
                capture.InitializeWithSettingsAsync(&settings)?.join().context("Cannot open camera. Enable Camera access for desktop apps in Windows Privacy settings, and close apps using it exclusively")?;
                let source = capture
                    .FrameSources()?
                    .into_iter()
                    .find_map(|entry| {
                        entry.Value().ok().filter(|s| {
                            s.Info().and_then(|i| i.SourceKind()).ok()
                                == Some(MediaFrameSourceKind::Color)
                        })
                    })
                    .context("Camera has no color video stream")?;
                // Prefer a fast 720p mode; never decode a 4K camera just to draw a circle.
                let mut formats = source
                    .SupportedFormats()?
                    .into_iter()
                    .filter_map(|f| {
                        let v = f.VideoFormat().ok()?;
                        let rate = f.FrameRate().ok()?;
                        let fps =
                            rate.Numerator().ok()? as f64 / rate.Denominator().ok()?.max(1) as f64;
                        let (w, h) = (v.Width().ok()?, v.Height().ok()?);
                        (w <= 1280 && h <= 960 && (15.0..=60.1).contains(&fps)).then_some((
                            fps,
                            w as u64 * h as u64,
                            f,
                        ))
                    })
                    .collect::<Vec<_>>();
                formats.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
                if let Some((_, _, format)) = formats.first() {
                    source.SetFormatAsync(format)?.join()?;
                }
                let video = source.CurrentFormat()?.VideoFormat()?;
                let (w, h) = (video.Width()?, video.Height()?);
                ensure!(w > 0 && h > 0, "Invalid camera dimensions");
                let scale = (1280.0 / w as f64).min(720.0 / h as f64).min(1.0);
                let reader = capture
                    .CreateFrameReaderWithSubtypeAndSizeAsync(
                        &source,
                        &"BGRA8".into(),
                        BitmapSize {
                            Width: (w as f64 * scale).round().max(2.0) as u32,
                            Height: (h as f64 * scale).round().max(2.0) as u32,
                        },
                    )?
                    .join()?;
                let _close_reader = Close::new(&reader)?;
                reader.SetAcquisitionMode(MediaFrameReaderAcquisitionMode::Realtime)?;
                let clock = HostClock::new()?;
                let failed = Arc::new(AtomicBool::new(false));
                let (tx, bad) = (events_tx.clone(), failed.clone());
                let watch = out.clone();
                let arrived = reader.FrameArrived(&TypedEventHandler::<
                    MediaFrameReader,
                    MediaFrameArrivedEventArgs,
                >::new(move |reader, _| {
                    if bad.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let Some(reader) = reader.as_ref() else {
                        return Ok(());
                    };
                    // A coalesced event may have no frame left; that is not a device error.
                    let Ok(frame) = reader.TryAcquireLatestFrame() else {
                        return Ok(());
                    };
                    let result = (|| -> Result<()> {
                        let _close_frame = Close::new(&frame)?;
                        let timestamp =
                            clock.instant(frame.SystemRelativeTime()?.Value()?.Duration)?;
                        let bitmap = frame.VideoMediaFrame()?.SoftwareBitmap()?;
                        let _close_bitmap = Close::new(&bitmap)?;
                        ensure!(
                            bitmap.BitmapPixelFormat()? == BitmapPixelFormat::Bgra8,
                            "Camera did not supply BGRA pixels"
                        );
                        let (w, h) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
                        let buffer = bitmap.LockBuffer(BitmapBufferAccessMode::Read)?;
                        let _close_buffer = Close::new(&buffer)?;
                        let plane = buffer.GetPlaneDescription(0)?;
                        let reference = buffer.CreateReference()?;
                        let _close_reference = Close::new(&reference)?;
                        let access: IMemoryBufferByteAccess = reference.cast()?;
                        let (mut pointer, mut length) = (std::ptr::null_mut(), 0);
                        unsafe {
                            access.GetBuffer(&mut pointer, &mut length)?;
                        }
                        ensure!(
                            !pointer.is_null() && plane.StartIndex >= 0 && plane.Stride > 0,
                            "Invalid camera buffer layout"
                        );
                        let start = plane.StartIndex as usize;
                        let size = (plane.Stride as usize)
                            .checked_mul(h as usize)
                            .context("Camera buffer overflow")?;
                        ensure!(
                            start
                                .checked_add(size)
                                .is_some_and(|end| end <= length as usize),
                            "Truncated camera buffer"
                        );
                        // Buffer/reference/bitmap stay alive and locked while rows are copied.
                        let bytes = unsafe { std::slice::from_raw_parts(pointer.add(start), size) };
                        let video = VideoFrame::from_strided_at(
                            w,
                            h,
                            plane.Stride as usize,
                            bytes,
                            timestamp,
                        )
                        .context("Invalid camera rows")?;
                        out.set(video);
                        Ok(())
                    })();
                    if let Err(error) = result {
                        bad.store(true, Ordering::Relaxed);
                        let _ = tx.try_send(Err(format!("{error:#}")));
                    }
                    Ok(())
                }))?;
                ensure!(
                    reader.StartAsync()?.join()? == MediaFrameReaderStartStatus::Success,
                    "Camera stream could not start. Check Windows Camera privacy access and device availability."
                );
                let _ = events_tx.try_send(Ok(()));
                let started = Instant::now();
                let mut stalled = false;
                while !cancel.load(Ordering::Relaxed) && !failed.load(Ordering::Relaxed) {
                    // A disconnected/blocked device can stop producing frames without an event.
                    if watch
                        .get()
                        .map_or(started.elapsed() > Duration::from_secs(10), |f| {
                            f.captured_at.elapsed() > Duration::from_secs(3)
                        })
                    {
                        stalled = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                reader.RemoveFrameArrived(arrived)?;
                reader.StopAsync()?.join()?;
                ensure!(
                    !stalled,
                    "Camera stopped delivering frames. Reconnect it and check Windows Camera privacy access."
                );
                Ok(())
            })();
            if let Err(error) = result {
                let _ = events_tx.try_send(Err(format!("{error:#}")));
            }
        });
        Self {
            frames,
            events,
            stop,
        }
    }
}
impl Drop for Camera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
