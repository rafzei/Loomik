use loomik::{
    media::{self, Aspect, Fit, MediaKind, MediaSource, Options, Reader},
    model::{CameraPlacement, Format, Quality, RecordingSource, Settings},
    recording::{self, Recording, RecordingRequest, encoder::Encoder, frame::LatestFrame},
};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn ffmpeg(args: &[&str], output: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success());
}
fn source(path: &Path, kind: MediaKind) -> MediaSource {
    MediaSource {
        info: media::probe(path, kind).unwrap(),
        options: Options::default(),
    }
}
fn rgb(frame: &loomik::recording::frame::VideoFrame, x: u32, y: u32) -> [u8; 3] {
    let p = &frame.bgra[((y * frame.width + x) * 4) as usize..];
    [p[2], p[1], p[0]]
}

#[test]
fn image_canvas_preserves_aspect_transparency_and_jpeg_orientation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("portrait.png");
    let mut image = image::RgbaImage::from_pixel(40, 80, image::Rgba([240, 30, 20, 255]));
    image.put_pixel(20, 40, image::Rgba([255, 0, 0, 0]));
    image.save(&path).unwrap();
    let mut s = source(&path, MediaKind::Image);
    s.options.aspect = Aspect::Square;
    let mut reader = Reader::open(&s, 40, 40, 30).unwrap();
    let frame = reader.next_frame().unwrap();
    assert_eq!(rgb(&frame, 0, 20), [24, 25, 28]);
    assert_eq!(rgb(&frame, 20, 10), [240, 30, 20]);
    s.options.fit = Fit::Fill;
    let frame = Reader::open(&s, 40, 40, 30).unwrap().next_frame().unwrap();
    assert_eq!(rgb(&frame, 0, 20), [240, 30, 20]);
    assert_eq!(rgb(&frame, 20, 20), [24, 25, 28]);
    let jpeg = dir.path().join("oriented.jpg");
    image::DynamicImage::ImageRgba8(image)
        .to_rgb8()
        .save(&jpeg)
        .unwrap();
    let data = fs::read(&jpeg).unwrap();
    // EXIF Orientation=6 (90 degrees clockwise), one IFD0 short entry.
    let exif = [
        b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0,
        0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut rotated = vec![0xff, 0xd8, 0xff, 0xe1];
    rotated.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    rotated.extend_from_slice(&exif);
    rotated.extend_from_slice(&data[2..]);
    fs::write(&jpeg, rotated).unwrap();
    let s = source(&jpeg, MediaKind::Image);
    assert_eq!((s.info.width, s.info.height), (80, 40));
    let frame = Reader::open(&s, 80, 40, 30).unwrap().next_frame().unwrap();
    assert_eq!((frame.width, frame.height), (80, 40));
    assert!(media::probe(&dir.path().join("missing.png"), MediaKind::Image).is_err());
    fs::write(dir.path().join("broken.mp4"), "not a movie").unwrap();
    assert!(media::probe(&dir.path().join("broken.mp4"), MediaKind::Video).is_err());
}

