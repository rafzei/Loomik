//! Canvas-relative camera controls for Linux, where global window positioning
//! and capture exclusion are not portable across desktop compositors.
use super::{
    media_canvas::{CanvasAction, CanvasState},
    *,
};
use crate::model::CameraPlacement;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub struct CaptureCanvas {
    pub visible: bool,
    pub placement: Arc<Mutex<CameraPlacement>>,
    dimensions: (u32, u32),
    texture: Option<TextureHandle>,
    frame_time: Option<Instant>,
}
impl Default for CaptureCanvas {
    fn default() -> Self {
        Self {
            visible: true,
            placement: Arc::new(Mutex::new(CameraPlacement {
                x: 30.0,
                y: 30.0,
                diameter: 180.0,
                visible: false,
                mirror: true,
            })),
            dimensions: (0, 0),
            texture: None,
            frame_time: None,
        }
    }
}
impl CaptureCanvas {
    pub fn prepare(&mut self, dimensions: (u32, u32), camera: bool, mirror: bool) {
        let mut placement = self.placement.lock().unwrap();
        if self.dimensions != dimensions {
            if self.dimensions.0 > 0 {
                placement.x *= dimensions.0 as f64 / self.dimensions.0 as f64;
                placement.y *= dimensions.1 as f64 / self.dimensions.1 as f64;
                placement.diameter *= dimensions.0.min(dimensions.1) as f64
                    / self.dimensions.0.min(self.dimensions.1) as f64;
            }
            self.dimensions = dimensions;
            self.texture = None;
            self.frame_time = None;
        }
        placement.visible = camera;
        placement.mirror = mirror;
    }
    pub fn show(
        &mut self,
        ctx: &Context,
        title: &str,
        state: CanvasState<'_>,
    ) -> Option<CanvasAction> {
        if !self.visible {
            return None;
        }
        if let Some(frame) = &state.background
            && self.frame_time != Some(frame.captured_at)
        {
            let pixels = frame
                .bgra
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| Color32::from_rgb(p[2], p[1], p[0]))
                .collect();
            let image = ColorImage::new([frame.width as usize, frame.height as usize], pixels);
            if let Some(texture) = &mut self.texture {
                texture.set(image, TextureOptions::LINEAR);
            } else {
                self.texture =
                    Some(ctx.load_texture("capture-canvas", image, TextureOptions::LINEAR));
            }
            self.frame_time = Some(frame.captured_at);
        }
        if !state.active {
            self.texture = None;
            self.frame_time = None;
        }
        let mut action = None;
        ctx.show_viewport_immediate(ViewportId::from_hash_of("capture-canvas"),floating("Loomik recording studio",vec2(800.0,580.0)).with_resizable(true).with_min_inner_size(vec2(480.0,360.0)),|ctx,_| {
            if ctx.input(|i|i.viewport().close_requested()) {self.visible=false;return;}
            CentralPanel::default().frame(panel_frame(false)).show(ctx,|ui| {
                window_drag(ui,ctx.content_rect().shrink(5.0));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Recording studio").strong().size(17.0));
                    ui.with_layout(Layout::right_to_left(Align::Center),|ui| {
                        if icon_button(ui,egui_phosphor::regular::X,"Hide studio",26.0,false).clicked() {self.visible=false;}
                    });
                });
                ui.add(Label::new(RichText::new(title).small().color(MUTED)).truncate());
                let available=vec2(ui.available_width(),(ui.available_height()-110.0).max(120.0));
                let aspect=self.dimensions.0 as f32 / self.dimensions.1.max(1) as f32;
                let size=vec2(available.x.min(available.y*aspect),available.y.min(available.x/aspect));
                let (area,_)=ui.allocate_exact_size(available,Sense::hover());
                let canvas=Rect::from_center_size(area.center(),size);
                ui.painter().rect_filled(canvas,0,DARK);
                if let Some(texture)=&self.texture {
                    ui.painter().image(texture.id(),canvas,Rect::from_min_max(Pos2::ZERO,pos2(1.0,1.0)),WHITE);
                } else {
                    ui.painter().text(canvas.center(),Align2::CENTER_CENTER,"Start recording to preview the shared window",FontId::proportional(14.0),WHITE);
                }
                let scale=canvas.width()/self.dimensions.0.max(1) as f32;
                let mut p=self.placement.lock().unwrap();
                if p.visible {
                    let circle=Rect::from_min_size(canvas.min+vec2(p.x as f32,p.y as f32)*scale,Vec2::splat(p.diameter as f32*scale));
                    let hit=ui.interact(circle,Id::new("capture-camera"),Sense::click_and_drag()).on_hover_cursor(CursorIcon::Grab);
                    if let Some(texture)=state.camera {ui.painter().with_clip_rect(canvas).add(Shape::mesh(camera_mesh(texture.id(),circle,p.mirror,state.camera_aspect)));}
                    if hit.dragged() {let delta=hit.drag_delta()/scale;p.x+=delta.x as f64;p.y+=delta.y as f64;}
                    if hit.hovered() {p.diameter+=ctx.input(|i|i.smooth_scroll_delta.y) as f64/scale as f64*0.2;}
                    let min=self.dimensions.0.min(self.dimensions.1) as f64;
                    p.diameter=p.diameter.clamp(min*0.1,min*0.8);
                    p.x=p.x.clamp(0.0,(self.dimensions.0 as f64-p.diameter).max(0.0));
                    p.y=p.y.clamp(0.0,(self.dimensions.1 as f64-p.diameter).max(0.0));
                }
                ui.horizontal(|ui| {
                    let min=self.dimensions.0.min(self.dimensions.1) as f64;
                    ui.add_enabled(p.visible,Slider::new(&mut p.diameter,min*0.1..=min*0.8).text("Camera size").show_value(false));
                    ui.label(RichText::new("Drag camera to position it").small().color(MUTED));
                });
                ui.horizontal(|ui| {
                    if ui.add(Button::new(if state.active {"Stop"} else {"Record"}).fill(ORANGE)).clicked() {action=Some(if state.active {CanvasAction::Stop} else {CanvasAction::Start});}
                    if ui.add_enabled(state.active&&!state.starting,Button::new("Pause / Resume").fill(SURFACE)).clicked() {action=Some(CanvasAction::Pause);}
                    ui.label(RichText::new(crate::model::format_duration(state.elapsed)).monospace());
                });
                ui.label(RichText::new("Only the selected window and camera are saved · Controls stay outside the recording").small().color(MUTED));
            });
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_placement_tracks_output_resolution_without_desktop_coordinates() {
        let mut canvas = CaptureCanvas::default();
        canvas.prepare((1920, 1080), true, true);
        {
            let mut p = canvas.placement.lock().unwrap();
            p.x = 120.0;
            p.y = 240.0;
            p.diameter = 300.0;
        }
        canvas.prepare((1280, 720), true, false);
        let p = *canvas.placement.lock().unwrap();
        assert_eq!((p.x, p.y, p.diameter), (80.0, 160.0, 200.0));
        assert!(p.visible && !p.mirror);
        canvas.prepare((1280, 720), false, false);
        assert!(!canvas.placement.lock().unwrap().visible);
    }
}
