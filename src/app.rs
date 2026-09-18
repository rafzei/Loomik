use crate::{
    capture, media,
    model::*,
    recording::{
        self, Recording, RecordingRequest,
        frame::{CameraFrameRenderer, LatestFrame, VideoFrame, composite_camera},
    },
    ui,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, *};
use egui_phosphor::regular as icons;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

enum AppEvent {
    Sources(Result<Vec<Source>, String>),
    Devices(Result<(Vec<Device>, Vec<Device>), String>),
    Encoder(Result<PathBuf, String>),
    Snapshot(Result<PathBuf, String>),
    Media(Result<media::MediaInfo, String>),
}
#[derive(Clone, Copy, PartialEq)]
enum Confirmation {
    Restart,
    Discard,
    Quit,
}

struct NativeVerification {
    dir: PathBuf,
    stage: u8,
    entered: Instant,
    paused_time: Duration,
    requested_permission: bool,
    pause_verified: bool,
    video_path: Option<PathBuf>,
    countdown_seen: Vec<u8>,
    countdown_timer_zero: bool,
    zero_shown_at: Option<Instant>,
    began_after_zero: bool,
    cancelled_countdown: bool,
}

pub struct LoomikApp {
    repaint: Context,
    settings: Settings,
    sources: Vec<Source>,
    source: usize,
    media: Option<ui::media_canvas::MediaEditor>,
    camera_only: bool,
    #[cfg(target_os = "linux")]
    capture_canvas: ui::capture_canvas::CaptureCanvas,
    #[cfg(target_os = "linux")]
    portal_picker: bool,
    media_loading: bool,
    cameras: Vec<Device>,
    microphones: Vec<Device>,
    camera_id: Option<String>,
    microphone_id: Option<String>,
    camera: Option<capture::Camera>,
    camera_texture: Option<TextureHandle>,
    camera_frame_time: Option<Instant>,
    placement: Arc<Mutex<CameraPlacement>>,
    recording: Option<Recording>,
    phase: Phase,
    countdown: Option<Countdown>,
    countdown_position: Pos2,
    show_settings: bool,
    more_settings: bool,
    collapsed: bool,
    snapshot_mode: bool,
    snapshot_pending: bool,
    refresh_pending: bool,
    ffmpeg_ready: bool,
    error: Option<String>,
    last_saved: Option<PathBuf>,
    confirmation: Option<Confirmation>,
    restart_after_stop: bool,
    quit_after_stop: bool,
    tx: Sender<AppEvent>,
    events: Receiver<AppEvent>,
    launched: Instant,
    smoke_dir: Option<PathBuf>,
    smoke_shot: bool,
    smoke_camera: LatestFrame,
    native_verification: Option<NativeVerification>,
    testing: bool,
}

