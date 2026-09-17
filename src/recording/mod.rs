pub mod audio;
pub mod encoder;
pub mod frame;
pub mod metrics;
pub mod source;
use source::FrameSource;

use crate::{
    capture,
    model::{CameraPlacement, Phase, RecordingClock, RecordingSource, Settings},
    recording::{
        encoder::Encoder,
        frame::{CameraCompositor, LatestFrame},
    },
};
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum Command {
    Begin,
    Pause,
    Resume,
    Stop,
    Discard,
}
#[derive(Debug)]
pub enum Event {
    Ready,
    Started,
    Paused,
    Resumed,
    Finalizing,
    Saved(PathBuf),
    Discarded,
    Failed(String),
}

pub struct Recording {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
    pub clock: Arc<Mutex<RecordingClock>>,
    pub background: LatestFrame,
}

pub struct RecordingRequest {
    pub settings: Settings,
    pub source: RecordingSource,
    pub microphone: Option<String>,
    pub camera: LatestFrame,
    pub placement: Arc<Mutex<CameraPlacement>>,
}

impl Recording {
    pub fn spawn(request: RecordingRequest) -> Self {
        let (commands, rx) = bounded(8);
        let (tx, events) = bounded(16);
        let clock = Arc::new(Mutex::new(RecordingClock::default()));
        let worker_clock = clock.clone();
        let background = LatestFrame::default();
        let worker_background = background.clone();
        thread::spawn(move || {
            let result = run(request, rx, &tx, worker_clock, worker_background);
            if let Err(error) = result {
                let _ = tx.send(Event::Failed(format!("{error:#}")));
            }
        });
        Self {
            commands,
            events,
            clock,
            background,
        }
    }
    pub fn command(&self, command: Command) {
        // Timestamp controls on the UI thread: an encoder pipe stall must not
        // move the pause/stop boundary in either the audio or video timeline.
        let mut clock = self.clock.lock().unwrap();
        match command {
            Command::Begin => clock.begin(Instant::now()),
            Command::Pause | Command::Stop | Command::Discard => clock.pause(Instant::now()),
            Command::Resume => clock.resume(Instant::now()),
        }
        let _ = self.commands.try_send(command);
    }
    pub fn elapsed(&self) -> Duration {
        self.clock.lock().unwrap().elapsed(Instant::now())
    }
}

fn run(
    request: RecordingRequest,
    commands: Receiver<Command>,
    events: &Sender<Event>,
    clock: Arc<Mutex<RecordingClock>>,
    background: LatestFrame,
) -> Result<()> {
    let settings = &request.settings;
    std::fs::create_dir_all(&settings.output_dir)
        .context("Cannot write to the selected output folder")?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stem = format!(
        "Loomik {}-{}",
        chrono::Local::now().format("%Y-%m-%d %H.%M.%S"),
        &id[..6]
    );
    let output = settings
        .output_dir
        .join(format!("{stem}.{}", settings.format.extension()));
    let session = settings.output_dir.join(format!(".loomik-{id}"));
    let mut session_dir = std::fs::DirBuilder::new();
    session_dir.recursive(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        // Recovery contains raw microphone audio and source paths. Restrict
        // access at creation, even when the output directory is shared.
        session_dir.mode(0o700);
    }
    session_dir.create(&session)?;
    std::fs::write(
        session.join("session.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"output":output,"fps":settings.fps,"format":settings.format.extension(),"recovery":"video.mp4 is a fragmented MP4 and can be remuxed with ffmpeg -i video.mp4 -c copy recovered.mp4"}),
        )?,
    )?;
    if let RecordingSource::Media(source) = &request.source {
        std::fs::write(
            session.join("background.json"),
            serde_json::to_vec_pretty(source)?,
        )?;
    }
    let result = record(
        &request,
        commands,
        events,
        clock.clone(),
        &session,
        &output,
        background,
    );
    clock.lock().unwrap().pause(Instant::now());
    if result.is_ok() {
        // A cleanup failure must not turn an already-saved movie into a failed
        // recording. Retain the recoverable session if the filesystem is busy.
        if let Err(error) = std::fs::remove_dir_all(&session) {
            eprintln!(
                "Recording finished; could not clean {}: {error}",
                session.display()
            );
        }
    }
    result.with_context(|| format!("Session files are in {}", session.display()))
}

