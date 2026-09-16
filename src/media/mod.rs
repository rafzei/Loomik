//! File-backed canvases. No desktop API or screen permission is used here.
mod reader;
use crate::{
    model::{Bounds, Quality},
    recording::encoder::{ffprobe_path, media_command},
};
use anyhow::{Context, Result, ensure};
use image::ImageDecoder;
pub use reader::Reader;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Video,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Aspect {
    #[default]
    Original,
    Landscape,
    Portrait,
    Square,
}
impl Aspect {
    pub const ALL: [Self; 4] = [
        Self::Original,
        Self::Landscape,
        Self::Portrait,
        Self::Square,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Landscape => "16:9",
            Self::Portrait => "9:16",
            Self::Square => "1:1",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Fit {
    #[default]
    Fit,
    Fill,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub has_audio: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Options {
    pub aspect: Aspect,
    pub fit: Fit,
    pub start: f64,
    pub loop_video: bool,
    pub source_audio: bool,
    pub source_volume: f32,
    pub microphone_volume: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            aspect: Aspect::Original,
            fit: Fit::Fit,
            start: 0.0,
            loop_video: false,
            source_audio: false,
            source_volume: 1.0,
            microphone_volume: 1.0,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSource {
    pub info: MediaInfo,
    pub options: Options,
}
impl MediaSource {
    pub fn dimensions(&self, quality: Quality) -> (u32, u32) {
        let (w, h) = match self.options.aspect {
            Aspect::Original => (self.info.width, self.info.height),
            Aspect::Landscape => (3840, 2160),
            Aspect::Portrait => (2160, 3840),
            Aspect::Square => (3840, 3840),
        };
        quality.dimensions(w, h)
    }
    pub fn bounds(&self, quality: Quality) -> Bounds {
        let (w, h) = self.dimensions(quality);
        Bounds {
            x: 0.0,
            y: 0.0,
            width: w as f64,
            height: h as f64,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.info.path.is_file(),
            "The background file is missing or unreadable."
        );
        ensure!(
            self.options.start.is_finite() && self.options.start >= 0.0,
            "Invalid starting position"
        );
        ensure!(
            self.info.kind == MediaKind::Image || self.options.start < self.info.duration,
            "The starting position must be before the end of the video."
        );
        for v in [self.options.source_volume, self.options.microphone_volume] {
            ensure!(
                v.is_finite() && (0.0..=2.0).contains(&v),
                "Invalid audio level"
            );
        }
        Ok(())
    }
    /// Identical input options for video decoding and source-audio finalization.
    pub fn ffmpeg_input(&self, command: &mut Command) {
        if self.options.loop_video {
            command.args(["-stream_loop", "-1"]);
        }
        command
            .args([
                "-ss",
                &format!("{:.9}", self.options.start),
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(&self.info.path);
    }
}

pub fn probe(path: &Path, kind: MediaKind) -> Result<MediaInfo> {
    ensure!(path.is_file(), "Choose a readable local media file.");
    let path = path
        .canonicalize()
        .context("Cannot open the background file")?;
    if kind == MediaKind::Image {
        let mut decoder = image::ImageReader::open(&path)?
            .with_guessed_format()?
            .into_decoder()?;
        let (mut width, mut height) = decoder.dimensions();
        ensure!(
            width as u64 * height as u64 <= 60_000_000,
            "Image is too large (maximum 60 megapixels)."
        );
        let orientation = decoder.orientation()?;
        if matches!(
            orientation,
            image::metadata::Orientation::Rotate90
                | image::metadata::Orientation::Rotate270
                | image::metadata::Orientation::Rotate90FlipH
                | image::metadata::Orientation::Rotate270FlipH
        ) {
            std::mem::swap(&mut width, &mut height);
        }
        return Ok(MediaInfo {
            path,
            kind,
            width,
            height,
            duration: 0.0,
            has_audio: false,
        });
    }
    let output=media_command(ffprobe_path()?).args(["-v","error","-protocol_whitelist","file,pipe","-show_entries","stream=codec_type,width,height,sample_aspect_ratio:stream_side_data=rotation:format=duration","-of","json"]).arg(&path).output().context("Cannot inspect the video")?;
    ensure!(
        output.status.success(),
        "Cannot read this video: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let streams = value["streams"]
        .as_array()
        .context("No media streams found")?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .context("The file has no video stream")?;
    let mut width = video["width"].as_u64().context("Missing video width")? as u32;
    let mut height = video["height"].as_u64().context("Missing video height")? as u32;
    if let Some((a, b)) = video["sample_aspect_ratio"]
        .as_str()
        .and_then(|s| s.split_once(':'))
        && let (Ok(a), Ok(b)) = (a.parse::<f64>(), b.parse::<f64>())
        && a > 0.0
        && b > 0.0
    {
        width = (width as f64 * a / b).round() as u32;
    }
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|a| a.iter().find_map(|v| v["rotation"].as_i64()))
        .unwrap_or(0);
    if rotation.rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .context("The video has no finite duration")?;
    ensure!(
        width > 0
            && height > 0
            && width <= 32768
            && height <= 32768
            && duration.is_finite()
            && duration > 0.0,
        "Invalid video dimensions or duration"
    );
    Ok(MediaInfo {
        path,
        kind,
        width,
        height,
        duration,
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}
