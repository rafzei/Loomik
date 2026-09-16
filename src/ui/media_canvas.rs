use super::*;
use crate::{
    media::{self, Aspect, Fit, MediaInfo, MediaKind, MediaSource, Options},
    model::{CameraPlacement, Settings},
    recording::frame::{LatestFrame, VideoFrame},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct Preview {
    frames: LatestFrame,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
}
impl Preview {
    fn new(source: MediaSource, dimensions: (u32, u32), playing: bool, ctx: Context) -> Self {
        let frames = LatestFrame::default();
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (out, err, cancel) = (frames.clone(), error.clone(), stop.clone());
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let mut reader = media::Reader::open(&source, dimensions.0, dimensions.1, 30)?;
                let mut frame_index = 0u64;
                let start = Instant::now();
                while !cancel.load(Ordering::Relaxed) {
                    if frame_index == 0
                        || (playing
                            && source.info.kind == MediaKind::Video
                            && frame_index as f64 / 30.0 <= start.elapsed().as_secs_f64())
                    {
                        let frame = reader.next_frame()?;
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        out.set_shared(frame);
                        ctx.request_repaint();
                        frame_index += 1;
                        if !playing || source.info.kind == MediaKind::Image {
                            break;
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(8));
                    }
                }
                Ok(())
            })();
            if let Err(e) = result
                && !cancel.load(Ordering::Relaxed)
            {
                *err.lock().unwrap() = Some(format!("{e:#}"));
                ctx.request_repaint();
            }
        });
        Self {
            frames,
            error,
            stop,
        }
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct MediaEditor {
    pub source: MediaSource,
    pub placement: Arc<Mutex<CameraPlacement>>,
    pub visible: bool,
    pub action: Option<CanvasAction>,
    preview: Option<Preview>,
    texture: Option<TextureHandle>,
    texture_frame: Option<Instant>,
    playing: bool,
    play_started: Instant,
    dimensions: (u32, u32),
    preview_key: Option<(Options, (u32, u32), bool)>,
    seek_draft: Option<f64>,
}
pub enum CanvasAction {
    Start,
    Stop,
    Pause,
}
pub struct CanvasState<'a> {
    pub settings: &'a Settings,
    pub camera: Option<&'a TextureHandle>,
    pub camera_aspect: f32,
    pub camera_enabled: bool,
    pub active: bool,
    pub starting: bool,
    pub elapsed: Duration,
    pub background: Option<Arc<VideoFrame>>,
    pub capture: Option<std::path::PathBuf>,
}
impl MediaEditor {
    fn visual_options(&self) -> Options {
        let mut options = self.source.options.clone();
        // Audio sliders must not restart the video decoder during a gesture.
        options.source_audio = false;
        options.source_volume = 1.0;
        options.microphone_volume = 1.0;
        options
    }
    pub fn new(info: MediaInfo) -> Self {
        Self {
            source: MediaSource {
                info,
                options: Options::default(),
            },
            placement: Arc::new(Mutex::new(CameraPlacement {
                x: 30.0,
                y: 30.0,
                diameter: 180.0,
                visible: false,
                mirror: true,
            })),
            visible: true,
            action: None,
            preview: None,
            texture: None,
            texture_frame: None,
            playing: false,
            play_started: Instant::now(),
            dimensions: (0, 0),
            preview_key: None,
            seek_draft: None,
        }
    }
    fn playhead(&self) -> f64 {
        if !self.playing {
            return self.source.options.start;
        }
        let seconds = self.source.options.start + self.play_started.elapsed().as_secs_f64();
        if self.source.options.loop_video {
            seconds % self.source.info.duration.max(0.001)
        } else {
            seconds.min((self.source.info.duration - 0.001).max(0.0))
        }
    }
    pub fn prepare_recording(&mut self) -> MediaSource {
        self.source.options.start = self.playhead();
        self.playing = false;
        self.preview = None;
        self.preview_key = None;
        self.source.clone()
    }
    pub fn refresh(&mut self, ctx: &Context, settings: &Settings, active: bool) {
        let dimensions = self.source.dimensions(settings.quality);
        if self.dimensions != dimensions {
            let mut p = self.placement.lock().unwrap();
            if self.dimensions.0 > 0 {
                p.x *= dimensions.0 as f64 / self.dimensions.0 as f64;
                p.y *= dimensions.1 as f64 / self.dimensions.1 as f64;
                p.diameter *= dimensions.0.min(dimensions.1) as f64
                    / self.dimensions.0.min(self.dimensions.1) as f64;
            } else {
                p.diameter = dimensions.0.min(dimensions.1) as f64 * 0.27;
                p.x = dimensions.0 as f64 * 0.04;
                p.y = dimensions.1 as f64 - p.diameter - dimensions.1 as f64 * 0.04;
            }
            self.dimensions = dimensions;
        }
        if !active && self.visible {
            let key = (self.visual_options(), dimensions, self.playing);
            if self.preview_key.as_ref() != Some(&key) {
                if self.playing {
                    self.source.options.start = self.playhead();
                    self.play_started = Instant::now();
                }
                let (w, h) = crate::model::Quality::Compact.dimensions(dimensions.0, dimensions.1);
                self.preview = Some(Preview::new(
                    self.source.clone(),
                    (w, h),
                    self.playing,
                    ctx.clone(),
                ));
                self.preview_key = Some((self.visual_options(), dimensions, self.playing));
            }
        } else {
            self.preview = None;
            self.preview_key = None;
        }
    }
    pub fn show(&mut self, ctx: &Context, state: CanvasState<'_>) {
        self.action = None;
        self.refresh(ctx, state.settings, state.active);
        {
            let mut placement = self.placement.lock().unwrap();
            placement.visible = state.camera_enabled;
            placement.mirror = state.settings.mirror_camera;
        }
        if !self.visible {
            return;
        }
        let frame = state
            .background
            .or_else(|| self.preview.as_ref().and_then(|p| p.frames.get()));
        if let Some(frame) = frame
            && self.texture_frame != Some(frame.captured_at)
        {
            let image = ColorImage::new(
                [frame.width as usize, frame.height as usize],
                frame
                    .bgra
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| Color32::from_rgb(p[2], p[1], p[0]))
                    .collect(),
            );
            if let Some(texture) = &mut self.texture {
                texture.set(image, TextureOptions::LINEAR);
            } else {
                self.texture =
                    Some(ctx.load_texture("media-canvas", image, TextureOptions::LINEAR));
            }
            self.texture_frame = Some(frame.captured_at);
        }
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("media-canvas"),
            floating("Loomik · Background", vec2(760.0, 700.0))
                .with_position(pos2(480.0, 160.0))
                .with_resizable(true)
                .with_min_inner_size(vec2(440.0, 500.0)),
            |ctx, _| {
                if !ctx.wants_keyboard_input() {
                    if ctx
                        .input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R))
                    {
                        self.action = Some(CanvasAction::Start);
                    }
                    if ctx
                        .input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S))
                    {
                        self.action = Some(CanvasAction::Stop);
                    }
                    if state.active && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
                    {
                        self.action = Some(CanvasAction::Pause);
                    }
                }
                if state.starting && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                {
                    self.action = Some(CanvasAction::Stop);
                }
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.visible = false;
                    self.playing = false;
                    self.preview = None;
                    return;
                }
                CentralPanel::default()
                    .frame(panel_frame(false))
                    .show(ctx, |ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                        ui.spacing_mut().button_padding = vec2(12.0, 8.0);
                        ui.spacing_mut().interact_size.y = 30.0;
                        ui.spacing_mut().slider_width = 180.0;
                        ui.style_mut()
                            .text_styles
                            .insert(TextStyle::Body, FontId::proportional(14.0));
                        ui.style_mut()
                            .text_styles
                            .insert(TextStyle::Button, FontId::proportional(14.0));
                        ui.style_mut()
                            .text_styles
                            .insert(TextStyle::Small, FontId::proportional(11.0));
                        ui.visuals_mut().override_text_color = Some(INK);
                        let widgets = &mut ui.visuals_mut().widgets;
                        for widget in [&mut widgets.inactive, &mut widgets.hovered] {
                            widget.corner_radius = CornerRadius::same(8);
                        }
                        window_drag(ui, ctx.content_rect().shrink(5.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Background studio").strong().size(17.0));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if icon_button(
                                    ui,
                                    egui_phosphor::regular::X,
                                    "Hide studio",
                                    26.0,
                                    false,
                                )
                                .clicked()
                                {
                                    self.visible = false;
                                    self.playing = false;
                                }
                            });
                        });
                        ui.add(
                            Label::new(
                                RichText::new(
                                    self.source
                                        .info
                                        .path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy(),
                                )
                                .color(MUTED)
                                .size(12.0),
                            )
                            .truncate(),
                        );
                        let available = vec2(
                            ui.available_width(),
                            (ui.available_height() - 228.0).max(140.0),
                        );
                        let aspect = self.dimensions.0 as f32 / self.dimensions.1 as f32;
                        let size = vec2(
                            available.x.min(available.y * aspect),
                            available.y.min(available.x / aspect),
                        );
                        let (area, _) = ui.allocate_exact_size(available, Sense::hover());
                        let canvas = Rect::from_center_size(area.center(), size);
                        ui.painter()
                            .rect_filled(canvas, 0, Color32::from_rgb(24, 25, 28));
                        if let Some(texture) = &self.texture {
                            ui.painter().image(
                                texture.id(),
                                canvas,
                                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                                WHITE,
                            );
                        } else {
                            ui.painter().text(
                                canvas.center(),
                                Align2::CENTER_CENTER,
                                "Preparing background…",
                                FontId::proportional(14.0),
                                WHITE,
                            );
                        }
                        let scale = canvas.width() / self.dimensions.0 as f32;
                        let mut p = self.placement.lock().unwrap();
                        p.visible = state.camera_enabled;
                        p.mirror = state.settings.mirror_camera;
                        if p.visible {
                            let circle = Rect::from_min_size(
                                canvas.min + vec2(p.x as f32, p.y as f32) * scale,
                                Vec2::splat(p.diameter as f32 * scale),
                            );
                            let hit = ui
                                .interact(circle, Id::new("canvas-camera"), Sense::click_and_drag())
                                .on_hover_cursor(CursorIcon::Grab);
                            if let Some(texture) = state.camera {
                                ui.painter()
                                    .with_clip_rect(canvas)
                                    .add(Shape::mesh(camera_mesh(
                                        texture.id(),
                                        circle,
                                        p.mirror,
                                        state.camera_aspect,
                                    )));
                            }
                            if hit.dragged() {
                                let delta = hit.drag_delta() / scale;
                                p.x += delta.x as f64;
                                p.y += delta.y as f64;
                            }
                            if hit.hovered() {
                                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                                p.diameter = (p.diameter + scroll as f64 / scale as f64 * 0.2)
                                    .clamp(
                                        self.dimensions.0.min(self.dimensions.1) as f64 * 0.1,
                                        self.dimensions.0.min(self.dimensions.1) as f64 * 0.8,
                                    );
                            }
                            p.x = p.x.clamp(
                                -p.diameter * 0.5,
                                self.dimensions.0 as f64 - p.diameter * 0.5,
                            );
                            p.y = p.y.clamp(
                                -p.diameter * 0.5,
                                self.dimensions.1 as f64 - p.diameter * 0.5,
                            );
                        }
                        drop(p);
                        ScrollArea::vertical()
                            .max_height(ui.available_height())
                            .show(ui, |ui| {
                                if self.source.info.kind == MediaKind::Video {
                                    ui.add_enabled_ui(!state.active, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            if ui
                                                .add(
                                                    Button::new(if self.playing {
                                                        "Pause preview"
                                                    } else {
                                                        "Play preview"
                                                    })
                                                    .fill(SURFACE)
                                                    .corner_radius(10),
                                                )
                                                .clicked()
                                            {
                                                if self.playing {
                                                    self.source.options.start = self.playhead();
                                                }
                                                self.playing = !self.playing;
                                                self.play_started = Instant::now();
                                            }
                                            let mut position = if state.active {
                                                let position = self.source.options.start
                                                    + state.elapsed.as_secs_f64();
                                                if self.source.options.loop_video {
                                                    position % self.source.info.duration.max(0.001)
                                                } else {
                                                    position.min(self.source.info.duration)
                                                }
                                            } else {
                                                self.seek_draft.unwrap_or_else(|| self.playhead())
                                            };
                                            let seek = ui.add(
                                                Slider::new(
                                                    &mut position,
                                                    0.0..=(self.source.info.duration - 0.001)
                                                        .max(0.0),
                                                )
                                                .show_value(false),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} / {}",
                                                    crate::model::format_duration(
                                                        Duration::from_secs_f64(position.max(0.0))
                                                    ),
                                                    crate::model::format_duration(
                                                        Duration::from_secs_f64(
                                                            self.source.info.duration
                                                        )
                                                    )
                                                ))
                                                .monospace()
                                                .size(12.0)
                                                .color(MUTED),
                                            );
                                            if seek.changed() {
                                                self.seek_draft = Some(position);
                                            }
                                            if (seek.changed() && !seek.dragged())
                                                || seek.drag_stopped()
                                            {
                                                let position =
                                                    self.seek_draft.take().unwrap_or(position);
                                                self.source.options.start = position;
                                                self.play_started = Instant::now();
                                                self.playing = false;
                                            }
                                            ui.checkbox(
                                                &mut self.source.options.loop_video,
                                                "Loop",
                                            );
                                        });
                                    });
                                }
                                ui.add_enabled_ui(!state.active, |ui| {
                                    ui.horizontal(|ui| {
                                        Select::new(
                                            "canvas-aspect",
                                            self.source.options.aspect.label(),
                                            110.0,
                                        )
                                        .show(ui, |ui| {
                                            for a in Aspect::ALL {
                                                select_option(
                                                    ui,
                                                    &mut self.source.options.aspect,
                                                    a,
                                                    a.label(),
                                                );
                                            }
                                        });
                                        Select::new(
                                            "canvas-fit",
                                            if self.source.options.fit == Fit::Fit {
                                                "Fit"
                                            } else {
                                                "Fill"
                                            },
                                            90.0,
                                        )
                                        .show(ui, |ui| {
                                            select_option(
                                                ui,
                                                &mut self.source.options.fit,
                                                Fit::Fit,
                                                "Fit",
                                            );
                                            select_option(
                                                ui,
                                                &mut self.source.options.fit,
                                                Fit::Fill,
                                                "Fill",
                                            );
                                        });
                                        ui.label(
                                            RichText::new(format!(
                                                "{} × {}",
                                                self.dimensions.0, self.dimensions.1
                                            ))
                                            .small()
                                            .color(MUTED),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.add_enabled(
                                            self.source.info.has_audio,
                                            Checkbox::new(
                                                &mut self.source.options.source_audio,
                                                "Video audio",
                                            ),
                                        );
                                        ui.add_enabled(
                                            self.source.info.has_audio
                                                && self.source.options.source_audio,
                                            Slider::new(
                                                &mut self.source.options.source_volume,
                                                0.0..=2.0,
                                            )
                                            .show_value(false),
                                        );
                                        ui.label(format!(
                                            "{:.0}%",
                                            self.source.options.source_volume * 100.0
                                        ));
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Microphone level");
                                        ui.add(
                                            Slider::new(
                                                &mut self.source.options.microphone_volume,
                                                0.0..=2.0,
                                            )
                                            .show_value(false),
                                        );
                                        ui.label(format!(
                                            "{:.0}%",
                                            self.source.options.microphone_volume * 100.0
                                        ));
                                    });
                                });
                                ui.horizontal_wrapped(|ui| {
                                    let mut p = self.placement.lock().unwrap();
                                    let min = self.dimensions.0.min(self.dimensions.1) as f64;
                                    let mut ratio = p.diameter / min;
                                    if ui
                                        .add_enabled(
                                            p.visible,
                                            Slider::new(&mut ratio, 0.1..=0.8)
                                                .text("Camera size")
                                                .show_value(false),
                                        )
                                        .changed()
                                    {
                                        p.diameter = ratio * min;
                                    }
                                    ui.label(
                                        RichText::new("Drag camera to position it")
                                            .small()
                                            .color(MUTED),
                                    );
                                });
                                if let Some(preview) = &self.preview
                                    && let Some(error) = preview.error.lock().unwrap().as_ref()
                                {
                                    ui.colored_label(Color32::DARK_RED, error);
                                }
                                if self.source.info.kind == MediaKind::Video {
                                    ui.label(
                                        RichText::new(
                                            "Silent preview · Audio levels apply to the recording",
                                        )
                                        .size(11.0)
                                        .color(MUTED),
                                    );
                                }
                            });
                    });
                if let Some(path) = &state.capture {
                    crate::app::save_viewport_png(ctx, path.clone());
                }
            },
        );
    }
}