fn record(
    request: &RecordingRequest,
    commands: Receiver<Command>,
    events: &Sender<Event>,
    clock: Arc<Mutex<RecordingClock>>,
    session: &std::path::Path,
    output: &std::path::Path,
    background: LatestFrame,
) -> Result<()> {
    let source = &request.source;
    // Linux studio consumes the newest native frame directly. Its live preview
    // must not wait for the recorder's timestamp assembly allowance.
    let screen = if cfg!(target_os = "linux") && source.requires_screen_permission() {
        background.clone()
    } else {
        LatestFrame::default()
    };
    let settings = &request.settings;
    let (width, height) = source.dimensions(settings.quality);
    let mut encoder = Encoder::start(
        session.to_path_buf(),
        width,
        height,
        settings.fps,
        settings.quality,
    )?;
    let mut capture = source::Input::open(
        source,
        width,
        height,
        settings.fps,
        settings.show_cursor,
        screen.clone(),
    )?;
    let mut audio = request
        .microphone
        .as_deref()
        .map(|id| capture::Microphone::start(id, session, clock.clone()))
        .transpose()?;
    if let Some(audio) = &audio {
        let metadata_path = session.join("session.json");
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&metadata_path)?)?;
        metadata["microphone_sample_rate"] = audio.sample_rate().into();
        metadata["microphone_channels"] = 1.into();
        metadata["microphone_sample_format"] = "f32le".into();
        std::fs::write(metadata_path, serde_json::to_vec_pretty(&metadata)?)?;
    }
    let wait_started = Instant::now();
    while !capture.ready() {
        if matches!(
            commands.try_recv(),
            Ok(Command::Stop | Command::Discard)
                | Err(crossbeam_channel::TryRecvError::Disconnected)
        ) {
            let _ = events.send(Event::Discarded);
            return Ok(());
        }
        capture.check()?;
        anyhow::ensure!(
            wait_started.elapsed() < Duration::from_secs(15),
            "No screen frames arrived. Check Screen Recording permission and that the source is still available."
        );
        thread::sleep(Duration::from_millis(20));
    }
    events.send(Event::Ready)?;
    // Native streams are warm, but the media timeline is empty until the UI
    // finishes 3–2–1–0. Audio clips pre-roll against that same empty timeline.
    if !wait_for_begin(&commands, || {
        capture.check()?;
        if let Some(audio) = &mut audio {
            audio.check()?;
        }
        Ok(())
    })? {
        events.send(Event::Discarded)?;
        return Ok(());
    }
    events.send(Event::Started)?;
    let mut phase = Phase::Recording;
    let mut discard = false;
    // Output-only delivery allowance: camera preview always consumes latest.
    // Acquisition history is bounded separately; no unbounded latency queue.
    let delivery_budget = Duration::from_millis(100);
    let mut compositor = CameraCompositor::default();
    let mut scratch = None;
    let mut last_screen = screen.get();
    let mut last_camera = None;
    let mut encode_latency = metrics::Latency::default();
    let mut composition_latency = metrics::Latency::default();
    let mut scheduling_latency = metrics::Latency::default();
    let mut camera_age = metrics::Latency::default();
    let mut screen_age = metrics::Latency::default();
    let mut history_misses = 0u64;
    let mut missing_camera = 0u64;
    let mut repeated_camera = 0u64;
    let mut previous_camera_time = None;
    let mut source_read_latency = metrics::Latency::default();
    loop {
        for command in commands.try_iter() {
            match command {
                Command::Pause if phase == Phase::Recording => {
                    phase = Phase::Paused;
                    events.send(Event::Paused)?;
                }
                Command::Resume if phase == Phase::Paused => {
                    phase = Phase::Recording;
                    events.send(Event::Resumed)?;
                }
                Command::Stop => {
                    phase = Phase::Stopping;
                }
                Command::Discard => {
                    phase = Phase::Stopping;
                    discard = true;
                }
                _ => {}
            }
        }
        capture.check()?;
        if let Some(audio) = &mut audio {
            audio.check()?;
        }
        if discard {
            break;
        }
        let now = Instant::now();
        let timeline = clock.lock().unwrap().clone();
        let elapsed = timeline.elapsed(now);
        let due_frames = (elapsed.as_secs_f64() * settings.fps as f64).ceil() as u64;
        if encoder.frame_count >= due_frames.max(1) {
            if phase == Phase::Stopping {
                break;
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let position = Duration::from_secs_f64(encoder.frame_count as f64 / settings.fps as f64);
        let Some(target) = timeline.instant_at(position) else {
            if phase == Phase::Stopping {
                break;
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        };
        // Also flush pre-pause/stop slots only after their delivery deadline so
        // late callbacks are handled identically at every boundary.
        if now < target + delivery_budget {
            thread::sleep((target + delivery_budget - now).min(Duration::from_millis(2)));
            continue;
        }
        let lag = elapsed.saturating_sub(position);
        anyhow::ensure!(
            lag < Duration::from_secs(2),
            "Video encoding cannot keep up. Recording stopped to preserve synchronization. Choose 30 fps or a lower quality; completed fragments remain in the session folder."
        );
        scheduling_latency.add(lag);
        let read_started = Instant::now();
        let selected = capture.frame_at(target)?;
        source_read_latency.add(read_started.elapsed());
        if let Some(selected) = selected {
            if !source.requires_screen_permission() {
                background.set_shared(selected.clone());
            }
            last_screen = Some(selected);
        } else {
            history_misses += 1;
        }
        let Some(screen_frame) = last_screen
            .as_ref()
            .filter(|f| !source.requires_screen_permission() || f.captured_at <= target)
        else {
            continue;
        };
        // If an encoder stall exhausts history, hold the prior image at its
        // original time. Never pull a current/future image into an older slot.
        if source.requires_screen_permission() {
            screen_age.add(target.saturating_duration_since(screen_frame.captured_at));
        }
        if let Some(selected) = request.camera.at_or_before(target) {
            last_camera = Some(selected);
        }
        let camera = last_camera.as_ref().filter(|c| {
            target.saturating_duration_since(c.captured_at) <= Duration::from_millis(250)
        });
        let placement = *request.placement.lock().unwrap();
        let compose_started = Instant::now();
        let bytes = if let Some(camera) = camera.filter(|_| placement.visible) {
            camera_age.add(target.saturating_duration_since(camera.captured_at));
            if previous_camera_time == Some(camera.captured_at) {
                repeated_camera += 1;
            }
            previous_camera_time = Some(camera.captured_at);
            let frame = scratch.get_or_insert_with(|| (**screen_frame).clone());
            frame.bgra.clone_from(&screen_frame.bgra);
            frame.width = screen_frame.width;
            frame.height = screen_frame.height;
            compositor.apply(frame, camera, placement, capture.bounds());
            &frame.bgra
        } else {
            if placement.visible {
                missing_camera += 1;
            }
            &screen_frame.bgra
        };
        composition_latency.add(compose_started.elapsed());
        let encode_started = Instant::now();
        encoder.write_frame(bytes)?;
        encode_latency.add(encode_started.elapsed());
    }

    drop(capture);
    let (audio_rate, audio_report) = if let Some(mut audio) = audio {
        let report = audio.stop()?;
        (Some(audio.sample_rate()), Some(report))
    } else {
        (None, None)
    };
    let performance = serde_json::json!({
        "source_kind":match source {RecordingSource::Desktop(s)=>format!("{:?}",s.kind).to_lowercase(),RecordingSource::Media(s)=>format!("{:?}",s.info.kind).to_lowercase()},
        "source_frame_read":source_read_latency.report(),
        "encoder": encoder.backend, "hardware_fallback_reason": encoder.fallback_reason,
        "width": width, "height": height, "fps": settings.fps, "frames": encoder.frame_count,
        "output_delivery_budget_ms": delivery_budget.as_millis(),
        "encode_write": encode_latency.report(), "composition": composition_latency.report(),
        "output_scheduling_lag": scheduling_latency.report(),
        "screen_capture_delivery": screen.delivery_report(), "camera_capture_delivery": request.camera.delivery_report(),
        "screen_frame_age": screen_age.report(), "camera_frame_age": camera_age.report(),
        "screen_history_misses": history_misses, "missing_camera_frames": missing_camera, "repeated_camera_frames": repeated_camera,
        "audio": audio_report,
        "note": "Timestamp/processing metrics, not a physical sensor-to-display or lip-sync measurement. Static screens and lower-fps cameras repeat frames normally."
    });
    std::fs::write(
        session.join("performance.json"),
        serde_json::to_vec_pretty(&performance)?,
    )?;
    if discard {
        drop(encoder);
        events.send(Event::Discarded)?;
    } else {
        events.send(Event::Finalizing)?;
        let media = match source {
            RecordingSource::Media(s) => Some(s),
            _ => None,
        };
        encoder.finalize_with_media(output, settings.format, audio_rate, media)?;
        // Small, bounded diagnostic sidecar. No per-frame trace or captured data.
        if let Err(error) = std::fs::copy(
            session.join("performance.json"),
            output.with_extension("performance.json"),
        ) {
            eprintln!("Could not save recording performance report: {error}");
        }
        events.send(Event::Saved(output.to_path_buf()))?;
    }
    Ok(())
}

fn wait_for_begin(
    commands: &Receiver<Command>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    loop {
        check()?;
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(Command::Begin) => return Ok(true),
            Ok(Command::Stop | Command::Discard)
            | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Ok(false),
            _ => {}
        }
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;

    #[test]
    fn prepared_streams_wait_for_begin_and_can_be_cancelled() {
        for command in [Command::Begin, Command::Stop, Command::Discard] {
            let expected = matches!(command, Command::Begin);
            let (tx, rx) = bounded(8);
            let mut checks = 0;
            assert_eq!(
                wait_for_begin(&rx, || {
                    checks += 1;
                    if checks == 2 {
                        tx.send(if expected {
                            Command::Begin
                        } else {
                            Command::Discard
                        })?;
                    }
                    Ok(())
                })
                .unwrap(),
                expected
            );
            assert_eq!(checks, 2);
            tx.send(command).unwrap();
            assert_eq!(wait_for_begin(&rx, || Ok(())).unwrap(), expected);
        }
        let (tx, rx) = bounded(1);
        drop(tx);
        assert!(!wait_for_begin(&rx, || Ok(())).unwrap());
    }

    #[test]
    fn preparation_failure_does_not_begin_recording() {
        let (_tx, rx) = bounded(1);
        assert!(wait_for_begin(&rx, || anyhow::bail!("device disconnected")).is_err());
    }
}
