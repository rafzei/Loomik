use loomik::{
    model::{Bounds, CameraPlacement, Quality},
    recording::{
        encoder::Encoder,
        frame::{CameraCompositor, VideoFrame},
        metrics::Latency,
    },
};
use std::time::Instant;

/// A local throughput measurement, not an assertion about all users' hardware.
#[test]
#[ignore = "run in release mode to measure 1080p60 encode/composition throughput"]
fn realtime_1080p60_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let mut encoder = Encoder::start(
        dir.path().join("session"),
        1920,
        1080,
        60,
        Quality::Balanced,
    )
    .unwrap();
    let mut screen = VideoFrame {
        width: 1920,
        height: 1080,
        bgra: vec![255; 1920 * 1080 * 4],
        captured_at: Instant::now(),
    };
    let camera = VideoFrame {
        width: 640,
        height: 480,
        bgra: vec![160; 640 * 480 * 4],
        captured_at: Instant::now(),
    };
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    let placement = CameraPlacement {
        visible: true,
        ..CameraPlacement::default()
    };
    let mut compositor = CameraCompositor::default();
    let mut writes = Latency::default();
    let started = Instant::now();
    for frame in 0..180 {
        // Changing checkerboard with gradients creates actual encoder work.
        for (i, p) in screen.bgra.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = i % 1920;
            let y = i / 1920;
            p.copy_from_slice(&[
                ((x + frame * 3) % 256) as u8,
                ((y + frame * 2) % 256) as u8,
                (((x / 32 + y / 32 + frame / 8) % 2) * 220) as u8,
                255,
            ]);
        }
        compositor.apply(&mut screen, &camera, placement, bounds);
        let write = Instant::now();
        encoder.write_frame(&screen.bgra).unwrap();
        writes.add(write.elapsed());
    }
    encoder.finish_video().unwrap();
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "{}",
        serde_json::json!({"encoder":encoder.backend, "fallback":encoder.fallback_reason, "frames":180, "seconds":seconds, "throughput_fps":180.0/seconds, "encode_write":writes.report()})
    );
}