impl LoomikApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        smoke_dir: Option<PathBuf>,
        verify_dir: Option<PathBuf>,
        reset_settings: bool,
    ) -> Self {
        ui::configure(&cc.egui_ctx);
        if let Some(dir) = &smoke_dir
            && std::env::args().any(|arg| arg == "--hover-smoke")
        {
            cc.egui_ctx
                .add_plugin(ui::hover_check::HoverCheck::new(dir.clone()));
        }
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "settings"))
            .unwrap_or_default();
        if reset_settings {
            settings = Settings::default();
        }
        let testing = smoke_dir.is_some() || verify_dir.is_some();
        if ![15, 30, 60].contains(&settings.fps) {
            settings.fps = 30;
        }
        settings.camera_size = settings.camera_size.clamp(100.0, 420.0);
        let (tx, events) = unbounded();
        let mut app = Self {
            settings,
            sources: vec![],
            source: 0,
            media: None,
            camera_only: false,
            #[cfg(target_os = "linux")]
            capture_canvas: ui::capture_canvas::CaptureCanvas::default(),
            #[cfg(target_os = "linux")]
            portal_picker: false,
            media_loading: false,
            cameras: vec![],
            microphones: vec![],
            camera_id: None,
            microphone_id: None,
            camera: None,
            repaint: cc.egui_ctx.clone(),
            camera_texture: None,
            camera_frame_time: None,
            placement: Arc::new(Mutex::new(CameraPlacement::default())),
            recording: None,
            phase: Phase::Idle,
            countdown: None,
            countdown_position: Pos2::ZERO,
            show_settings: true,
            more_settings: false,
            collapsed: false,
            snapshot_mode: false,
            snapshot_pending: false,
            refresh_pending: false,
            ffmpeg_ready: false,
            error: None,
            last_saved: None,
            confirmation: None,
            restart_after_stop: false,
            quit_after_stop: false,
            tx,
            events,
            launched: Instant::now(),
            smoke_dir,
            smoke_shot: false,
            smoke_camera: LatestFrame::default(),
            native_verification: verify_dir.map(|dir| NativeVerification {
                dir,
                stage: 0,
                entered: Instant::now(),
                paused_time: Duration::ZERO,
                requested_permission: false,
                pause_verified: false,
                video_path: None,
                countdown_seen: Vec::new(),
                countdown_timer_zero: true,
                zero_shown_at: None,
                began_after_zero: false,
                cancelled_countdown: false,
            }),
            testing,
        };
        app.refresh(false);
        let args: Vec<_> = std::env::args().collect();
        app.camera_only = args.iter().any(|arg| arg == "--camera-only");
        if let Some(path) = args
            .iter()
            .position(|a| a == "--media")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
        {
            let kind = if path.extension().is_some_and(|s| {
                ["png", "jpg", "jpeg"].contains(&s.to_string_lossy().to_lowercase().as_str())
            }) {
                media::MediaKind::Image
            } else {
                media::MediaKind::Video
            };
            app.media_loading = true;
            let tx = app.tx.clone();
            let ctx = app.repaint.clone();
            std::thread::spawn(move || {
                let _ = tx.send(AppEvent::Media(
                    media::probe(&path, kind).map_err(|e| format!("{e:#}")),
                ));
                ctx.request_repaint();
            });
        }
        let tx = app.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(AppEvent::Encoder(
                recording::encoder::ffmpeg_path().map_err(|e| e.to_string()),
            ));
        });
        if app.smoke_dir.is_some() {
            // Explicit UI-only fixture; never used by a normal recording.
            let mut pixels = vec![0u8; 320 * 240 * 4];
            for y in 0..240 {
                for x in 0..320 {
                    let i = (y * 320 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[
                        (80 + y / 2) as u8,
                        (100 + x / 3) as u8,
                        180,
                        255,
                    ]);
                }
            }
            app.smoke_camera.set(VideoFrame {
                width: 320,
                height: 240,
                bgra: pixels,
                captured_at: Instant::now(),
            });
        }
        app
    }

    fn refresh(&mut self, request_permission: bool) {
        if self.refresh_pending {
            return;
        }
        self.refresh_pending = true;
        self.error = None;
        #[cfg(target_os = "linux")]
        {
            self.portal_picker =
                request_permission && std::env::var_os("WAYLAND_DISPLAY").is_some();
        }
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            if request_permission {
                // Let the UI unmap its own windows before a Wayland picker opens.
                #[cfg(target_os = "linux")]
                std::thread::sleep(Duration::from_millis(250));
                capture::request_screen_permission();
            }
            let sources = if capture::screen_permission() {
                capture::discover_sources().map_err(|e| e.to_string())
            } else {
                Ok(vec![])
            };
            let _ = tx.send(AppEvent::Sources(sources));
            repaint.request_repaint();
            let devices = (|| -> anyhow::Result<_> {
                Ok((
                    capture::discover_cameras()?,
                    capture::discover_microphones()?,
                ))
            })();
            let _ = tx.send(AppEvent::Devices(devices.map_err(|e| e.to_string())));
            repaint.request_repaint();
        });
    }

    fn poll(&mut self, ctx: &Context) {
        for event in self.events.try_iter() {
            match event {
                AppEvent::Media(result) => {
                    self.media_loading = false;
                    match result {
                        Ok(info) => {
                            self.camera_only = false;
                            self.media = Some(ui::media_canvas::MediaEditor::new(info));
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                AppEvent::Sources(result) => {
                    #[cfg(target_os = "linux")]
                    {
                        self.portal_picker = false;
                    }
                    match result {
                        Ok(sources) => {
                            let previous = self.sources.get(self.source).map(|s| (s.kind, s.id));
                            self.source = previous
                                .and_then(|p| sources.iter().position(|s| (s.kind, s.id) == p))
                                .unwrap_or(0);
                            self.sources = sources;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                AppEvent::Devices(result) => {
                    // Sources arrive first; discovery is only complete after
                    // the camera and microphone lists have arrived as well.
                    self.refresh_pending = false;
                    match result {
                        Ok((c, m)) => {
                            self.cameras = c;
                            self.microphones = m;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                AppEvent::Encoder(result) => match result {
                    Ok(_) => self.ffmpeg_ready = true,
                    Err(e) => self.error = Some(e),
                },
                AppEvent::Snapshot(result) => {
                    self.snapshot_pending = false;
                    match result {
                        Ok(p) => self.last_saved = Some(p),
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
        let mut camera_failed = None;
        if let Some(camera) = &self.camera {
            for event in camera.events.try_iter() {
                if let Err(e) = event {
                    camera_failed = Some(e);
                }
            }
        }
        if let Some(error) = camera_failed {
            self.error = Some(error);
            self.camera = None;
            self.camera_id = None;
            self.placement.lock().unwrap().visible = false;
        }
        let mut completed = false;
        if let Some(recording) = &self.recording {
            for event in recording.events.try_iter() {
                match event {
                    recording::Event::Ready => {
                        if self.phase == Phase::Starting {
                            self.countdown = Some(Countdown::default());
                        }
                    }
                    recording::Event::Started => {
                        if self.phase == Phase::Starting {
                            self.phase = Phase::Recording;
                        }
                    }
                    recording::Event::Paused => self.phase = Phase::Paused,
                    recording::Event::Resumed => self.phase = Phase::Recording,
                    recording::Event::Finalizing => self.phase = Phase::Stopping,
                    recording::Event::Saved(path) => {
                        self.last_saved = Some(path);
                        completed = true;
                        self.show_settings = true;
                    }
                    recording::Event::Discarded => completed = true,
                    recording::Event::Failed(e) => {
                        self.error = Some(e);
                        self.show_settings = true;
                        self.restart_after_stop = false;
                        self.quit_after_stop = false;
                        completed = true;
                    }
                }
            }
        }
        if completed {
            self.phase = Phase::Idle;
            self.countdown = None;
            self.recording = None;
            if self.quit_after_stop {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            if self.restart_after_stop {
                self.restart_after_stop = false;
                self.start();
            }
        }
    }

    fn camera_frames(&self) -> LatestFrame {
        self.camera
            .as_ref()
            .map(|c| c.frames.clone())
            .unwrap_or_else(|| self.smoke_camera.clone())
    }
    fn choose_media(&mut self, kind: media::MediaKind) {
        let dialog = rfd::FileDialog::new().set_title("Choose a recording background");
        let dialog = if kind == media::MediaKind::Image {
            dialog.add_filter("Images", &["png", "jpg", "jpeg"])
        } else {
            dialog.add_filter("Videos", &["mp4", "mov", "mkv", "webm"])
        };
        if let Some(path) = dialog.pick_file() {
            self.media_loading = true;
            self.error = None;
            let tx = self.tx.clone();
            let ctx = self.repaint.clone();
            std::thread::spawn(move || {
                let result = media::probe(&path, kind).map_err(|e| format!("{e:#}"));
                let _ = tx.send(AppEvent::Media(result));
                ctx.request_repaint();
            });
        }
    }
    fn selected_source(&self) -> Option<RecordingSource> {
        if self.camera_only {
            return self
                .camera_frames()
                .get()
                .filter(|frame| {
                    self.smoke_dir.is_some()
                        || (self.camera_id.is_some()
                            && frame.captured_at.elapsed() < Duration::from_secs(2))
                })
                .map(|frame| RecordingSource::Camera {
                    width: frame.width,
                    height: frame.height,
                });
        }
        self.media
            .as_ref()
            .map(|m| RecordingSource::Media(m.source.clone()))
            .or_else(|| {
                self.sources
                    .get(self.source)
                    .cloned()
                    .map(RecordingSource::Desktop)
            })
    }
    fn recording_placement(&self) -> Arc<Mutex<CameraPlacement>> {
        #[cfg(target_os = "linux")]
        if self.media.is_none() && !self.camera_only {
            return self.capture_canvas.placement.clone();
        }
        self.media
            .as_ref()
            .map(|m| m.placement.clone())
            .unwrap_or_else(|| self.placement.clone())
    }
    fn start(&mut self) {
        if self.phase.is_active()
            || !self.ffmpeg_ready
            || self.media_loading
            || self.snapshot_pending
        {
            return;
        }
        if let Some(media) = &mut self.media {
            media.prepare_recording();
            media.refresh(&self.repaint, &self.settings, true);
        }
        let Some(source) = self.selected_source() else {
            self.error = Some(
                if self.camera_only {
                    "Choose a camera and wait for its preview before recording."
                } else {
                    "Choose a recording source first."
                }
                .into(),
            );
            return;
        };
        #[cfg(target_os = "linux")]
        if self.media.is_none() && !self.camera_only {
            self.capture_canvas.prepare(
                source.dimensions(self.settings.quality),
                self.camera_id.is_some(),
                self.settings.mirror_camera,
            );
        }
        self.error = None;
        self.last_saved = None;
        self.phase = Phase::Starting;
        self.countdown = None;
        let bounds = match &source {
            RecordingSource::Desktop(source) => countdown_display(source, &self.sources),
            RecordingSource::Media(_) | RecordingSource::Camera { .. } => {
                let viewport = if self.camera_only {
                    "camera-preview"
                } else {
                    "media-canvas"
                };
                let rect = self
                    .repaint
                    .input_for(ViewportId::from_hash_of(viewport), |i| {
                        i.viewport().outer_rect
                    })
                    .unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)));
                Bounds {
                    x: rect.min.x as f64,
                    y: rect.min.y as f64,
                    width: rect.width() as f64,
                    height: rect.height() as f64,
                }
            }
        };
        self.countdown_position = pos2(
            (bounds.x + bounds.width * 0.5 - 120.0) as f32,
            (bounds.y + bounds.height * 0.5 - 140.0) as f32,
        );
        #[cfg(target_os = "windows")]
        if source.requires_screen_permission() {
            let scale = self.repaint.pixels_per_point();
            self.countdown_position = pos2(
                (bounds.x + bounds.width * 0.5) as f32 / scale - 120.0,
                (bounds.y + bounds.height * 0.5) as f32 / scale - 140.0,
            );
        }
        self.collapsed = false;
        self.recording = Some(Recording::spawn(RecordingRequest {
            settings: self.settings.clone(),
            source,
            microphone: self.microphone_id.clone(),
            camera: self.camera_frames(),
            placement: self.recording_placement(),
        }));
        let repaint = self.repaint.clone();
        self.recording
            .as_ref()
            .unwrap()
            .background
            .set_waker(Arc::new(move || repaint.request_repaint()));
    }
    fn stop(&mut self) {
        self.countdown = None;
        if let Some(recording) = &self.recording {
            recording.command(recording::Command::Stop);
            self.phase = Phase::Stopping;
        }
    }
    fn toggle_pause(&mut self) {
        if let Some(recording) = &self.recording {
            if self.phase == Phase::Recording {
                recording.command(recording::Command::Pause);
            } else if self.phase == Phase::Paused {
                recording.command(recording::Command::Resume);
            }
        }
    }
    fn change_camera(&mut self) {
        self.camera = None;
        self.camera_texture = None;
        self.camera_frame_time = None;
        if let Some(id) = self.camera_id.clone() {
            let camera = capture::Camera::start(id);
            let ctx = self.repaint.clone();
            camera
                .frames
                .set_waker(Arc::new(move || ctx.request_repaint()));
            self.camera = Some(camera);
        }
        let mut placement = self.placement.lock().unwrap();
        placement.visible = self.camera_id.is_some();
        placement.diameter = self.settings.camera_size as f64;
        placement.mirror = self.settings.mirror_camera;
    }
    fn snapshot(&mut self) {
        use crate::recording::source::FrameSource;
        if self.phase.is_active() || self.snapshot_pending || self.media_loading {
            return;
        }
        if let Some(media) = &mut self.media {
            media.prepare_recording();
        }
        let Some(source) = self.selected_source() else {
            return;
        };
        self.snapshot_pending = true;
        self.error = None;
        #[cfg(target_os = "linux")]
        if self.media.is_none() && !self.camera_only {
            self.capture_canvas.prepare(
                source.dimensions(self.settings.quality),
                self.camera_id.is_some(),
                self.settings.mirror_camera,
            );
        }
        let settings = self.settings.clone();
        let tx = self.tx.clone();
        let camera = self.camera_frames();
        let placement = self.recording_placement();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<PathBuf> {
                let frame = if source.is_camera() {
                    camera.clone()
                } else {
                    LatestFrame::default()
                };
                let (w, h) = source.dimensions(settings.quality);
                let mut capture = recording::source::Input::open(
                    &source,
                    w,
                    h,
                    30,
                    settings.show_cursor,
                    frame.clone(),
                )?;
                let started = Instant::now();
                let mut screen = loop {
                    if let Some(frame) = capture.frame_at(Instant::now())? {
                        break (*frame).clone();
                    }
                    capture.check()?;
                    anyhow::ensure!(
                        started.elapsed() < Duration::from_secs(12),
                        "Video capture timed out"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                };
                if source.is_camera() {
                    screen = CameraFrameRenderer::default()
                        .render(&screen, w, h, placement.lock().unwrap().mirror)
                        .clone();
                } else if let Some(camera) = camera.get() {
                    composite_camera(
                        &mut screen,
                        &camera,
                        *placement.lock().unwrap(),
                        capture.bounds(),
                    );
                }
                drop(capture);
                for pixel in screen.bgra.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                std::fs::create_dir_all(&settings.output_dir)?;
                let output = settings.output_dir.join(format!(
                    "Loomik {}-{}.png",
                    chrono::Local::now().format("%Y-%m-%d %H.%M.%S"),
                    &uuid::Uuid::new_v4().simple().to_string()[..6]
                ));
                image::save_buffer(&output, &screen.bgra, w, h, image::ColorType::Rgba8)?;
                Ok(output)
            })();
            let _ = tx.send(AppEvent::Snapshot(result.map_err(|e| format!("{e:#}"))));
        });
    }

    fn settings_panel(&mut self, ctx: &Context) {
        let height = if self.more_settings { 624.0 } else { 458.0 }
            + if self.media.is_some() { 40.0 } else { 0.0 }
            + if self.camera_only { 24.0 } else { 0.0 }
            + if cfg!(target_os = "linux") && self.media.is_none() && !self.camera_only {
                if self.sources.is_empty() { 24.0 } else { 64.0 }
            } else {
                0.0
            }
            + if (self.sources.is_empty() || cfg!(target_os = "linux"))
                && self.media.is_none()
                && !self.camera_only
                && capture::desktop_supported()
                && !self.refresh_pending
                && !self.phase.is_active()
            {
                40.0
            } else {
                0.0
            }
            + if self.error.is_some() { 60.0 } else { 0.0 };
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(340.0, height)));
        CentralPanel::default()
            .frame(ui::panel_frame(false))
            .show(ctx, |ui| {
                ui::window_drag(ui, ctx.content_rect().shrink(5.0));
                ui.horizontal(|ui| {
                    ui.add(
                        Label::new(RichText::new(icons::RECORD).color(ui::ORANGE).size(27.0))
                            .sense(Sense::hover()),
                    );
                    ui.add(
                        Label::new(RichText::new("Loomik").size(18.0).strong())
                            .sense(Sense::hover()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui::icon_button(ui, icons::X, "Hide settings", 26.0, false).clicked() {
                            self.show_settings = false;
                        }
                    });
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(ui::SURFACE)
                        .corner_radius(12)
                        .inner_margin(4)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_enabled_ui(
                                !self.phase.is_active() && !self.snapshot_pending,
                                |ui| {
                                    for (still, icon, label) in [
                                        (false, icons::VIDEO_CAMERA, "Video"),
                                        (true, icons::CAMERA, "Photo"),
                                    ] {
                                        let active = self.snapshot_mode == still;
                                        if ui
                                            .add_sized(
                                                [126.0, 34.0],
                                                Button::new(
                                                    RichText::new(format!("{icon}  {label}"))
                                                        .color(if active {
                                                            ui::BLUE
                                                        } else {
                                                            ui::MUTED
                                                        }),
                                                )
                                                .fill(if active {
                                                    ui::WHITE
                                                } else {
                                                    Color32::TRANSPARENT
                                                })
                                                .corner_radius(9),
                                            )
                                            .clicked()
                                        {
                                            self.snapshot_mode = still;
                                        }
                                    }
                                },
                            );
                        });
                });
                ui.add_space(8.0);
                ui.add_enabled_ui(!self.phase.is_active() && !self.snapshot_pending && !self.media_loading, |ui| {
                    let selected = if self.media_loading { "Loading background…".into() }
                        else if self.camera_only { "Camera only".into() }
                        else if let Some(media)=&self.media { media.source.info.path.file_name().unwrap_or_default().to_string_lossy().into_owned() }
                        else {self.sources.get(self.source).map(|s|s.name.clone()).unwrap_or_else(||"Choose a source".into())};
                    let width=ui.available_width();
                    let mut picked_media=None;
                    let mut picked_desktop=false;
                    let mut picked_camera=false;
                    let source_icon=if self.camera_only { icons::VIDEO_CAMERA } else { match self.media.as_ref().map(|m|m.source.info.kind) {
                        Some(media::MediaKind::Image)=>icons::IMAGE,
                        Some(media::MediaKind::Video)=>icons::FILM_STRIP,
                        None=>icons::MONITOR,
                    }};
                    let select=ui::Select::new("source",&selected,width).icon(source_icon).show(ui,|ui| {
                        let mut camera_only = self.camera_only;
                        if ui::select_option(ui, &mut camera_only, true, "Camera only").clicked() { picked_camera=true; }
                        ui.separator();
                        let mut desktop = if self.media.is_none() && !self.camera_only { Some(self.source) } else { None };
                        for (i,source) in self.sources.iter().enumerate() {
                            if ui::select_option(ui,&mut desktop,Some(i),&source.name).clicked() {self.source=i; picked_desktop=true;}
                        }
                        if self.sources.is_empty() {
                            ui::select_hint(ui,if capture::desktop_supported() {"Screen capture needs permission. Camera and file backgrounds work without it."} else {"Desktop capture is not available on this OS yet. Choose a camera or file background."});
                        }
                        ui.separator();
                        let mut kind=None;
                        if ui::select_option(ui,&mut kind,Some(media::MediaKind::Image),"Image file…").clicked() {picked_media=kind;}
                        if ui::select_option(ui,&mut kind,Some(media::MediaKind::Video),"Video file…").clicked() {picked_media=kind;}
                    });
                    self.select_smoke(ctx,"source",&select.response);
                    if picked_desktop {self.media=None; self.camera_only=false;}
                    if picked_camera {self.media=None; self.camera_only=true; self.error=None;}
                    if let Some(kind)=picked_media {self.choose_media(kind);}
                    let previous = self.camera_id.clone();
                    let label = self
                        .camera_id
                        .as_ref()
                        .and_then(|id| self.cameras.iter().find(|c| &c.id == id))
                        .map(|d| d.name.as_str())
                        .unwrap_or(if self.camera_only { "Choose a camera" } else { "Camera off" });
                    let select = ui::Select::new("camera", label, width)
                        .icon(icons::VIDEO_CAMERA)
                        .show(ui, |ui| {
                            ui::select_option(ui, &mut self.camera_id, None, "Camera off");
                            for device in &self.cameras {
                                ui::select_option(
                                    ui,
                                    &mut self.camera_id,
                                    Some(device.id.clone()),
                                    &device.name,
                                );
                            }
                            if self.cameras.is_empty() {
                                ui::select_hint(ui, "No cameras connected");
                            }
                        });
                    self.select_smoke(ctx, "camera", &select.response);
                    if previous != self.camera_id {
                        self.change_camera();
                    }
                    let label = self
                        .microphone_id
                        .as_ref()
                        .and_then(|id| self.microphones.iter().find(|d| &d.id == id))
                        .map(|d| d.name.as_str())
                        .unwrap_or("Microphone off");
                    let select = ui::Select::new("microphone", label, width)
                        .icon(icons::MICROPHONE)
                        .accent(self.microphone_id.is_some())
                        .show(ui, |ui| {
                            ui::select_option(ui, &mut self.microphone_id, None, "Microphone off");
                            for device in &self.microphones {
                                ui::select_option(
                                    ui,
                                    &mut self.microphone_id,
                                    Some(device.id.clone()),
                                    &device.name,
                                );
                            }
                        });
                    self.select_smoke(ctx, "microphone", &select.response);
                });
                if self.camera_only {
                    let hint = if self.camera_id.is_none() && self.smoke_dir.is_none() { "Choose a camera to record." }
                        else if self.selected_source().is_none() { "Waiting for the camera…" }
                        else { "Full camera frame · No screen capture" };
                    ui.label(RichText::new(hint).size(11.0).color(ui::MUTED));
                }
                if let Some(media)=&mut self.media && ui.add_sized([ui.available_width(),30.0],Button::new("Open background studio").fill(ui::SURFACE).corner_radius(10)).clicked() {media.visible=true;}
                #[cfg(target_os = "linux")]
                if self.media.is_none() && !self.camera_only {
                    ui.label(RichText::new("Window capture · Whole-display capture unavailable").size(11.0).color(ui::MUTED));
                    if !self.sources.is_empty() && ui.add_sized([ui.available_width(),30.0],Button::new("Open recording studio").fill(ui::SURFACE).corner_radius(10)).clicked() {self.capture_canvas.visible=true;}
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            Button::new(
                                RichText::new(format!(
                                    "{}  {} · {} fps",
                                    icons::SLIDERS_HORIZONTAL,
                                    self.settings.format.label(),
                                    self.settings.fps
                                ))
                                .small()
                                .color(ui::MUTED),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.more_settings = !self.more_settings;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.phase.is_active() && !self.refresh_pending,
                                Button::new(
                                    RichText::new(icons::ARROWS_CLOCKWISE).color(ui::MUTED),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Refresh connected devices and screens")
                            .clicked()
                        {
                            self.refresh(false);
                        }
                    });
                });
                if self.more_settings {
                    self.advanced_settings(ui);
                }
                if (self.sources.is_empty() || cfg!(target_os = "linux")) && self.media.is_none() && !self.camera_only && capture::desktop_supported()
                    && !self.refresh_pending && !self.phase.is_active()
                    && ui
                        .add_sized(
                            [ui.available_width(), 36.0],
                            Button::new(if cfg!(target_os = "linux") {"Choose window…"} else {"Allow Screen Recording"}).fill(ui::SURFACE),
                        )
                        .clicked()
                {
                    self.refresh(true);
                }
                self.feedback(ui);
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{}  Saved only on this device",
                                icons::LOCK_SIMPLE
                            ))
                            .size(11.0)
                            .color(ui::MUTED),
                        );
                    });
                    ui.add_space(5.0);
                    if self.phase.is_active() {
                        let elapsed = self
                            .recording
                            .as_ref()
                            .map(|r| format_duration(r.elapsed()))
                            .unwrap_or_else(|| "0:00".into());
                        let label = match self.phase {
                            Phase::Starting => {
                                if let Some(countdown) = &self.countdown {
                                    format!("Cancel countdown  ·  {}", countdown.digit())
                                } else {
                                    "Cancel start…".into()
                                }
                            }
                            Phase::Stopping => "Saving your recording…".into(),
                            _ => format!("{}  Stop & save  ·  {elapsed}", icons::STOP),
                        };
                        ui.add_enabled_ui(
                            matches!(
                                self.phase,
                                Phase::Starting | Phase::Recording | Phase::Paused
                            ),
                            |ui| {
                                if ui::primary(ui, &label, ui.available_width()).clicked() {
                                    self.stop();
                                }
                            },
                        );
                    } else {
                        let enabled = self.selected_source().is_some() && !self.media_loading
                            && !self.snapshot_pending
                            && (self.snapshot_mode || self.ffmpeg_ready);
                        let label = if self.snapshot_pending {
                            "Capturing…"
                        } else if self.snapshot_mode {
                            if self.camera_only { "Take Photo" } else { "Take Screenshot" }
                        } else {
                            "Start Recording"
                        };
                        ui.add_enabled_ui(enabled, |ui| {
                            if ui::primary(ui, label, ui.available_width()).clicked() {
                                if self.snapshot_mode {
                                    self.snapshot();
                                } else {
                                    self.start();
                                }
                            }
                        });
                    }
                });
            });
    }

    fn advanced_settings(&mut self, ui: &mut Ui) {
        ui.add_enabled_ui(!self.phase.is_active(), |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(88.0);
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.label(RichText::new("Format").size(11.0).color(ui::MUTED));
                    let select = ui::Select::new("format", self.settings.format.label(), 88.0)
                        .show(ui, |ui| {
                            for format in Format::ALL {
                                ui::select_option(
                                    ui,
                                    &mut self.settings.format,
                                    format,
                                    format.label(),
                                );
                            }
                        });
                    self.select_smoke(ui.ctx(), "format", &select.response);
                });
                ui.vertical(|ui| {
                    ui.set_width(192.0);
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.label(RichText::new("Quality").size(11.0).color(ui::MUTED));
                    let select = ui::Select::new("quality", self.settings.quality.label(), 192.0)
                        .show(ui, |ui| {
                            for quality in Quality::ALL {
                                ui::select_option(
                                    ui,
                                    &mut self.settings.quality,
                                    quality,
                                    quality.label(),
                                );
                            }
                        });
                    self.select_smoke(ui.ctx(), "quality", &select.response);
                });
            });
            ui.horizontal(|ui| {
                let select = ui::Select::new("fps", &format!("{} fps", self.settings.fps), 88.0)
                    .show(ui, |ui| {
                        for fps in [15, 30, 60] {
                            ui::select_option(
                                ui,
                                &mut self.settings.fps,
                                fps,
                                &format!("{fps} fps"),
                            );
                        }
                    });
                self.select_smoke(ui.ctx(), "fps", &select.response);
                if self.media.is_none() && !self.camera_only && !cfg!(target_os = "linux") {
                    ui.checkbox(&mut self.settings.show_cursor, "Show cursor");
                }
            });
            if ui
                .add_sized(
                    [ui.available_width(), 34.0],
                    Button::new(
                        RichText::new(format!(
                            "{}  {}",
                            icons::FOLDER,
                            truncate(&self.settings.output_dir.to_string_lossy(), 34)
                        ))
                        .size(12.0),
                    )
                    .fill(ui::SURFACE)
                    .corner_radius(10),
                )
                .on_hover_text(self.settings.output_dir.to_string_lossy())
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.settings.output_dir)
                    .pick_folder()
            {
                self.settings.output_dir = path;
            }
        });
        if self.camera_only {
            ui.label(
                RichText::new("The full camera frame is saved")
                    .small()
                    .color(ui::MUTED),
            );
        } else if self.media.is_some() {
            ui.label(
                RichText::new("Resize the camera in Background studio")
                    .small()
                    .color(ui::MUTED),
            );
        } else if cfg!(target_os = "linux") {
            ui.label(
                RichText::new("Move and resize the camera in Recording studio")
                    .small()
                    .color(ui::MUTED),
            );
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Camera size").small().color(ui::MUTED));
                ui.add(
                    Slider::new(&mut self.settings.camera_size, 100.0..=420.0).show_value(false),
                );
            });
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.settings.mirror_camera, "Mirror camera");
            if !self.camera_only {
                ui.label(
                    RichText::new("Drag circle to move")
                        .size(11.0)
                        .color(ui::MUTED),
                );
            }
        });
        let mut p = self.placement.lock().unwrap();
        p.diameter = self.settings.camera_size as f64;
        p.mirror = self.settings.mirror_camera;
    }

    fn feedback(&mut self, ui: &mut Ui) {
        if let Some(error) = self.error.clone() {
            Frame::new()
                .fill(Color32::from_rgb(255, 240, 234))
                .corner_radius(10)
                .inner_margin(9)
                .show(ui, |ui| {
                    ScrollArea::vertical().max_height(58.0).show(ui, |ui| {
                        ui.label(
                            RichText::new(error)
                                .size(12.0)
                                .color(Color32::from_rgb(157, 53, 28)),
                        );
                    });
                });
        } else if let Some(path) = &self.last_saved {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{}  Saved", icons::CHECK_CIRCLE))
                        .size(12.0)
                        .color(Color32::from_rgb(42, 126, 75)),
                );
                if ui
                    .link(
                        RichText::new(if cfg!(target_os = "macos") {
                            "Show in Finder"
                        } else {
                            "Show in folder"
                        })
                        .size(12.0),
                    )
                    .clicked()
                {
                    let _ = crate::platform::reveal(path);
                }
            });
        } else {
            ui.label(
                RichText::new(if self.phase == Phase::Paused {
                    "Paused. Take your time."
                } else if self.phase.is_active() {
                    "Controls stay out of your recording."
                } else {
                    "No time limit. Make it your own."
                })
                .size(12.0)
                .color(ui::MUTED),
            );
        }
    }

    fn request_quit(&mut self, ctx: &Context) {
        match self.phase {
            Phase::Starting => {
                self.quit_after_stop = true;
                self.stop();
            }
            Phase::Stopping => self.quit_after_stop = true,
            Phase::Recording | Phase::Paused => {
                self.confirmation = Some(Confirmation::Quit);
                self.show_settings = true;
            }
            _ => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
        }
    }

    fn toolbar(&mut self, ctx: &Context) {
        let height = if self.collapsed { 174.0 } else { 354.0 };
        let elapsed = if self.smoke_dir.is_some()
            && !self.hover_smoke()
            && self.launched.elapsed() > Duration::from_secs(6)
        {
            "100:00:00".into()
        } else {
            self.recording
                .as_ref()
                .map(|r| format_duration(r.elapsed()))
                .unwrap_or_else(|| "0:00".into())
        };
        let width = (elapsed.len() as f32 * 7.5 + 30.0).max(78.0);
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("toolbar"),
            ui::floating("Loomik controls", vec2(width, height))
                // This is the initial position. The native window and the
                // drag handler own later moves; don't feed rounded OS readings
                // back into the builder on hover/repaint.
                .with_position(pos2(24.0, 160.0)),
            |ctx, _| {
                // The builder emits a resize only when size actually changes.
                CentralPanel::default()
                    .frame(ui::panel_frame(true))
                    .show(ctx, |ui| {
                        ui::window_drag(ui, ctx.content_rect().shrink(5.0));
                        ui.visuals_mut().override_text_color = Some(ui::WHITE);
                        ui.vertical_centered(|ui| {
                            ui.add_sized(
                                [40.0, 16.0],
                                Label::new(
                                    RichText::new(icons::DOTS_SIX_VERTICAL)
                                        .size(16.0)
                                        .color(ui::MUTED),
                                )
                                .sense(Sense::hover()),
                            )
                            .on_hover_text("Drag controls");
                            let active = self.phase.is_active();
                            let busy = self.phase == Phase::Stopping;
                            // Color-only feedback: an appearing outside stroke
                            // also enlarges the painted button by one pixel.
                            let widgets = &mut ui.visuals_mut().widgets;
                            // Immediate native viewports can select a different
                            // system theme than the root. Apply geometry here,
                            // to the actual toolbar style used for this frame.
                            for state in [
                                &mut widgets.inactive,
                                &mut widgets.hovered,
                                &mut widgets.active,
                            ] {
                                state.expansion = 0.0;
                                state.bg_stroke = Stroke::NONE;
                            }
                            widgets.inactive.weak_bg_fill = if active {
                                ui::ORANGE
                            } else {
                                Color32::from_rgb(62, 63, 69)
                            };
                            widgets.hovered.weak_bg_fill = if active {
                                Color32::from_rgb(249, 89, 57)
                            } else {
                                Color32::from_rgb(76, 77, 84)
                            };
                            widgets.active.weak_bg_fill = if active {
                                Color32::from_rgb(220, 64, 36)
                            } else {
                                Color32::from_rgb(53, 54, 60)
                            };
                            let button = ui
                                .add_enabled(
                                    !busy
                                        && (active
                                            || (self.ffmpeg_ready
                                                && self.selected_source().is_some()
                                                && !self.media_loading
                                                && !self.snapshot_pending)),
                                    Button::new(
                                        RichText::new(if active {
                                            icons::STOP
                                        } else {
                                            icons::RECORD
                                        })
                                        .size(26.0)
                                        .color(if active { ui::WHITE } else { ui::ORANGE }),
                                    )
                                    .min_size(vec2(42.0, 42.0))
                                    .corner_radius(12),
                                )
                                .on_hover_text(if self.phase == Phase::Starting {
                                    "Cancel start · Esc"
                                } else if active {
                                    "Stop and save · ⌘⇧S"
                                } else {
                                    "Start recording · ⌘⇧R"
                                });
                            if button.clicked() {
                                if active {
                                    self.stop();
                                } else {
                                    self.start();
                                }
                            }
                            ui.add(
                                Label::new(RichText::new(&elapsed).monospace().size(11.5).color(
                                    if self.phase == Phase::Paused {
                                        Color32::from_rgb(253, 190, 97)
                                    } else {
                                        Color32::from_rgb(181, 185, 195)
                                    },
                                ))
                                .sense(Sense::hover()),
                            );
                            if !self.collapsed {
                                ui.add_space(2.0);
                                ui.add_enabled_ui(
                                    matches!(self.phase, Phase::Recording | Phase::Paused),
                                    |ui| {
                                        if ui::icon_button(
                                            ui,
                                            if self.phase == Phase::Paused {
                                                icons::PLAY
                                            } else {
                                                icons::PAUSE
                                            },
                                            if self.phase == Phase::Paused {
                                                "Resume · Space"
                                            } else {
                                                "Pause · Space"
                                            },
                                            38.0,
                                            true,
                                        )
                                        .clicked()
                                        {
                                            self.toggle_pause();
                                        }
                                    },
                                );
                                ui.add_enabled_ui(
                                    matches!(self.phase, Phase::Recording | Phase::Paused),
                                    |ui| {
                                        if ui::icon_button(
                                            ui,
                                            icons::ARROW_COUNTER_CLOCKWISE,
                                            "Start over",
                                            38.0,
                                            true,
                                        )
                                        .clicked()
                                        {
                                            self.confirmation = Some(Confirmation::Restart);
                                            self.show_settings = true;
                                        }
                                        if ui::icon_button(
                                            ui,
                                            icons::TRASH,
                                            "Discard recording",
                                            38.0,
                                            true,
                                        )
                                        .clicked()
                                        {
                                            self.confirmation = Some(Confirmation::Discard);
                                            self.show_settings = true;
                                        }
                                    },
                                );
                                if ui::icon_button(
                                    ui,
                                    icons::SLIDERS_HORIZONTAL,
                                    "Recording settings",
                                    38.0,
                                    true,
                                )
                                .clicked()
                                {
                                    self.show_settings = !self.show_settings;
                                }
                                ui.add_space(2.0);
                                ui.separator();
                            }
                            if ui::icon_button(
                                ui,
                                if self.collapsed {
                                    icons::CARET_DOUBLE_RIGHT
                                } else {
                                    icons::CARET_DOUBLE_LEFT
                                },
                                if self.collapsed {
                                    "Expand controls"
                                } else {
                                    "Collapse controls"
                                },
                                24.0,
                                true,
                            )
                            .clicked()
                            {
                                self.collapsed = !self.collapsed;
                            }
                            ui.add_space(2.0);
                            if ui::icon_button(ui, icons::X, "Quit Loomik", 24.0, true).clicked() {
                                self.request_quit(ctx);
                            }
                        });
                    });
                self.shortcuts(ctx);
                self.smoke_capture(ctx, "toolbar");
            },
        );
    }

    fn camera_view(&mut self, ctx: &Context) {
        let visible = self.camera.is_some() || self.smoke_dir.is_some();
        if !visible {
            return;
        }
        let frames = self.camera_frames();
        let camera = frames.get();
        if let Some(frame) = &camera
            && self.camera_frame_time != Some(frame.captured_at)
        {
            let pixels = frame
                .bgra
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| Color32::from_rgb(p[2], p[1], p[0]))
                .collect();
            let image = ColorImage::new([frame.width as usize, frame.height as usize], pixels);
            if let Some(texture) = &mut self.camera_texture {
                texture.set(image, TextureOptions::LINEAR);
            } else {
                self.camera_texture =
                    Some(ctx.load_texture("camera", image, TextureOptions::LINEAR));
            }
            self.camera_frame_time = Some(frame.captured_at);
        }
        if self.camera_only {
            self.camera_only_view(ctx, camera.as_deref());
            return;
        }
        if self.media.is_some() || cfg!(target_os = "linux") {
            return;
        }
        let size = self.settings.camera_size;
        let p = *self.placement.lock().unwrap();
        let camera_position = if cfg!(target_os = "windows") {
            pos2(52.0, 512.0)
        } else {
            pos2(p.x as f32 - 8.0, p.y as f32 - 8.0)
        };
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("camera"),
            ui::floating("Loomik camera", vec2(size + 16.0, size + 16.0))
                .with_position(camera_position),
            |ctx, _| {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(size + 16.0, size + 16.0)));
                CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
                    let rect = Rect::from_min_size(pos2(8.0, 8.0), vec2(size, size));
                    ui::window_drag(ui, rect);
                    if let Some(texture) = &self.camera_texture {
                        let aspect = camera
                            .as_ref()
                            .map(|f| f.width as f32 / f.height as f32)
                            .unwrap_or(4.0 / 3.0);
                        ui.painter().add(Shape::mesh(ui::camera_mesh(
                            texture.id(),
                            rect,
                            self.settings.mirror_camera,
                            aspect,
                        )));
                    } else {
                        ui.painter()
                            .circle_filled(rect.center(), size * 0.5, ui::DARK);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "Connecting camera…",
                            FontId::proportional(13.0),
                            ui::WHITE,
                        );
                    }
                    ui.painter().circle_stroke(
                        rect.center(),
                        size * 0.5,
                        Stroke::new(2.0_f32, Color32::from_white_alpha(220)),
                    );
                    if ui.rect_contains_pointer(rect) {
                        let controls = Rect::from_center_size(
                            rect.center() + vec2(0.0, size * 0.31),
                            vec2(96.0, 28.0),
                        );
                        ui.painter()
                            .rect_filled(controls, 14, Color32::from_black_alpha(165));
                        for (i, label) in ["−", "+"].iter().enumerate() {
                            let r = Rect::from_min_size(
                                controls.min + vec2(i as f32 * 48.0, 0.0),
                                vec2(48.0, 28.0),
                            );
                            let hit = ui.interact(r, Id::new(("resize-camera", i)), Sense::click());
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                *label,
                                FontId::proportional(20.0),
                                ui::WHITE,
                            );
                            if hit.clicked() {
                                self.settings.camera_size = (self.settings.camera_size
                                    + if i == 0 { -25.0 } else { 25.0 })
                                .clamp(100.0, 420.0);
                            }
                        }
                        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                        if scroll != 0.0 {
                            self.settings.camera_size =
                                (self.settings.camera_size + scroll * 0.2).clamp(100.0, 420.0);
                        }
                    }
                });
                if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                    let mut p = self.placement.lock().unwrap();
                    p.x = rect.min.x as f64 + 8.0;
                    p.y = rect.min.y as f64 + 8.0;
                    p.diameter = self.settings.camera_size as f64;
                    #[cfg(target_os = "windows")]
                    if let Some(native) = crate::platform::windows::camera_bounds(
                        8.0,
                        self.settings.camera_size as f64,
                        ctx.pixels_per_point() as f64,
                    ) {
                        p.x = native.x;
                        p.y = native.y;
                        p.diameter = native.diameter;
                    }
                    p.mirror = self.settings.mirror_camera;
                    p.visible = true;
                }
                self.smoke_capture(ctx, "camera");
                self.shortcuts(ctx);
            },
        );
    }

    fn camera_only_view(&mut self, ctx: &Context, frame: Option<&VideoFrame>) {
        let aspect = frame.map_or(4.0 / 3.0, |frame| frame.width as f32 / frame.height as f32);
        let preview_size = vec2(480.0_f32.min(480.0 * aspect), 480.0_f32.min(480.0 / aspect));
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("camera-preview"),
            ui::floating("Loomik camera preview", preview_size + vec2(34.0, 66.0))
                .with_resizable(true)
                .with_min_inner_size(vec2(260.0, 200.0))
                .with_position(pos2(460.0, 130.0)),
            |ctx, _| {
                CentralPanel::default()
                    .frame(ui::panel_frame(false).inner_margin(12))
                    .show(ctx, |ui| {
                        ui::window_drag(ui, ctx.content_rect().shrink(5.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Camera only").strong());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(frame) = frame {
                                    let (width, height) =
                                        self.settings.quality.dimensions(frame.width, frame.height);
                                    ui.label(
                                        RichText::new(format!("{width} × {height}"))
                                            .small()
                                            .color(ui::MUTED),
                                    );
                                }
                            });
                        });
                        let available =
                            (ui.available_size() - vec2(0.0, 10.0)).max(Vec2::splat(1.0));
                        let (canvas, _) = ui.allocate_exact_size(available, Sense::hover());
                        let height = canvas.height().min(canvas.width() / aspect);
                        let rect =
                            Rect::from_center_size(canvas.center(), vec2(height * aspect, height));
                        ui.painter().rect_filled(canvas, 4, ui::DARK);
                        if let Some(texture) = &self.camera_texture {
                            let uv = if self.settings.mirror_camera {
                                Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0))
                            } else {
                                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
                            };
                            ui.painter().image(texture.id(), rect, uv, ui::WHITE);
                        } else {
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                "Connecting camera…",
                                FontId::proportional(14.0),
                                ui::WHITE,
                            );
                        }
                        let grip = Rect::from_min_size(
                            ctx.content_rect().max - vec2(35.0, 33.0),
                            vec2(20.0, 20.0),
                        );
                        let resize = ui
                            .interact(
                                grip,
                                Id::new("camera-preview-resize"),
                                Sense::click_and_drag(),
                            )
                            .on_hover_cursor(CursorIcon::ResizeNwSe)
                            .on_hover_text("Drag to resize preview");
                        let anchor_id = resize.id.with("anchor");
                        if resize.drag_started()
                            && let Some((pointer, size)) = ctx.input(|i| {
                                Some((i.pointer.press_origin()?, i.viewport().inner_rect?.size()))
                            })
                        {
                            ctx.data_mut(|data| data.insert_temp(anchor_id, (pointer, size)));
                        }
                        if resize.dragged()
                            && let Some(pointer) = resize.interact_pointer_pos()
                            && let Some((start, size)) =
                                ctx.data(|data| data.get_temp::<(Pos2, Vec2)>(anchor_id))
                        {
                            ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                                (size + (pointer - start)).max(vec2(260.0, 200.0)),
                            ));
                        }
                        if !ctx.input(|i| i.pointer.primary_down()) {
                            ctx.data_mut(|data| data.remove::<(Pos2, Vec2)>(anchor_id));
                        }
                        for inset in [7.0, 12.0] {
                            ui.painter().line_segment(
                                [
                                    grip.right_bottom() - vec2(inset, 2.0),
                                    grip.right_bottom() - vec2(2.0, inset),
                                ],
                                Stroke::new(1.5_f32, ui::MUTED),
                            );
                        }
                    });
                self.smoke_capture(ctx, "camera-preview");
                self.shortcuts(ctx);
            },
        );
    }

    #[cfg(target_os = "linux")]
    fn capture_studio(&mut self, ctx: &Context) {
        if self.media.is_some() || self.camera_only {
            return;
        }
        let Some(source) = self.sources.get(self.source) else {
            return;
        };
        let dimensions = RecordingSource::Desktop(source.clone()).dimensions(self.settings.quality);
        self.capture_canvas.prepare(
            dimensions,
            self.camera_id.is_some(),
            self.settings.mirror_camera,
        );
        let frame = self.camera.as_ref().and_then(|c| c.frames.get());
        let action = self.capture_canvas.show(
            ctx,
            &source.name,
            ui::media_canvas::CanvasState {
                settings: &self.settings,
                camera: self.camera_texture.as_ref(),
                camera_aspect: frame
                    .as_ref()
                    .map_or(4.0 / 3.0, |f| f.width as f32 / f.height as f32),
                camera_enabled: self.camera_id.is_some(),
                active: self.phase.is_active(),
                starting: self.phase == Phase::Starting,
                elapsed: self
                    .recording
                    .as_ref()
                    .map_or(Duration::ZERO, |r| r.elapsed()),
                background: self.recording.as_ref().and_then(|r| r.background.get()),
                capture: None,
            },
        );
        match action {
            Some(ui::media_canvas::CanvasAction::Start) => self.start(),
            Some(ui::media_canvas::CanvasAction::Stop) => self.stop(),
            Some(ui::media_canvas::CanvasAction::Pause) => self.toggle_pause(),
            None => {}
        }
    }

    #[cfg(target_os = "linux")]
    fn hide_for_portal(&self, ctx: &Context) -> bool {
        if !self.portal_picker {
            return false;
        }
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(false));
        for name in [
            "toolbar",
            "camera",
            "media-canvas",
            "capture-canvas",
            "countdown",
        ] {
            ctx.send_viewport_cmd_to(
                ViewportId::from_hash_of(name),
                ViewportCommand::Visible(false),
            );
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        true
    }

    fn shortcuts(&mut self, ctx: &Context) {
        if self.phase == Phase::Starting
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.stop();
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R)) {
            self.start();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)) {
            self.stop();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)) {
            self.toggle_pause();
        }
    }

    fn countdown_view(&mut self, ctx: &Context) {
        if self.countdown.is_none() {
            return;
        }
        let mut begin = false;
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("countdown"),
            ui::floating("Loomik countdown", vec2(240.0, 280.0))
                .with_position(self.countdown_position),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.stop();
                    return;
                }
                self.shortcuts(ctx);
                let Some(countdown) = self.countdown.as_mut() else {
                    return;
                };
                let Some(digit) = countdown.tick(Instant::now()) else {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                    begin = true;
                    return;
                };
                CentralPanel::default()
                    .frame(ui::panel_frame(true))
                    .show(ctx, |ui| {
                        ui::window_drag(ui, ctx.content_rect().shrink(5.0));
                        let painter = ui.painter();
                        painter.text(
                            pos2(120.0, 38.0),
                            Align2::CENTER_CENTER,
                            "GET READY",
                            FontId::proportional(12.0),
                            Color32::from_rgb(181, 185, 195),
                        );
                        painter.circle_stroke(
                            pos2(120.0, 133.0),
                            72.0,
                            Stroke::new(2.0_f32, ui::ORANGE),
                        );
                        painter.text(
                            pos2(120.0, 128.0),
                            Align2::CENTER_CENTER,
                            digit.to_string(),
                            FontId::proportional(88.0),
                            ui::WHITE,
                        );
                        if ui
                            .put(
                                Rect::from_min_size(pos2(40.0, 227.0), vec2(160.0, 32.0)),
                                Button::new(
                                    RichText::new("Cancel · Esc").size(13.0).color(ui::WHITE),
                                )
                                .fill(Color32::from_rgb(62, 63, 69))
                                .corner_radius(10),
                            )
                            .clicked()
                        {
                            self.stop();
                        }
                    });
                if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                    self.countdown_position = rect.min;
                }
                if let Some(verify) = &mut self.native_verification {
                    verify.countdown_timer_zero &= self
                        .recording
                        .as_ref()
                        .is_some_and(|r| r.elapsed().is_zero());
                    if verify.countdown_seen.last() != Some(&digit) {
                        verify.countdown_seen.push(digit);
                        if digit == 0 {
                            verify.zero_shown_at = Some(Instant::now());
                        }
                        save_viewport_png(ctx, verify.dir.join(format!("countdown-{digit}.png")));
                    }
                }
                ctx.request_repaint_after(Duration::from_millis(16));
            },
        );
        if begin
            && self.phase == Phase::Starting
            && self.countdown.take().is_some()
            && let Some(recording) = &self.recording
        {
            if let Some(verify) = &mut self.native_verification {
                verify.began_after_zero = verify
                    .zero_shown_at
                    .is_some_and(|t| t.elapsed() >= Duration::from_millis(290));
            }
            recording.command(recording::Command::Begin);
        }
    }

    fn confirm(&mut self, ctx: &Context) {
        let Some(action) = self.confirmation else {
            return;
        };
        egui::Modal::new(Id::new("confirm-recording-action")).show(ctx, |ui| {
            ui.set_width(240.0);
            ui::window_drag(ui, Rect::from_min_size(ui.cursor().min, vec2(240.0, 30.0)));
            ui.heading(match action {
                Confirmation::Restart => "Start over?",
                Confirmation::Discard => "Discard this recording?",
                Confirmation::Quit => "Finish before quitting?",
            });
            ui.add_space(8.0);
            ui.label(if action == Confirmation::Quit {
                "Your recording will be saved before Loomik closes."
            } else {
                "This removes the current take. It cannot be undone."
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.confirmation = None;
                }
                if ui
                    .button(match action {
                        Confirmation::Restart => "Start over",
                        Confirmation::Discard => "Discard",
                        Confirmation::Quit => "Save & quit",
                    })
                    .clicked()
                {
                    self.confirmation = None;
                    if action == Confirmation::Quit {
                        self.quit_after_stop = true;
                        self.stop();
                    } else {
                        self.restart_after_stop = action == Confirmation::Restart;
                        if let Some(r) = &self.recording {
                            r.command(recording::Command::Discard);
                            self.phase = Phase::Stopping;
                        }
                    }
                }
            });
        });
    }

    fn select_smoke(&self, ctx: &Context, name: &str, response: &Response) {
        if self.select_smoke_name() == Some(name)
            && response.enabled()
            && !Popup::is_id_open(ctx, response.id.with("popup"))
        {
            Popup::open_id(ctx, response.id.with("popup"));
        }
    }
    fn hover_smoke(&self) -> bool {
        self.smoke_dir.is_some() && std::env::args().any(|arg| arg == "--hover-smoke")
    }
    fn select_smoke_name(&self) -> Option<&'static str> {
        if self.smoke_dir.is_none() || !std::env::args().any(|a| a == "--select-smoke") {
            return None;
        }
        let stage = (self.launched.elapsed().as_secs_f64() / 2.0) as usize;
        [
            "closed",
            "closed",
            "source",
            "camera",
            "microphone",
            "format",
            "quality",
            "fps",
            "disabled",
        ]
        .get(stage)
        .copied()
    }

    fn smoke_capture(&self, ctx: &Context, name: &str) {
        if let Some(dir) = &self.smoke_dir {
            if self.smoke_shot {
                // eframe's Glow backend doesn't process Screenshot commands for
                // immediate child viewports. Capture after their last draw call.
                let suffix = if self.hover_smoke() {
                    let step =
                        ((self.launched.elapsed().as_secs_f64() - 1.0).max(0.0) / 0.5) as usize;
                    ui::hover_check::POINTS[step.min(ui::hover_check::POINTS.len() - 1)].0
                } else if let Some(name) = self.select_smoke_name() {
                    match name {
                        "source" => "-select-source",
                        "camera" => "-select-camera",
                        "microphone" => "-select-microphone",
                        "format" => "-select-format",
                        "quality" => "-select-quality",
                        "fps" => "-select-fps",
                        "disabled" => "-select-disabled",
                        _ => "-select-closed",
                    }
                } else if self.launched.elapsed() > Duration::from_secs(6) {
                    "-long-timer"
                } else if self.collapsed {
                    "-collapsed"
                } else if self.more_settings {
                    "-advanced"
                } else {
                    ""
                };
                let output = dir.join(format!("{name}{suffix}.png"));
                let size = ctx
                    .input(|i| i.viewport().inner_rect.map(|r| r.size()))
                    .unwrap_or_else(|| ctx.content_rect().size())
                    * ctx.pixels_per_point();
                let pixels = [size.x as u32, size.y as u32];
                ctx.layer_painter(LayerId::new(Order::Debug, Id::new("smoke-capture")))
                    .add(PaintCallback {
                        rect: ctx.content_rect(),
                        callback: Arc::new(eframe::egui_glow::CallbackFn::new(
                            move |_info, painter| {
                                let shot = painter.read_screen_rgba(pixels);
                                let _ = std::fs::create_dir_all(output.parent().unwrap());
                                let bytes: Vec<u8> =
                                    shot.pixels.iter().flat_map(|p| p.to_array()).collect();
                                let _ = image::save_buffer(
                                    &output,
                                    &bytes,
                                    shot.width() as u32,
                                    shot.height() as u32,
                                    image::ColorType::Rgba8,
                                );
                            },
                        )),
                    });
            }
            ctx.input(|i| {
                for event in &i.events {
                    if let Event::Screenshot { image, .. } = event {
                        let _ = std::fs::create_dir_all(dir);
                        let bytes: Vec<u8> =
                            image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        let _ = image::save_buffer(
                            dir.join(format!("{name}.png")),
                            &bytes,
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        );
                    }
                }
            });
        }
    }

    fn verify_native(&mut self, ctx: &Context) {
        let Some(mut verify) = self.native_verification.take() else {
            return;
        };
        let _ = std::fs::create_dir_all(&verify.dir);
        if let Some(error) = &self.error {
            let _ = std::fs::write(
                verify.dir.join("native-result.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"status":"failed","error":error,"stage":verify.stage}),
                )
                .unwrap(),
            );
            return;
        }
        match verify.stage {
            0 => {
                if self.media_loading {
                    // File probing is asynchronous and never requests Screen Recording.
                } else if self.media.is_none() && !self.camera_only && !capture::screen_permission()
                {
                    if !verify.requested_permission && !self.refresh_pending {
                        verify.requested_permission = true;
                        self.refresh(true);
                        let _ = std::fs::write(
                            verify.dir.join("permission-required.txt"),
                            "Allow Loomik in macOS Screen & System Audio Recording. Camera and microphone prompts follow. Then press Refresh in Loomik if needed.",
                        );
                    }
                } else if self.media.is_none() && !self.camera_only && self.sources.is_empty() {
                    if !self.refresh_pending {
                        self.refresh(false);
                    }
                } else if self.ffmpeg_ready && !self.refresh_pending {
                    self.source = 0;
                    self.settings.output_dir = verify.dir.clone();
                    self.settings.quality = Quality::Balanced;
                    self.settings.format = Format::Mp4;
                    if let Some(media) = &mut self.media {
                        media.source.options.source_audio = media.source.info.has_audio;
                        media.source.options.loop_video =
                            std::env::args().any(|arg| arg == "--media-loop");
                    }
                    let args: Vec<_> = std::env::args().collect();
                    self.settings.fps = args
                        .iter()
                        .position(|a| a == "--verify-fps")
                        .and_then(|i| args.get(i + 1))
                        .and_then(|s| s.parse().ok())
                        .filter(|fps| [15, 30, 60].contains(fps))
                        .unwrap_or(60);
                    self.camera_id = self.cameras.first().map(|d| d.id.clone());
                    self.microphone_id = self
                        .microphones
                        .iter()
                        .find(|d| self.cameras.first().is_some_and(|c| c.name == d.name))
                        .or(self.microphones.first())
                        .map(|d| d.id.clone());
                    self.change_camera();
                    verify.stage = 1;
                    verify.entered = Instant::now();
                }
            }
            1 => {
                if self.camera_id.is_none() || self.camera_frames().get().is_some() {
                    self.start();
                    verify.stage = 10;
                    verify.entered = Instant::now();
                }
            }
            10 => {
                if self.countdown.as_ref().is_some_and(|c| c.digit() == 2) {
                    verify.cancelled_countdown = self
                        .recording
                        .as_ref()
                        .is_some_and(|r| r.elapsed().is_zero());
                    self.stop();
                    verify.stage = 11;
                }
            }
            11 if self.phase == Phase::Idle => {
                verify.cancelled_countdown &= self.last_saved.is_none()
                    && std::fs::read_dir(&verify.dir).is_ok_and(|entries| {
                        !entries
                            .flatten()
                            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "mp4"))
                    });
                verify.countdown_seen.clear();
                self.start();
                verify.stage = 2;
                verify.entered = Instant::now();
            }
            2 => {
                if self.phase == Phase::Recording
                    && self
                        .recording
                        .as_ref()
                        .is_some_and(|r| r.elapsed() > Duration::from_secs(2))
                {
                    self.toggle_pause();
                    verify.stage = 3;
                    verify.entered = Instant::now();
                }
            }
            3 => {
                if self.phase == Phase::Paused {
                    self.show_settings = false;
                    verify.paused_time = self.recording.as_ref().unwrap().elapsed();
                    verify.stage = 4;
                    verify.entered = Instant::now();
                    self.settings.camera_size = 270.0;
                    if let Some(media) = &mut self.media {
                        let (w, h) = media.source.dimensions(self.settings.quality);
                        let mut p = media.placement.lock().unwrap();
                        p.x = w as f64 * 0.52;
                        p.y = h as f64 * 0.44;
                        p.diameter = w.min(h) as f64 * 0.35;
                    }
                    ctx.send_viewport_cmd_to(
                        ViewportId::from_hash_of("camera"),
                        ViewportCommand::OuterPosition(pos2(580.0, 300.0)),
                    );
                }
            }
            4 => {
                if verify.entered.elapsed() > Duration::from_secs(2) {
                    self.show_settings = true;
                    let after = self.recording.as_ref().unwrap().elapsed();
                    verify.pause_verified = after == verify.paused_time;
                    self.toggle_pause();
                    verify.stage = 5;
                    verify.entered = Instant::now();
                }
            }
            5 => {
                if self.phase == Phase::Recording
                    && self.recording.as_ref().unwrap().elapsed() > Duration::from_secs(5)
                {
                    self.stop();
                    verify.stage = 6;
                    verify.entered = Instant::now();
                }
            }
            6 if self.phase == Phase::Idle => {
                verify.video_path = self.last_saved.clone();
                self.snapshot();
                verify.stage = 7;
            }
            7 if !self.snapshot_pending => {
                let p = *self.recording_placement().lock().unwrap();
                let source = if self.camera_only {
                    Some("Camera only".to_owned())
                } else {
                    self.media
                        .as_ref()
                        .map(|m| m.source.info.path.to_string_lossy().into_owned())
                        .or_else(|| self.sources.get(self.source).map(|s| s.name.clone()))
                };
                let _=std::fs::write(verify.dir.join("native-result.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"recorded","output":verify.video_path,"screenshot":self.last_saved,"pause_timer_verified":verify.pause_verified,"paused_at_seconds":verify.paused_time.as_secs_f64(),"camera":self.camera_id,"microphone":self.microphone_id,"camera_placement":{"x":p.x,"y":p.y,"diameter":p.diameter},"source":source,"media_background":self.media.as_ref().map(|m|&m.source),"countdown_digits":verify.countdown_seen,"countdown_timer_zero":verify.countdown_timer_zero,"began_after_zero":verify.began_after_zero,"cancelled_countdown_without_movie":verify.cancelled_countdown})).unwrap());
                self.camera = None;
                self.camera_id = None;
                self.placement.lock().unwrap().visible = false;
                ctx.send_viewport_cmd(ViewportCommand::Close);
                return;
            }
            _ => {}
        }
        self.native_verification = Some(verify);
    }
}