#[test]
fn video_seeking_hold_loop_vfr_and_rotation_decode_incrementally() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("source.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=red:s=96x64:r=30:d=1",
            "-vf",
            "drawbox=x=0:y=0:w=iw:h=ih:c=blue:t=fill:enable='gte(t,0.5)'",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
        &clip,
    );
    let mut s = source(&clip, MediaKind::Video);
    s.options.start = 0.6;
    let mut reader = Reader::open(&s, 96, 64, 30).unwrap();
    for _ in 0..65 {
        assert!(rgb(&reader.next_frame().unwrap(), 40, 30)[2] > 220);
    }
    drop(reader);
    s.options.start = 0.0;
    s.options.loop_video = true;
    let mut reader = Reader::open(&s, 96, 64, 30).unwrap();
    for n in 0..70 {
        let color = rgb(&reader.next_frame().unwrap(), 40, 30);
        if n % 30 < 14 {
            assert!(color[0] > 220, "frame {n}: {color:?}");
        }
        if n % 30 > 16 {
            assert!(color[2] > 220, "frame {n}: {color:?}");
        }
    }
    drop(reader);
    let rotated = dir.path().join("rotated.mov");
    ffmpeg(
        &[
            "-display_rotation:v:0",
            "90",
            "-i",
            clip.to_str().unwrap(),
            "-c",
            "copy",
        ],
        &rotated,
    );
    let s = source(&rotated, MediaKind::Video);
    assert_eq!((s.info.width, s.info.height), (64, 96));
    let frame = Reader::open(&s, 64, 96, 30).unwrap().next_frame().unwrap();
    assert!(rgb(&frame, 30, 40)[0] > 220);
    let vfr = dir.path().join("vfr.mkv");
    ffmpeg(
        &[
            "-i",
            clip.to_str().unwrap(),
            "-vf",
            "select='eq(n,0)+eq(n,3)+eq(n,15)+eq(n,27)'",
            "-fps_mode",
            "vfr",
            "-c:v",
            "ffv1",
        ],
        &vfr,
    );
    let mut reader = Reader::open(&source(&vfr, MediaKind::Video), 96, 64, 30).unwrap();
    for n in 0..30 {
        let color = rgb(&reader.next_frame().unwrap(), 40, 30);
        if n < 14 {
            assert!(color[0] > 220);
        }
        if n > 16 {
            assert!(color[2] > 220);
        }
    }
}

#[test]
fn media_worker_records_without_desktop_capture_and_preserves_pause() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("background.png");
    image::RgbImage::from_pixel(96, 64, image::Rgb([30, 150, 70]))
        .save(&file)
        .unwrap();
    let input = RecordingSource::Media(source(&file, MediaKind::Image));
    assert!(!input.requires_screen_permission());
    let recording = Recording::spawn(RecordingRequest {
        settings: Settings {
            output_dir: dir.path().join("movies"),
            quality: Quality::Compact,
            fps: 30,
            ..Settings::default()
        },
        source: input,
        microphone: None,
        camera: LatestFrame::default(),
        placement: Arc::new(Mutex::new(CameraPlacement::default())),
    });
    assert!(matches!(
        recording
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap(),
        recording::Event::Ready
    ));
    assert_eq!(recording.elapsed(), Duration::ZERO);
    recording.command(recording::Command::Begin);
    assert!(matches!(
        recording
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        recording::Event::Started
    ));
    std::thread::sleep(Duration::from_millis(350));
    recording.command(recording::Command::Pause);
    assert!(matches!(
        recording
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        recording::Event::Paused
    ));
    let paused = recording.elapsed();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(recording.elapsed(), paused);
    recording.command(recording::Command::Resume);
    std::thread::sleep(Duration::from_millis(350));
    recording.command(recording::Command::Stop);
    // sleep() is only a minimum delay: busy CI runners can oversleep. Compare
    // decoded frames with the actual active timeline, which excludes the pause.
    let active_duration = recording.elapsed();
    let deadline = Instant::now() + Duration::from_secs(15);
    let movie = loop {
        assert!(Instant::now() < deadline);
        match recording
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            recording::Event::Saved(path) => break path,
            recording::Event::Failed(e) => panic!("{e}"),
            _ => {}
        }
    };
    let data = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&movie)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(data.status.success());
    let count = data.stdout.len() / (96 * 64 * 3);
    let decoded_duration = count as f64 / 30.0;
    assert!(
        (decoded_duration - active_duration.as_secs_f64()).abs() <= 1.0 / 30.0,
        "{count} frames for {active_duration:?} of active recording"
    );
    for p in data.stdout.as_chunks::<3>().0 {
        assert!(p[1] > 130 && p[0] < 50 && p[2] < 90);
    }
}

