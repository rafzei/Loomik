use crate::model::{Format, Quality};
use anyhow::{Context, Result, bail};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

pub fn ffmpeg_path() -> Result<PathBuf> {
    tool_path("ffmpeg", "LOOMIK_FFMPEG")
}
pub fn ffprobe_path() -> Result<PathBuf> {
    tool_path("ffprobe", "LOOMIK_FFPROBE")
}
/// Native GUI builds must not create FFmpeg console windows over the desktop.
pub fn media_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    // Keep the same builder on non-Windows platforms.
    let _ = &mut command;
    command
}
fn tool_path(name: &str, variable: &str) -> Result<PathBuf> {
    let executable = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os(variable) {
        candidates.push(PathBuf::from(path));
    }
    if name == "ffprobe"
        && let Ok(ffmpeg) = ffmpeg_path()
        && let Some(parent) = ffmpeg.parent()
    {
        candidates.push(parent.join(&executable));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&executable));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin").join(&executable),
        PathBuf::from("/usr/local/bin").join(&executable),
        PathBuf::from(&executable),
    ]);
    for path in candidates {
        if media_command(&path)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Ok(path);
        }
    }
    bail!(
        "{name} is missing. Install FFmpeg (including ffprobe), add its bin folder to PATH, or set {variable}, then reopen Loomik."
    )
}

/// Probe a real hardware session at the requested dimensions. Merely listing
/// an FFmpeg codec does not prove that hardware encoding is available.
#[cfg(target_os = "windows")]
fn windows_encoder_options(backend: &str) -> &'static [&'static str] {
    match backend {
        "h264_nvenc" => &[
            "-preset",
            "p1",
            "-tune",
            "ull",
            "-rc",
            "cbr",
            "-rc-lookahead",
            "0",
            "-zerolatency",
            "1",
            "-delay",
            "0",
        ],
        "h264_qsv" => &[
            "-preset",
            "veryfast",
            "-async_depth",
            "1",
            "-look_ahead",
            "0",
        ],
        "h264_amf" => &["-usage", "ultralowlatency", "-quality", "speed"],
        _ => &[],
    }
}
fn choose_backend(ffmpeg: &Path, width: u32, height: u32) -> (&'static str, Option<String>) {
    if cfg!(target_os = "macos") {
        let result = media_command(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                &format!("color=size={width}x{height}:rate=30"),
                "-frames:v",
                "2",
                "-c:v",
                "h264_videotoolbox",
                "-allow_sw",
                "0",
                "-realtime",
                "1",
                "-bf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-f",
                "null",
                "-",
            ])
            .output();
        match result {
            Ok(output) if output.status.success() => return ("h264_videotoolbox", None),
            Ok(output) => {
                return (
                    "libx264",
                    Some(
                        String::from_utf8_lossy(&output.stderr)
                            .trim()
                            .chars()
                            .take(2000)
                            .collect(),
                    ),
                );
            }
            Err(error) => return ("libx264", Some(error.to_string())),
        }
    }
    #[cfg(target_os = "windows")]
    {
        let mut failures = Vec::new();
        for backend in ["h264_nvenc", "h264_qsv", "h264_amf"] {
            let result = media_command(ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-nostdin",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=size={width}x{height}:rate=30"),
                    "-frames:v",
                    "2",
                    "-c:v",
                    backend,
                ])
                .args(windows_encoder_options(backend))
                .args([
                    "-b:v", "4M", "-bf", "0", "-pix_fmt", "yuv420p", "-f", "null", "-",
                ])
                .output();
            match result {
                Ok(output) if output.status.success() => return (backend, None),
                Ok(output) => failures.push(format!(
                    "{backend}: {}",
                    String::from_utf8_lossy(&output.stderr)
                        .trim()
                        .chars()
                        .take(500)
                        .collect::<String>()
                )),
                Err(e) => failures.push(format!("{backend}: {e}")),
            }
        }
        ("libx264", Some(failures.join("\n")))
    }
    #[cfg(not(target_os = "windows"))]
    (
        "libx264",
        Some("Native hardware backend not implemented on this OS".into()),
    )
}

/// Writes fragmented MP4 continuously. Abrupt exits leave recoverable fragments.
pub struct Encoder {
    child: Child,
    input: Option<ChildStdin>,
    expected_size: usize,
    pub frame_count: u64,
    pub session: PathBuf,
    ffmpeg: PathBuf,
    fps: u32,
    finished: bool,
    pub backend: &'static str,
    pub fallback_reason: Option<String>,
}