impl eframe::App for LoomikApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        #[cfg(target_os = "linux")]
        if self.hide_for_portal(ctx) {
            return;
        }
        self.verify_native(ctx);
        if self.smoke_dir.is_some() && !self.hover_smoke() {
            self.more_settings = self.launched.elapsed() > Duration::from_secs(4);
            self.collapsed = self.select_smoke_name().is_none()
                && self.launched.elapsed() > Duration::from_secs(5);
            if self.select_smoke_name() == Some("microphone") {
                // Select a label only; UI smoke never opens an audio stream.
                self.microphone_id = self.microphones.first().map(|d| d.id.clone());
            }
            if self.select_smoke_name() == Some("disabled") {
                Popup::close_all(ctx);
                self.phase = Phase::Recording;
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.phase.is_active() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if self.phase == Phase::Starting {
                self.quit_after_stop = true;
                self.stop();
            } else {
                self.confirmation = Some(Confirmation::Quit);
            }
            self.show_settings = true;
        }
        ctx.send_viewport_cmd(ViewportCommand::Visible(self.show_settings));
        if self.show_settings {
            self.settings_panel(ctx);
            self.confirm(ctx);
        }
        #[cfg(target_os = "linux")]
        if self.hide_for_portal(ctx) {
            return;
        }
        self.shortcuts(ctx);
        self.smoke_shot = self.smoke_dir.is_some()
            && self.launched.elapsed() > Duration::from_secs(3)
            && (self.select_smoke_name().is_some()
                || self.launched.elapsed() < Duration::from_secs(7));
        self.toolbar(ctx);
        self.camera_view(ctx);
        #[cfg(target_os = "linux")]
        self.capture_studio(ctx);
        if let Some(media) = &mut self.media {
            let frame = self.camera.as_ref().and_then(|c| c.frames.get());
            media.show(
                ctx,
                ui::media_canvas::CanvasState {
                    settings: &self.settings,
                    camera: self.camera_texture.as_ref(),
                    camera_aspect: frame
                        .as_ref()
                        .map_or(4.0 / 3.0, |f| f.width as f32 / f.height as f32),
                    camera_enabled: self.camera_id.is_some() || self.smoke_dir.is_some(),
                    active: self.phase.is_active(),
                    starting: self.phase == Phase::Starting,
                    elapsed: self
                        .recording
                        .as_ref()
                        .map_or(Duration::ZERO, |r| r.elapsed()),
                    background: self.recording.as_ref().and_then(|r| r.background.get()),
                    capture: if self.smoke_shot {
                        self.smoke_dir
                            .as_ref()
                            .map(|d| d.join("background-studio.png"))
                    } else {
                        None
                    },
                },
            );
        }
        if let Some(action) = self.media.as_mut().and_then(|m| m.action.take()) {
            match action {
                ui::media_canvas::CanvasAction::Start => self.start(),
                ui::media_canvas::CanvasAction::Stop => self.stop(),
                ui::media_canvas::CanvasAction::Pause => self.toggle_pause(),
            }
        }
        self.countdown_view(ctx);
        self.smoke_capture(ctx, "settings");
        if let Some(dir) = &self.smoke_dir
            && self.launched.elapsed()
                > Duration::from_secs(if std::env::args().any(|a| a == "--select-smoke") {
                    18
                } else {
                    8
                })
        {
            let report = serde_json::json!({"screen_permission":capture::screen_permission(),"sources":self.sources.iter().map(|s|&s.name).collect::<Vec<_>>(),"cameras":self.cameras.iter().map(|d|&d.name).collect::<Vec<_>>(),"microphones":self.microphones.iter().map(|d|&d.name).collect::<Vec<_>>(),"ffmpeg_ready":self.ffmpeg_ready,"error":self.error});
            let _ = std::fs::write(
                dir.join("ui-smoke.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            );
            self.phase = Phase::Idle;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(if self.camera.is_some() {
            33
        } else {
            80
        }));
    }
    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if !self.testing {
            eframe::set_value(storage, "settings", &self.settings);
        }
    }
}

