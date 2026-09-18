use loomik::{
    model::{CameraPlacement, Quality, RecordingSource, Settings},
    recording::{
        Command, Event, Recording, RecordingRequest,
        frame::{CameraFrameRenderer, LatestFrame, VideoFrame},
        source::{FrameSource, Input},
    },
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn frame(captured_at: Instant) -> VideoFrame {
    let mut bgra = Vec::with_capacity(96 * 64 * 4);
    for _ in 0..64 {
        bgra.extend([0, 0, 240, 255].repeat(48));
        bgra.extend([240, 0, 0, 255].repeat(48));
    }
    VideoFrame {
        width: 96,
        height: 64,
        bgra,
        captured_at,
    }
}

fn source() -> RecordingSource {
    RecordingSource::Camera {
        width: 96,
        height: 64,
    }
}

fn next(recording: &Recording) -> Event {
    recording
        .events
        .recv_timeout(Duration::from_secs(15))
        .unwrap()
}

struct CameraFixture {
    frames: LatestFrame,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl CameraFixture {
    fn new() -> Self {
        let frames = LatestFrame::default();
        frames.set(frame(Instant::now()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (output, stop) = (frames.clone(), stopped.clone());
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                output.set(frame(Instant::now()));
                thread::sleep(Duration::from_millis(20));
            }
            output.clear();
        });
        Self {
            frames,
            stopped,
            worker: Some(worker),
        }
    }
    fn record(&self, output: &Path) -> Recording {
        Recording::spawn(RecordingRequest {
            settings: Settings {
                output_dir: output.to_path_buf(),
                fps: 30,
                ..Settings::default()
            },
            source: source(),
            microphone: None,
            camera: self.frames.clone(),
            // A visible circle must never be composited onto a camera-only take.
            placement: Arc::new(Mutex::new(CameraPlacement {
                x: 0.0,
                y: 0.0,
                diameter: 64.0,
                mirror: true,
                visible: true,
            })),
        })
    }
}
impl Drop for CameraFixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn camera_input_uses_acquisition_history_and_detects_disconnects() {
    let frames = LatestFrame::default();
    let now = Instant::now();
    let older = now - Duration::from_millis(100);
    frames.set(frame(older));
    frames.set(frame(now));
    assert!(!source().requires_screen_permission());
    assert!(source().is_live());
    assert_eq!(source().dimensions(Quality::Crisp), (96, 64));
    let mut input = Input::open(&source(), 96, 64, 30, true, frames.clone()).unwrap();
    assert!(input.ready());
    input.check().unwrap();
    assert_eq!(
        input
            .frame_at(now - Duration::from_millis(50))
            .unwrap()
            .unwrap()
            .captured_at,
        older
    );
    assert!(
        input
            .frame_at(older - Duration::from_millis(1))
            .unwrap()
            .is_none()
    );
    frames.clear();
    assert!(!input.ready());
    assert!(
        input
            .check()
            .unwrap_err()
            .to_string()
            .contains("Camera stopped")
    );
    frames.set(frame(now - Duration::from_secs(3)));
    assert!(
        input.check().is_err(),
        "A frozen feed must not silently record forever"
    );
}

#[test]
fn full_camera_frame_preserves_edges_and_mirrors_at_export_size() {
    let camera = frame(Instant::now());
    let mut renderer = CameraFrameRenderer::default();
    for mirror in [false, true, false] {
        let output = renderer.render(&camera, 48, 32, mirror);
        assert_eq!((output.width, output.height), (48, 32));
        assert_eq!(output.captured_at, camera.captured_at);
        for y in [0, 31] {
            let left = &output.bgra[y * 48 * 4..][..4];
            let right = &output.bgra[(y * 48 + 47) * 4..][..4];
            assert_eq!(
                left,
                if mirror {
                    &[240, 0, 0, 255]
                } else {
                    &[0, 0, 240, 255]
                }
            );
            assert_eq!(
                right,
                if mirror {
                    &[0, 0, 240, 255]
                } else {
                    &[240, 0, 0, 255]
                }
            );
        }
    }
    assert_eq!(renderer.render(&camera, 96, 64, false).bgra, camera.bgra);
}

#[test]
fn camera_only_movie_keeps_the_full_frame_and_excludes_pauses() {
    let dir = tempfile::tempdir().unwrap();
    let camera = CameraFixture::new();
    let recording = camera.record(dir.path());
    assert!(matches!(next(&recording), Event::Ready));
    assert!(recording.elapsed().is_zero());
    recording.command(Command::Begin);
    assert!(matches!(next(&recording), Event::Started));
    thread::sleep(Duration::from_millis(350));
    recording.command(Command::Pause);
    assert!(matches!(next(&recording), Event::Paused));
    let paused = recording.elapsed();
    thread::sleep(Duration::from_millis(250));
    assert_eq!(recording.elapsed(), paused);
    recording.command(Command::Resume);
    assert!(matches!(next(&recording), Event::Resumed));
    thread::sleep(Duration::from_millis(350));
    recording.command(Command::Stop);
    let duration = recording.elapsed();
    assert!(matches!(next(&recording), Event::Finalizing));
    let Event::Saved(movie) = next(&recording) else {
        panic!("Camera movie was not saved")
    };
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&movie)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let count = decoded.stdout.len() / (96 * 64 * 3);
    assert!((count as f64 / 30.0 - duration.as_secs_f64()).abs() < 0.05);
    for image in decoded.stdout.as_chunks::<{ 96 * 64 * 3 }>().0 {
        for y in [0, 32, 63] {
            for x in [0, 20, 75, 95] {
                let pixel = &image[(y * 96 + x) * 3..][..3];
                if x < 48 {
                    assert!(pixel[2] > 220 && pixel[0] < 20, "{x},{y}: {pixel:?}");
                } else {
                    assert!(pixel[0] > 220 && pixel[2] < 20, "{x},{y}: {pixel:?}");
                }
            }
        }
    }
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(movie.with_extension("performance.json")).unwrap())
            .unwrap();
    assert_eq!(report["source_kind"], "camera");
    assert_eq!(report["width"], 96);
    assert_eq!(report["height"], 64);
}

#[test]
fn camera_countdown_cancels_and_disconnect_retains_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let camera = CameraFixture::new();
    let recording = camera.record(dir.path());
    assert!(matches!(next(&recording), Event::Ready));
    recording.command(Command::Stop);
    assert!(matches!(next(&recording), Event::Discarded));
    let recording = camera.record(&dir.path().join("disconnect"));
    assert!(matches!(next(&recording), Event::Ready));
    recording.command(Command::Begin);
    assert!(matches!(next(&recording), Event::Started));
    thread::sleep(Duration::from_millis(350));
    drop(camera);
    let Event::Failed(error) = next(&recording) else {
        panic!("Camera disconnect was not reported")
    };
    assert!(error.contains("Camera stopped"), "{error}");
    let session = std::fs::read_dir(dir.path().join("disconnect"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(session.join("session.json").exists());
    assert!(session.join("video.mp4").exists());
}