impl Encoder {
    pub fn start(
        session: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        quality: Quality,
    ) -> Result<Self> {
        Self::start_using(session, width, height, fps, quality, ffmpeg_path()?)
    }
    fn start_using(
        session: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        quality: Quality,
        ffmpeg: PathBuf,
    ) -> Result<Self> {
        anyhow::ensure!(
            width > 0 && height > 0 && width.is_multiple_of(2) && height.is_multiple_of(2),
            "Video dimensions must be positive and even"
        );
        anyhow::ensure!([15, 30, 60].contains(&fps), "Unsupported frame rate");
        fs::create_dir_all(&session).context("Cannot create recording folder")?;
        let log = File::create(session.join("encoder.log"))?;
        let crf = match quality {
            Quality::Compact => "27",
            Quality::Balanced => "22",
            Quality::Crisp => "18",
        };
        let (backend, fallback_reason) = choose_backend(&ffmpeg, width, height);
        let mut command = media_command(&ffmpeg);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "warning",
                "-nostdin",
                "-n",
                "-f",
                "rawvideo",
                "-pixel_format",
                "bgra",
                "-video_size",
            ])
            .arg(format!("{width}x{height}"))
            .args([
                "-framerate",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                backend,
            ]);
        if backend == "h264_videotoolbox" {
            let bits_per_pixel = match quality {
                Quality::Compact => 0.09,
                Quality::Balanced => 0.14,
                Quality::Crisp => 0.22,
            };
            let bitrate = (width as f64 * height as f64 * fps as f64 * bits_per_pixel)
                .max(1_000_000.0) as u64;
            command.args([
                "-realtime",
                "1",
                "-allow_sw",
                "0",
                "-prio_speed",
                "1",
                "-b:v",
                &bitrate.to_string(),
            ]);
        } else if backend == "libx264" {
            command.args(["-preset", "veryfast", "-tune", "zerolatency", "-crf", crf]);
        }
        #[cfg(target_os = "windows")]
        if backend != "libx264" {
            let factor = match quality {
                Quality::Compact => 0.09,
                Quality::Balanced => 0.14,
                Quality::Crisp => 0.22,
            };
            let bitrate =
                (width as f64 * height as f64 * fps as f64 * factor).max(1_000_000.0) as u64;
            command
                .args(windows_encoder_options(backend))
                .args(["-b:v", &bitrate.to_string()]);
        }
        let mut child = command
            .args(["-bf", "0", "-pix_fmt", "yuv420p", "-g"])
            .arg((fps * 2).to_string())
            .args([
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
            ])
            .arg(session.join("video.mp4"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .context("Cannot start FFmpeg")?;
        let input = child
            .stdin
            .take()
            .context("FFmpeg input pipe was not created")?;
        Ok(Self {
            child,
            input: Some(input),
            expected_size: width as usize * height as usize * 4,
            frame_count: 0,
            session,
            ffmpeg,
            fps,
            finished: false,
            backend,
            fallback_reason,
        })
    }
    pub fn write_frame(&mut self, bytes: &[u8]) -> Result<()> {
        anyhow::ensure!(
            bytes.len() == self.expected_size,
            "Captured frame size changed unexpectedly"
        );
        self.input
            .as_mut()
            .context("Encoder is closed")?
            .write_all(bytes)
            .with_context(|| {
                format!(
                    "Video encoder stopped. Details: {}",
                    self.session.join("encoder.log").display()
                )
            })?;
        self.frame_count += 1;
        Ok(())
    }
    pub fn finish_video(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        if let Some(mut input) = self.input.take() {
            input.flush().context("Cannot flush video to encoder")?;
        }
        let status = self.child.wait().context("Cannot wait for video encoder")?;
        self.finished = true;
        if !status.success() {
            bail!(
                "FFmpeg failed: {}",
                fs::read_to_string(self.session.join("encoder.log")).unwrap_or_default()
            );
        }
        anyhow::ensure!(self.frame_count > 0, "No video frames were recorded");
        Ok(())
    }
    pub fn finalize(
        &mut self,
        output: &Path,
        format: Format,
        audio_rate: Option<u32>,
    ) -> Result<()> {
        self.finalize_with_media(output, format, audio_rate, None)
    }
    pub fn finalize_with_media(
        &mut self,
        output: &Path,
        format: Format,
        audio_rate: Option<u32>,
        media: Option<&crate::media::MediaSource>,
    ) -> Result<()> {
        self.finish_video()?;
        let temp = self
            .session
            .join(format!("finished.{}", format.extension()));
        let mut command = media_command(&self.ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i"])
            .arg(self.session.join("video.mp4"));
        let mut audio_tracks = Vec::new();
        let mut input_index = 1;
        if let Some(rate) = audio_rate {
            command
                .args(["-f", "f32le", "-ar", &rate.to_string(), "-ac", "1", "-i"])
                .arg(self.session.join("microphone.f32"));
            let gain = media.map_or(1.0, |m| m.options.microphone_volume);
            audio_tracks.push(format!("[{input_index}:a:0]volume={gain},apad[a0]"));
            input_index += 1;
        }
        if let Some(media) = media.filter(|m| m.info.has_audio && m.options.source_audio) {
            media.validate()?;
            media.ffmpeg_input(&mut command);
            audio_tracks.push(format!(
                "[{input_index}:a:0]aresample=48000:async=1:first_pts=0,volume={},apad[a{}]",
                media.options.source_volume,
                audio_tracks.len()
            ));
        }
        command.args(["-map", "0:v:0"]);
        if audio_tracks.is_empty() {
            command.arg("-an");
        } else {
            let labels = (0..audio_tracks.len())
                .map(|i| format!("[a{i}]"))
                .collect::<String>();
            let filter = format!(
                "{};{}amix=inputs={}:normalize=0:duration=longest,alimiter=limit=0.95:latency=1[mix]",
                audio_tracks.join(";"),
                labels,
                audio_tracks.len()
            );
            command.args([
                "-filter_complex",
                &filter,
                "-map",
                "[mix]",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
            ]);
        }
        command.args([
            "-c:v",
            "copy",
            "-t",
            &format!("{:.9}", self.frame_count as f64 / self.fps as f64),
        ]);
        if format != Format::Mkv {
            command.args(["-movflags", "+faststart"]);
        }
        let result = command
            .arg(&temp)
            .output()
            .context("Cannot finalize the recording")?;
        if !result.status.success() {
            bail!(
                "Export failed: {}. Recoverable files: {}",
                String::from_utf8_lossy(&result.stderr),
                self.session.display()
            );
        }
        // Hard-link is atomic and refuses to overwrite an existing file. Both
        // paths are on the same filesystem; a name collision never loses data.
        fs::hard_link(&temp, output).with_context(|| {
            format!(
                "Cannot save {}. Export remains at {}",
                output.display(),
                temp.display()
            )
        })?;
        Ok(())
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        self.input.take();
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn failed_hardware_probe_falls_back_to_decodable_software_video() {
        let folder = tempfile::tempdir().unwrap();
        let wrapper = folder.path().join("ffmpeg-no-hardware");
        let actual = ffmpeg_path().unwrap();
        let quoted = actual.to_string_lossy().replace('\'', "'\"'\"'");
        fs::write(&wrapper, format!("#!/bin/sh\nfor arg do\n if [ \"$arg\" = h264_videotoolbox ]; then echo 'Hardware unavailable in fixture' >&2; exit 1; fi\ndone\nexec '{quoted}' \"$@\"\n")).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let session = folder.path().join("session");
        let mut encoder =
            Encoder::start_using(session, 64, 48, 30, Quality::Balanced, wrapper).unwrap();
        assert_eq!(encoder.backend, "libx264");
        assert!(
            encoder
                .fallback_reason
                .as_ref()
                .unwrap()
                .contains("Hardware unavailable")
        );
        for _ in 0..30 {
            encoder
                .write_frame(&[20, 60, 200, 255].repeat(64 * 48))
                .unwrap();
        }
        let movie = folder.path().join("fallback.mp4");
        encoder.finalize(&movie, Format::Mp4, None).unwrap();
        let decode = Command::new(actual)
            .args(["-v", "error", "-i"])
            .arg(&movie)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output()
            .unwrap();
        assert!(decode.status.success());
        assert_eq!(decode.stdout.len(), 30 * 64 * 48 * 3);
        assert!(decode.stdout[0] > 180);
    }
}