fn countdown_display(source: &Source, sources: &[Source]) -> Bounds {
    if source.kind == SourceKind::Display {
        return source.bounds;
    }
    // A captured window may straddle screens. Use the display containing the
    // largest part of it, including displays with negative desktop coordinates.
    sources
        .iter()
        .filter(|s| s.kind == SourceKind::Display)
        .max_by(|a, b| {
            let overlap = |display: &Source| {
                let d = display.bounds;
                let w = source.bounds;
                ((d.x + d.width).min(w.x + w.width) - d.x.max(w.x)).max(0.0)
                    * ((d.y + d.height).min(w.y + w.height) - d.y.max(w.y)).max(0.0)
            };
            overlap(a).total_cmp(&overlap(b))
        })
        .map_or(source.bounds, |display| display.bounds)
}

pub(crate) fn save_viewport_png(ctx: &Context, output: PathBuf) {
    let size = ctx
        .input(|i| i.viewport().inner_rect.map(|r| r.size()))
        .unwrap_or_else(|| ctx.content_rect().size())
        * ctx.pixels_per_point();
    let pixels = [size.x as u32, size.y as u32];
    ctx.layer_painter(LayerId::new(Order::Debug, Id::new("verification-capture")))
        .add(PaintCallback {
            rect: ctx.content_rect(),
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                let shot = painter.read_screen_rgba(pixels);
                let _ = std::fs::create_dir_all(output.parent().unwrap());
                let bytes: Vec<u8> = shot.pixels.iter().flat_map(|p| p.to_array()).collect();
                let _ = image::save_buffer(
                    &output,
                    &bytes,
                    shot.width() as u32,
                    shot.height() as u32,
                    image::ColorType::Rgba8,
                );
            })),
        });
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn countdown_uses_selected_display_or_largest_window_overlap() {
        let make = |kind, x, width| Source {
            id: 1,
            kind,
            name: String::new(),
            bounds: Bounds {
                x,
                y: 0.0,
                width,
                height: 900.0,
            },
            pixel_width: 1440,
            pixel_height: 900,
        };
        let left = make(SourceKind::Display, -1440.0, 1440.0);
        let right = make(SourceKind::Display, 0.0, 1440.0);
        let displays = vec![left.clone(), right.clone()];
        assert_eq!(countdown_display(&left, &displays), left.bounds);
        assert_eq!(
            countdown_display(&make(SourceKind::Window, -600.0, 800.0), &displays),
            left.bounds
        );
        assert_eq!(
            countdown_display(&make(SourceKind::Window, -100.0, 800.0), &displays),
            right.bounds
        );
    }
}
