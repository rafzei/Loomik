use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[default]
    Mp4,
    Mov,
    Mkv,
}

impl Format {
    pub const ALL: [Self; 3] = [Self::Mp4, Self::Mov, Self::Mkv];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Mkv => "mkv",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mov => "MOV",
            Self::Mkv => "MKV",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Compact,
    #[default]
    Balanced,
    Crisp,
}
impl Quality {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Balanced, Self::Crisp];
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact · 720p",
            Self::Balanced => "Balanced · 1080p",
            Self::Crisp => "Crisp · up to 4K",
        }
    }
    pub fn dimensions(self, width: u32, height: u32) -> (u32, u32) {
        let limit = match self {
            Self::Compact => 1280.0,
            Self::Balanced => 1920.0,
            Self::Crisp => 3840.0,
        };
        let scale = (limit / width.max(height).max(1) as f64).min(1.0);
        (
            ((width as f64 * scale) as u32 / 2 * 2).max(2),
            ((height as f64 * scale) as u32 / 2 * 2).max(2),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub format: Format,
    pub quality: Quality,
    pub fps: u32,
    pub output_dir: PathBuf,
    pub show_cursor: bool,
    pub mirror_camera: bool,
    pub camera_size: f32,
}
impl Default for Settings {
    fn default() -> Self {
        let root = directories::UserDirs::new()
            .map(|d| d.video_dir().unwrap_or(d.home_dir()).to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            format: Format::Mp4,
            quality: Quality::Balanced,
            fps: 30,
            output_dir: root.join("Loomik"),
            show_cursor: true,
            mirror_camera: true,
            camera_size: 200.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Display,
    Window,
}

/// Desktop coordinates: macOS points, Windows physical pixels; media uses canvas pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub id: u64,
    pub kind: SourceKind,
    pub name: String,
    pub bounds: Bounds,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

#[derive(Debug, Clone)]
pub enum RecordingSource {
    Desktop(Source),
    Media(crate::media::MediaSource),
}
impl RecordingSource {
    pub fn dimensions(&self, quality: Quality) -> (u32, u32) {
        match self {
            Self::Desktop(s) => quality.dimensions(s.pixel_width, s.pixel_height),
            Self::Media(s) => s.dimensions(quality),
        }
    }
    pub fn requires_screen_permission(&self) -> bool {
        matches!(self, Self::Desktop(_))
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct CameraPlacement {
    pub x: f64,
    pub y: f64,
    pub diameter: f64,
    pub mirror: bool,
    pub visible: bool,
}
impl Default for CameraPlacement {
    fn default() -> Self {
        Self {
            x: 60.0,
            y: 520.0,
            diameter: 200.0,
            mirror: true,
            visible: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Recording,
    Paused,
    Stopping,
}

/// Each digit must get its own visible interval, even after a UI stall.
/// Zero remains visible briefly before the shared media clock is started.
#[derive(Debug)]
pub struct Countdown {
    digit: u8,
    shown_at: Option<Instant>,
}
impl Default for Countdown {
    fn default() -> Self {
        Self {
            digit: 3,
            shown_at: None,
        }
    }
}
impl Countdown {
    pub fn tick(&mut self, now: Instant) -> Option<u8> {
        let shown_at = self.shown_at.get_or_insert(now);
        let interval = if self.digit == 0 {
            Duration::from_millis(300)
        } else {
            Duration::from_secs(1)
        };
        if now.saturating_duration_since(*shown_at) >= interval {
            if self.digit == 0 {
                return None;
            }
            self.digit -= 1;
            *shown_at = now;
        }
        Some(self.digit)
    }
    pub fn digit(&self) -> u8 {
        self.digit
    }
}
impl Phase {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Recording | Self::Paused | Self::Stopping
        )
    }
}

/// A shared host-time timeline. Closed intervals preserve acquisition-time
/// mapping for callbacks delivered after pause/resume and for delayed encoders.
#[derive(Debug, Default, Clone)]
pub struct RecordingClock {
    intervals: Vec<ActiveInterval>,
}
#[derive(Debug, Clone)]
struct ActiveInterval {
    start: Instant,
    end: Option<Instant>,
    offset: Duration,
}
#[derive(Debug)]
pub struct ActiveSlice {
    pub start: Instant,
    pub end: Instant,
    pub output_start: Duration,
}
impl RecordingClock {
    pub fn begin(&mut self, now: Instant) {
        if self.intervals.is_empty() {
            self.start(now);
        }
    }
    pub fn start(&mut self, now: Instant) {
        self.intervals.clear();
        self.intervals.push(ActiveInterval {
            start: now,
            end: None,
            offset: Duration::ZERO,
        });
    }
    pub fn pause(&mut self, now: Instant) {
        if let Some(interval) = self.intervals.last_mut() {
            interval.end.get_or_insert(now.max(interval.start));
        }
    }
    pub fn resume(&mut self, now: Instant) {
        if !self.running() {
            let offset = self.elapsed(now);
            self.intervals.push(ActiveInterval {
                start: now,
                end: None,
                offset,
            });
        }
    }
    pub fn elapsed(&self, now: Instant) -> Duration {
        self.intervals
            .last()
            .map(|i| i.offset + i.end.unwrap_or(now).saturating_duration_since(i.start))
            .unwrap_or_default()
    }
    pub fn running(&self) -> bool {
        self.intervals.last().is_some_and(|i| i.end.is_none())
    }
    /// Output slot -> acquisition time, including slots encoded after a pause.
    pub fn instant_at(&self, position: Duration) -> Option<Instant> {
        self.intervals.iter().rev().find_map(|i| {
            if position < i.offset {
                return None;
            }
            let time = i.start.checked_add(position - i.offset)?;
            i.end.is_none_or(|end| time < end).then_some(time)
        })
    }
    /// Clip audio at start/pause/resume/stop at sample precision, even when a
    /// single native buffer straddles multiple boundaries.
    pub fn active_slices(&self, start: Instant, end: Instant) -> Vec<ActiveSlice> {
        self.intervals
            .iter()
            .rev()
            .take_while(|i| i.end.is_none_or(|e| e > start))
            .filter_map(|i| {
                let a = start.max(i.start);
                let b = end.min(i.end.unwrap_or(end));
                (b > a).then(|| ActiveSlice {
                    start: a,
                    end: b,
                    output_start: i.offset + a.duration_since(i.start),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

pub fn format_duration(time: Duration) -> String {
    let s = time.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_shows_every_digit_and_holds_zero_before_beginning() {
        let base = Instant::now();
        let mut countdown = Countdown::default();
        assert_eq!(countdown.tick(base), Some(3));
        assert_eq!(countdown.tick(base + Duration::from_millis(999)), Some(3));
        assert_eq!(countdown.tick(base + Duration::from_secs(1)), Some(2));
        assert_eq!(countdown.tick(base + Duration::from_secs(2)), Some(1));
        assert_eq!(countdown.tick(base + Duration::from_secs(3)), Some(0));
        assert_eq!(countdown.tick(base + Duration::from_millis(3299)), Some(0));
        assert_eq!(countdown.tick(base + Duration::from_millis(3300)), None);
        let mut stalled = Countdown::default();
        assert_eq!(stalled.tick(base), Some(3));
        assert_eq!(stalled.tick(base + Duration::from_secs(15)), Some(2));
        assert_eq!(stalled.tick(base + Duration::from_secs(16)), Some(1));
    }

    #[test]
    fn preroll_is_excluded_from_video_and_audio_and_begin_is_idempotent() {
        let base = Instant::now();
        let mut clock = RecordingClock::default();
        assert_eq!(
            clock.elapsed(base + Duration::from_secs(10)),
            Duration::ZERO
        );
        assert!(clock.instant_at(Duration::ZERO).is_none());
        assert!(
            clock
                .active_slices(base, base + Duration::from_secs(10))
                .is_empty()
        );
        let begin = base + Duration::from_secs(10);
        clock.begin(begin);
        clock.begin(begin + Duration::from_secs(1));
        assert_eq!(clock.instant_at(Duration::ZERO), Some(begin));
        let slices = clock.active_slices(base, begin + Duration::from_millis(10));
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].start, begin);
        assert_eq!(slices[0].output_start, Duration::ZERO);
    }
    #[test]
    fn paused_time_never_advances_and_hours_are_unbounded() {
        let now = Instant::now();
        let mut c = RecordingClock::default();
        c.start(now);
        c.pause(now + Duration::from_secs(12));
        assert_eq!(c.elapsed(now + Duration::from_secs(90)).as_secs(), 12);
        c.resume(now + Duration::from_secs(90));
        assert_eq!(c.elapsed(now + Duration::from_secs(100)).as_secs(), 22);
        assert_eq!(format_duration(Duration::from_secs(360000)), "100:00:00");
    }
    #[test]
    fn duplicate_pause_and_resume_are_idempotent() {
        let t = Instant::now();
        let mut c = RecordingClock::default();
        c.start(t);
        c.pause(t + Duration::from_secs(4));
        c.pause(t + Duration::from_secs(8));
        c.resume(t + Duration::from_secs(10));
        c.resume(t + Duration::from_secs(12));
        assert_eq!(c.elapsed(t + Duration::from_secs(20)).as_secs(), 14);
    }
    #[test]
    fn dimensions_preserve_aspect_and_are_encoder_safe() {
        assert_eq!(Quality::Balanced.dimensions(3840, 2160), (1920, 1080));
        assert_eq!(Quality::Balanced.dimensions(2160, 3840), (1080, 1920));
        assert_eq!(Quality::Compact.dimensions(999, 601), (998, 600));
    }
}

#[cfg(test)]
mod timeline_tests {
    use super::*;
    #[test]
    fn output_slots_keep_their_acquisition_time_after_pause_and_encoder_stall() {
        let base = Instant::now();
        let mut clock = RecordingClock::default();
        clock.start(base);
        clock.pause(base + Duration::from_secs(2));
        clock.resume(base + Duration::from_secs(9));
        assert_eq!(
            clock.instant_at(Duration::from_millis(1999)),
            Some(base + Duration::from_millis(1999))
        );
        assert_eq!(
            clock.instant_at(Duration::from_secs(2)),
            Some(base + Duration::from_secs(9))
        );
        assert_eq!(
            clock.instant_at(Duration::from_millis(2010)),
            Some(base + Duration::from_millis(9010))
        );
    }
}