#[test]
fn background_audio_and_microphone_mix_match_exported_video() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("flash-tone.mkv");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=black:s=96x64:r=30:d=1,drawbox=x=0:y=0:w=iw:h=ih:c=white:t=fill:enable='gte(t,0.5)'",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=if(gte(t\\,0.5)\\,0.2*sin(2*PI*440*t)\\,0):s=48000:d=1",
            "-c:v",
            "ffv1",
            "-c:a",
            "pcm_f32le",
        ],
        &clip,
    );
    let mut s = source(&clip, MediaKind::Video);
    s.options.source_audio = true;
    s.options.source_volume = 0.5;
    s.options.microphone_volume = 0.0;
    s.options.loop_video = true;
    let mut reader = Reader::open(&s, 96, 64, 30).unwrap();
    let session = dir.path().join("session");
    let mut encoder = Encoder::start(session.clone(), 96, 64, 30, Quality::Balanced).unwrap();
    for _ in 0..75 {
        encoder
            .write_frame(&reader.next_frame().unwrap().bgra)
            .unwrap();
    }
    fs::write(
        session.join("microphone.f32"),
        0.5f32.to_le_bytes().repeat(48_000 * 3),
    )
    .unwrap();
    let output = dir.path().join("mixed.mp4");
    encoder
        .finalize_with_media(&output, Format::Mp4, Some(48_000), Some(&s))
        .unwrap();
    let raw = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"])
        .output()
        .unwrap();
    assert!(raw.status.success());
    let samples: Vec<f32> = raw
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| f32::from_le_bytes(*p))
        .collect();
    let peak = |a: usize, b: usize| samples[a..b].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak(1000, 20_000) < 0.001);
    assert!(peak(30_000, 40_000) > 0.08 && peak(30_000, 40_000) < 0.13);
    assert!(peak(50_000, 68_000) < 0.002);
    assert!(peak(78_000, 88_000) > 0.08);
    let onset = samples[20_000..30_000]
        .iter()
        .position(|s| s.abs() > 0.02)
        .unwrap()
        + 20_000;
    assert!(
        (onset as f64 / 48_000.0 - 0.5).abs() < 0.01,
        "audio onset at {onset}"
    );
}

#[test]
fn paused_video_and_source_audio_resume_at_the_same_active_position() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("marker.mkv");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=black:s=96x64:r=30:d=2,drawbox=x=0:y=0:w=iw:h=ih:c=white:t=fill:enable='gte(t,0.5)'",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=if(gte(t\\,0.5)\\,0.2*sin(2*PI*440*t)\\,0):s=48000:d=2",
            "-c:v",
            "ffv1",
            "-c:a",
            "pcm_f32le",
        ],
        &clip,
    );
    let mut source = source(&clip, MediaKind::Video);
    source.options.source_audio = true;
    let recording = Recording::spawn(RecordingRequest {
        settings: Settings {
            output_dir: dir.path().join("output"),
            fps: 30,
            ..Settings::default()
        },
        source: RecordingSource::Media(source),
        microphone: None,
        camera: LatestFrame::default(),
        placement: Arc::new(Mutex::new(CameraPlacement::default())),
    });
    assert!(matches!(
        recording
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap(),
        recording::Event::Ready
    ));
    recording.command(recording::Command::Begin);
    assert!(matches!(
        recording
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        recording::Event::Started
    ));
    std::thread::sleep(Duration::from_millis(250));
    recording.command(recording::Command::Pause);
    assert!(matches!(
        recording
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        recording::Event::Paused
    ));
    std::thread::sleep(Duration::from_millis(400));
    recording.command(recording::Command::Resume);
    std::thread::sleep(Duration::from_millis(500));
    recording.command(recording::Command::Stop);
    let movie = loop {
        match recording
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
        {
            recording::Event::Saved(p) => break p,
            recording::Event::Failed(e) => panic!("{e}"),
            _ => {}
        }
    };
    let video = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&movie)
        .args([
            "-vf",
            "scale=1:1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    assert!(video.status.success());
    let frame = video
        .stdout
        .as_chunks::<3>()
        .0
        .iter()
        .position(|p| p[0] > 180)
        .unwrap();
    assert_eq!(frame, 15);
    let audio = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&movie)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"])
        .output()
        .unwrap();
    assert!(audio.status.success());
    let onset = audio
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .position(|p| f32::from_le_bytes(*p).abs() > 0.02)
        .unwrap();
    assert!((onset as f64 / 48_000.0 - frame as f64 / 30.0).abs() < 0.01);
}
