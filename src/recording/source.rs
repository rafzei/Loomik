use crate::{
    capture, media,
    model::{Bounds, RecordingSource},
    recording::frame::{LatestFrame, VideoFrame},
};
use anyhow::Result;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Frame coordinates belong to the source: desktop points or a file canvas.
pub trait FrameSource {
    fn check(&self) -> Result<()>;
    fn ready(&self) -> bool;
    fn bounds(&self) -> Bounds;
    fn frame_at(&mut self, target: Instant) -> Result<Option<Arc<VideoFrame>>>;
}
pub enum Input {
    Camera {
        frames: LatestFrame,
        bounds: Bounds,
    },
    Desktop {
        native: capture::ScreenCapture,
        frames: LatestFrame,
    },
    Media {
        reader: media::Reader,
        first: Option<Arc<VideoFrame>>,
        bounds: Bounds,
    },
}
impl Input {
    pub fn open(
        source: &RecordingSource,
        width: u32,
        height: u32,
        fps: u32,
        cursor: bool,
        frames: LatestFrame,
    ) -> Result<Self> {
        match source {
            RecordingSource::Camera { .. } => Ok(Self::Camera {
                frames,
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: width as f64,
                    height: height as f64,
                },
            }),
            RecordingSource::Desktop(s) => Ok(Self::Desktop {
                native: capture::ScreenCapture::start(
                    s,
                    width,
                    height,
                    fps,
                    cursor,
                    frames.clone(),
                )?,
                frames,
            }),
            RecordingSource::Media(s) => {
                let mut reader = media::Reader::open(s, width, height, fps)?;
                let first = Some(reader.next_frame()?);
                Ok(Self::Media {
                    reader,
                    first,
                    bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: width as f64,
                        height: height as f64,
                    },
                })
            }
        }
    }
}
impl FrameSource for Input {
    fn check(&self) -> Result<()> {
        match self {
            Self::Camera { frames, .. } => {
                anyhow::ensure!(
                    frames
                        .get()
                        .is_some_and(|frame| frame.captured_at.elapsed() < Duration::from_secs(2)),
                    "Camera stopped delivering frames. Check that it is connected and available."
                );
                Ok(())
            }
            Self::Desktop { native, .. } => native.check(),
            Self::Media { .. } => Ok(()),
        }
    }
    fn ready(&self) -> bool {
        match self {
            Self::Desktop { frames, .. } | Self::Camera { frames, .. } => frames.get().is_some(),
            Self::Media { .. } => true,
        }
    }
    fn bounds(&self) -> Bounds {
        match self {
            Self::Desktop { native, .. } => native.bounds(),
            Self::Media { bounds, .. } | Self::Camera { bounds, .. } => *bounds,
        }
    }
    fn frame_at(&mut self, target: Instant) -> Result<Option<Arc<VideoFrame>>> {
        match self {
            Self::Desktop { frames, .. } | Self::Camera { frames, .. } => {
                Ok(frames.at_or_before(target))
            }
            Self::Media { reader, first, .. } => Ok(Some(match first.take() {
                Some(f) => f,
                None => reader.next_frame()?,
            })),
        }
    }
}
