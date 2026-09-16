use loomik::{
    model::{Format, Quality},
    recording::encoder::Encoder,
};
use std::{fs, process::Command};

fn probe(path: &std::path::Path) -> serde_json::Value {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .expect("FFprobe must be installed for media integration tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn all_containers_produce_decodable_h264_and_optional_aac_with_correct_duration() {
    let dir = tempfile::tempdir().unwrap();
    for (index, format) in Format::ALL.into_iter().enumerate() {
        let session = dir.path().join(format!("session{index}"));
        let mut encoder = Encoder::start(session.clone(), 64, 48, 15, Quality::Balanced).unwrap();
        for n in 0..30 {
            let color = if n < 15 {
                [0u8, 0, 240, 255]
            } else {
                [240, 0, 0, 255]
            };
            encoder.write_frame(&color.repeat(64 * 48)).unwrap();
        }
        let audio = if index == 1 {
            let samples: Vec<u8> = (0..96000)
                .flat_map(|i| {
                    ((i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.1).to_le_bytes()
                })
                .collect();
            fs::write(session.join("microphone.f32"), samples).unwrap();
            Some(48000)
        } else {
            None
        };
        let output = dir.path().join(format!("movie.{}", format.extension()));
        encoder.finalize(&output, format, audio).unwrap();
        let metadata = probe(&output);
        let video = &metadata["streams"][0];
        assert_eq!(video["codec_name"], "h264");
        assert_eq!(video["pix_fmt"], "yuv420p");
        assert_eq!(video["width"], 64);
        assert_eq!(video["height"], 48);
        let duration: f64 = metadata["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((duration - 2.0).abs() < 0.08, "{format:?}: {duration}");
        if audio.is_some() {
            assert_eq!(metadata["streams"][1]["codec_name"], "aac");
        }
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args([
                "-map", "0:v", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert_eq!(decoded.stdout.len(), 30 * 64 * 48 * 3);
        let first = &decoded.stdout[..3];
        let last = &decoded.stdout[decoded.stdout.len() - 3..];
        assert!(first[0] > 220 && first[2] < 20);
        assert!(last[2] > 220 && last[0] < 20);
    }
}

#[test]
fn invalid_frames_are_rejected_and_an_existing_recording_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session");
    let mut encoder = Encoder::start(session.clone(), 64, 48, 15, Quality::Compact).unwrap();
    assert!(encoder.write_frame(&[0; 20]).is_err());
    encoder
        .write_frame(&[40, 40, 40, 255].repeat(64 * 48))
        .unwrap();
    let output = dir.path().join("existing.mp4");
    fs::write(&output, b"existing recording").unwrap();
    assert!(encoder.finalize(&output, Format::Mp4, None).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"existing recording");
    assert!(session.join("finished.mp4").exists());
}
