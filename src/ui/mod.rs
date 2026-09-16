mod select;
pub use select::{Select, select_hint, select_option};
mod window_drag;
pub use window_drag::window_drag;
#[cfg(any(target_os = "linux", test))]
pub mod capture_canvas;
pub mod hover_check;
pub mod media_canvas;

use eframe::egui::{self, *};

pub const INK: Color32 = Color32::from_rgb(35, 37, 41);
pub const MUTED: Color32 = Color32::from_rgb(113, 118, 128);
pub const SURFACE: Color32 = Color32::from_rgb(241, 242, 244);
pub const ORANGE: Color32 = Color32::from_rgb(239, 75, 43);
pub const BLUE: Color32 = Color32::from_rgb(25, 102, 226);
pub const DARK: Color32 = Color32::from_rgb(38, 39, 43);
pub const WHITE: Color32 = Color32::WHITE;

pub fn configure(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts
            .font_data
            .insert("system".into(), FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "system".into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
    let mut style = Style {
        visuals: Visuals::light(),
        ..Default::default()
    };
    style.visuals.panel_fill = WHITE;
    style.visuals.window_fill = WHITE;
    style.visuals.window_corner_radius = CornerRadius::same(16);
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = BLUE;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, WHITE);
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(9);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(9);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(9);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(229, 233, 240);
    style.spacing.item_spacing = vec2(8.0, 10.0);
    style.spacing.button_padding = vec2(10.0, 7.0);
    style.interaction.selectable_labels = false;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    ctx.set_style(style);
}

pub fn floating(title: &str, size: Vec2) -> ViewportBuilder {
    let builder = ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        .with_has_shadow(false);
    #[cfg(target_os = "linux")]
    let builder = builder.with_app_id("io.github.rafzei.Loomik");
    builder
}

pub fn panel_frame(dark: bool) -> Frame {
    Frame::new()
        .fill(if dark { DARK } else { WHITE })
        .corner_radius(if dark { 28 } else { 22 })
        .stroke(Stroke::new(
            1.0_f32,
            if dark {
                Color32::from_rgb(67, 68, 73)
            } else {
                Color32::from_rgb(227, 229, 233)
            },
        ))
        .inner_margin(if dark {
            Margin::symmetric(10, 10)
        } else {
            Margin::same(20)
        })
        .outer_margin(Margin::same(5))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(22),
        })
}

pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, size: f32, dark: bool) -> Response {
    let color = if dark {
        Color32::from_rgb(208, 211, 219)
    } else {
        MUTED
    };
    ui.add_sized(
        [size, size],
        Button::new(RichText::new(icon).size(22.0).color(color)).frame(false),
    )
    .on_hover_text(tip)
}

pub fn primary(ui: &mut Ui, label: &str, width: f32) -> Response {
    ui.add_sized(
        [width, 48.0],
        Button::new(RichText::new(label).size(15.0).strong().color(WHITE))
            .fill(ORANGE)
            .corner_radius(12),
    )
}

pub fn camera_mesh(texture: TextureId, rect: Rect, mirror: bool, aspect: f32) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let center = rect.center();
    let radius = rect.width() * 0.5;
    let crop_x = if aspect > 1.0 { 1.0 / aspect } else { 1.0 };
    let crop_y = if aspect < 1.0 { aspect } else { 1.0 };
    mesh.vertices.push(egui::epaint::Vertex {
        pos: center,
        uv: pos2(0.5, 0.5),
        color: WHITE,
    });
    for i in 0..=96 {
        let angle = i as f32 / 96.0 * std::f32::consts::TAU;
        let p = vec2(angle.cos(), angle.sin());
        let u = 0.5 + p.x * 0.5 * crop_x * (if mirror { -1.0 } else { 1.0 });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + p * radius,
            uv: pos2(u, 0.5 + p.y * 0.5 * crop_y),
            color: WHITE,
        });
        if i > 0 {
            mesh.indices.extend_from_slice(&[0, i, i + 1]);
        }
    }
    mesh
}
