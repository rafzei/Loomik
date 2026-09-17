use super::*;
use crate::recording::{
    encoder::{ffmpeg_path, media_command},
    frame::VideoFrame,
};
use crossbeam_channel::{Receiver, bounded};
use std::{
    io::Read,
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub enum Reader {
    Image(Arc<VideoFrame>),
    Video(Decoder),
}
impl Reader {
    pub fn open(source: &MediaSource, width: u32, height: u32, fps: u32) -> Result<Self> {
        source.validate()?;
        ensure!(
            width >= 2
                && height >= 2
                && width <= 3840
                && height <= 3840
                && [15, 30, 60].contains(&fps),
            "Invalid media canvas"
        );
        if source.info.kind == MediaKind::Image {
            return Ok(Self::Image(Arc::new(read_image(source, width, height)?)));
        }
        Ok(Self::Video(Decoder::open(source, width, height, fps)?))
    }
    pub fn next_frame(&mut self) -> Result<Arc<VideoFrame>> {
        match self {
            Self::Image(frame) => Ok(frame.clone()),
            Self::Video(decoder) => decoder.next_frame(),
        }
    }
}

fn read_image(source: &MediaSource, width: u32, height: u32) -> Result<VideoFrame> {
    // Recheck the current file: it may have changed since the source was probed.
    let mut decoder = image_decoder(&source.info.path)?;
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let resized = match source.options.fit {
        Fit::Fit => image.resize(width, height, image::imageops::FilterType::Triangle),
        Fit::Fill => image.resize_to_fill(width, height, image::imageops::FilterType::Triangle),
    }
    .to_rgba8();
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([24, 25, 28, 255]));
    image::imageops::overlay(
        &mut canvas,
        &resized,
        ((width - resized.width()) / 2) as i64,
        ((height - resized.height()) / 2) as i64,
    );
    let mut bgra = canvas.into_raw();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok(VideoFrame {
        width,
        height,
        bgra,
        captured_at: Instant::now(),
    })
}

pub struct Decoder {
    child: Child,
    frames: Receiver<Result<Arc<VideoFrame>, String>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}
impl Decoder {
    fn open(source: &MediaSource, width: u32, height: u32, fps: u32) -> Result<Self> {
        let geometry = match source.options.fit {
            Fit::Fit => format!(
                "scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=0x18191c"
            ),
            Fit::Fill => format!(
                "scale={width}:{height}:force_original_aspect_ratio=increase:force_divisible_by=2,crop={width}:{height}"
            ),
        };
        // FFmpeg applies rotation metadata before these filters. fps converts
        // variable presentation timestamps, not packet/decode arrival order.
        let filter = format!(
            "scale=iw*sar:ih,setsar=1,{geometry},setsar=1,fps={fps}:start_time=0:round=up{}",
            if source.options.loop_video {
                ""
            } else {
                ",tpad=stop_mode=clone:stop=-1"
            }
        );
        let mut command = media_command(ffmpeg_path()?);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-threads",
            "2",
        ]);
        source.ffmpeg_input(&mut command);
        let mut child = command
            .args([
                "-map", "0:v:0", "-an", "-sn", "-dn", "-vf", &filter, "-pix_fmt", "bgra", "-f",
                "rawvideo", "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot start the video decoder")?;
        let mut stdout = child.stdout.take().context("Missing decoder pipe")?;
        let (tx, frames) = bounded(2);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let reader = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let mut bytes = vec![0; width as usize * height as usize * 4];
                let message = match stdout.read_exact(&mut bytes) {
                    Ok(()) => Ok(Arc::new(VideoFrame {
                        width,
                        height,
                        bgra: bytes,
                        captured_at: Instant::now(),
                    })),
                    Err(e) => Err(format!(
                        "The background video decoder stopped: {e}. Check that the file is readable and its codec is supported by FFmpeg."
                    )),
                };
                let failed = message.is_err();
                let mut pending = message;
                loop {
                    match tx.send_timeout(pending, Duration::from_millis(20)) {
                        Ok(()) => break,
                        Err(crossbeam_channel::SendTimeoutError::Timeout(m)) => {
                            pending = m;
                            if stopped.load(Ordering::Relaxed) {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
                if failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            frames,
            stop,
            reader: Some(reader),
        })
    }
    fn next_frame(&self) -> Result<Arc<VideoFrame>> {
        self.frames
            .recv_timeout(Duration::from_secs(10))
            .context("Background video decode timed out")?
            .map_err(anyhow::Error::msg)
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
