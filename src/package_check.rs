//! Explicit, non-GUI smoke check of an installed macOS/Windows distribution.
//! Synthetic pixels/audio only; does not request camera, mic or screen access.
use crate::{
    model::{Format, Quality},
    recording::encoder::{self, Encoder},
};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

pub fn run(output: &Path) -> Result<()> {
    // A fresh directory avoids overwriting any user recording or prior evidence.
    fs::create_dir(output).context("Package-check output must be a new directory")?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let parent = exe
        .parent()
        .context("Application has no parent directory")?;
    let ffmpeg = encoder::ffmpeg_path()?.canonicalize()?;
    let ffprobe = encoder::ffprobe_path()?.canonicalize()?;
    for tool in [&ffmpeg, &ffprobe] {
        ensure!(
            tool.parent() == Some(parent),
            "Package check requires bundled tools beside the app, found {}",
            tool.display()
        );
    }
    let mut results = Vec::new();
    for (index, format) in Format::ALL.into_iter().enumerate() {
        let session = output.join(format!("session-{index}"));
        let mut encoder = Encoder::start(session.clone(), 64, 48, 15, Quality::Balanced)?;
        for frame in 0..30 {
            let pixel = if frame < 15 {
                [0, 0, 240, 255]
            } else {
                [240, 0, 0, 255]
            };
            encoder.write_frame(&pixel.repeat(64 * 48))?;
        }
        let samples: Vec<u8> = (0..96000)
            .flat_map(|i| {
                ((i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.1).to_le_bytes()
            })
            .collect();
        fs::write(session.join("microphone.f32"), samples)?;
        let path = output.join(format!("package-check.{}", format.extension()));
        let backend = encoder.backend;
        encoder.finalize(&path, format, Some(48000))?;
        let probe = encoder::media_command(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(&path)
            .output()?;
        ensure!(
            probe.status.success(),
            "FFprobe failed: {}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let streams = metadata["streams"].as_array().context("Missing streams")?;
        ensure!(
            streams
                .iter()
                .any(|s| s["codec_name"] == "h264" && s["width"] == 64 && s["height"] == 48),
            "Missing H.264 video"
        );
        ensure!(
            streams.iter().any(|s| s["codec_name"] == "aac"),
            "Missing AAC audio"
        );
        let duration: f64 = metadata["format"]["duration"]
            .as_str()
            .context("Missing duration")?
            .parse()?;
        ensure!(
            (duration - 2.0).abs() < 0.08,
            "Incorrect recording duration: {duration}"
        );
        let decoded = encoder::media_command(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args([
                "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
            ])
            .output()?;
        ensure!(
            decoded.status.success() && decoded.stdout.len() == 30 * 64 * 48 * 3,
            "Cannot decode all saved frames"
        );
        let first = &decoded.stdout[..3];
        let last = &decoded.stdout[decoded.stdout.len() - 3..];
        ensure!(
            first[0] > 220 && first[2] < 20 && last[2] > 220 && last[0] < 20,
            "Decoded pixels do not match the recording"
        );
        let audio = encoder::media_command(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args([
                "-map", "0:a:0", "-f", "f32le", "-ac", "1", "-ar", "48000", "pipe:1",
            ])
            .output()?;
        ensure!(
            audio.status.success() && audio.stdout.len() >= 95000 * 4,
            "Cannot decode the recorded audio"
        );
        let peak = audio
            .stdout
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v).abs())
            .fold(0.0_f32, f32::max);
        ensure!(
            peak > 0.05 && peak < 0.3,
            "Decoded audio is silent or corrupted"
        );
        results.push(serde_json::json!({"format":format.extension(),"duration":duration,"backend":backend,"decoded_frames":30,"audio_peak":peak}));
    }
    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"executable":exe,"ffmpeg":ffmpeg,"ffprobe":ffprobe,"checks":results,"status":"passed","hardware_capture_tested":false});
    fs::write(
        output.join("package-check.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
