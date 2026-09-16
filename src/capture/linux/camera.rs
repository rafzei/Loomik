use super::time::HostClock;
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
use v4l::{
    Device as VideoDevice, Format, FourCC,
    buffer::{Flags, Type},
    io::traits::CaptureStream,
    prelude::MmapStream,
    video::Capture,
};

pub fn discover_cameras() -> Result<Vec<Device>> {
    let mut devices = Vec::new();
    for entry in std::fs::read_dir("/dev")? {
        let path = entry?.path();
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("video"))
        {
            continue;
        }
        // Some V4L nodes expose metadata only, or belong to another user's session.
        let Ok(dev) = VideoDevice::with_path(&path) else {
            continue;
        };
        let Ok(caps) = dev.query_caps() else { continue };
        if caps
            .capabilities
            .contains(v4l::capability::Flags::VIDEO_CAPTURE | v4l::capability::Flags::STREAMING)
        {
            devices.push(Device {
                id: path.to_string_lossy().into_owned(),
                name: caps.card,
            });
        }
    }
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(devices)
}

pub struct Camera {
    pub frames: LatestFrame,
    pub events: Receiver<Result<(), String>>,
    stop: Arc<AtomicBool>,
}
impl Camera {
    pub fn start(id: String) -> Self {
        let frames = LatestFrame::default();
        let out = frames.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = stop.clone();
        let (tx, events) = bounded(4);
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let device = VideoDevice::with_path(&id).context(
                    "Cannot open camera. Check device access and whether another app is using it",
                )?;
                let formats = device.enum_formats()?;
                let fourcc = [FourCC::new(b"MJPG"), FourCC::new(b"YUYV")]
                    .into_iter()
                    .find(|code| formats.iter().any(|f| f.fourcc == *code))
                    .context("Camera needs an MJPEG or YUYV stream")?;
                let format = device.set_format(&Format::new(640, 480, fourcc))?;
                ensure!(
                    format.width > 0
                        && format.height > 0
                        && format.width <= 1920
                        && format.height <= 1080,
                    "Camera did not accept a realtime preview size"
                );
                let _ = device.set_params(&v4l::video::capture::Parameters::with_fps(30));
                ensure!(
                    format.fourcc == fourcc,
                    "Camera changed the requested pixel format"
                );
                use v4l::format::{Colorspace, Quantization};
                let rec709 = matches!(format.colorspace, Colorspace::Rec709);
                let full = matches!(format.quantization, Quantization::FullRange)
                    || (matches!(format.quantization, Quantization::Default)
                        && matches!(format.colorspace, Colorspace::JPEG));
                if fourcc == FourCC::new(b"YUYV") {
                    ensure!(
                        matches!(
                            format.colorspace,
                            Colorspace::Default
                                | Colorspace::SMPTE170M
                                | Colorspace::Rec709
                                | Colorspace::SRGB
                                | Colorspace::JPEG
                        ),
                        "Unsupported camera color space; choose an MJPEG camera mode"
                    );
                }
                let clock = HostClock::new()?;
                let mut stream = MmapStream::with_buffers(&device, Type::VideoCapture, 3)?;
                stream.set_timeout(Duration::from_millis(250));
                let mut last = Instant::now();
                let _ = tx.try_send(Ok(()));
                while !cancel.load(Ordering::Relaxed) {
                    let (bytes, metadata) = match stream.next() {
                        Ok(frame) => frame,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            ensure!(
                                last.elapsed() < Duration::from_secs(5),
                                "Camera stopped delivering frames"
                            );
                            continue;
                        }
                        Err(error) => {
                            return Err(error).context("Camera disconnected or capture failed");
                        }
                    };
                    if metadata.flags.contains(Flags::ERROR) {
                        continue;
                    }
                    ensure!(
                        metadata.flags & Flags::TIMESTAMP_MASK == Flags::TIMESTAMP_MONOTONIC,
                        "Camera does not provide monotonic timestamps required for synchronized recording"
                    );
                    let nanos = metadata
                        .timestamp
                        .sec
                        .checked_mul(1_000_000_000)
                        .and_then(|s| {
                            metadata
                                .timestamp
                                .usec
                                .checked_mul(1000)
                                .and_then(|ns| s.checked_add(ns))
                        })
                        .context("Invalid camera timestamp")?;
                    let captured_at = clock.instant(nanos)?;
                    let bytes = bytes
                        .get(..metadata.bytesused as usize)
                        .context("Truncated camera buffer")?;
                    let bgra = if fourcc == FourCC::new(b"MJPG") {
                        let image =
                            image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)?
                                .to_rgba8();
                        ensure!(
                            image.dimensions() == (format.width, format.height),
                            "Camera JPEG dimensions changed"
                        );
                        let mut pixels = image.into_raw();
                        for pixel in pixels.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }
                        pixels
                    } else {
                        crate::capture::linux_pixels::yuyv(
                            bytes,
                            format.width,
                            format.height,
                            format.stride as usize,
                            rec709,
                            full,
                        )?
                    };
                    out.set(VideoFrame {
                        width: format.width,
                        height: format.height,
                        bgra,
                        captured_at,
                    });
                    last = Instant::now();
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = tx.try_send(Err(format!("{error:#}")));
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
