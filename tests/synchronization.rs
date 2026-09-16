use loomik::{
    model::{Format, Quality, RecordingClock},
    recording::{
        audio::{AudioChunk, AudioWriter},
        encoder::Encoder,
    },
};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn exported_flash_and_tone_remain_aligned_after_pause_and_delayed_audio_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session");
    let mut encoder = Encoder::start(session.clone(), 64, 48, 60, Quality::Balanced).unwrap();
    let base = Instant::now() - Duration::from_secs(10);
    let mut clock = RecordingClock::default();
    clock.start(base);
    clock.pause(base + Duration::from_millis(800));
    clock.resume(base + Duration::from_millis(2800));
    clock.pause(base + Duration::from_secs(4));
    let event = base + Duration::from_secs(3);
    // Encode after stop, using original acquisition time for each output slot.
    for i in 0..120 {
        let capture_time = clock
            .instant_at(Duration::from_secs_f64(i as f64 / 60.0))
            .unwrap();
        let luma = if capture_time >= event { 240 } else { 10 };
        encoder
            .write_frame(&[luma, luma, luma, 255].repeat(64 * 48))
            .unwrap();
    }
    let samples = (0..240_000)
        .map(|i| {
            if (144_000..148_800).contains(&i) {
                0.8 * (i as f32 * std::f32::consts::TAU * 1000.0 / 48_000.0).sin()
            } else {
                0.0
            }
        })
        .collect();
    // The audio callback is intentionally delivered after all video, including
    // samples from the paused interval. Delivery order must not change timing.
    let mut audio = AudioWriter::new(
        fs::File::create(session.join("microphone.f32")).unwrap(),
        48_000,
    );
    audio
        .write(
            AudioChunk {
                samples,
                start: base,
                end: base + Duration::from_secs(5),
            },
            &clock,
        )
        .unwrap();
    audio.finish().unwrap();
    let output = dir.path().join("sync.mp4");
    encoder
        .finalize(&output, Format::Mp4, Some(48_000))
        .unwrap();
    let video = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-an", "-pix_fmt", "gray", "-f", "rawvideo", "pipe:1"])
        .output()
        .unwrap();
    assert!(video.status.success());
    let first_flash = video
        .stdout
        .as_chunks::<{ 64 * 48 }>()
        .0
        .iter()
        .position(|p| p[0] > 200)
        .unwrap();
    assert_eq!(first_flash, 60);
    let audio = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-vn", "-f", "f32le", "pipe:1"])
        .output()
        .unwrap();
    assert!(audio.status.success());
    let first_tone = audio
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .position(|p| f32::from_le_bytes(*p).abs() > 0.2)
        .unwrap();
    let offset = first_tone as f64 / 48_000.0 - first_flash as f64 / 60.0;
    assert!(
        offset.abs() < 0.002,
        "Encoded synthetic A/V marker skew: {} ms",
        offset * 1000.0
    );
}
