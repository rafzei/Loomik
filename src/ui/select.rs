use super::{BLUE, INK, MUTED, SURFACE, WHITE};
use eframe::egui::{self, *};
use egui_phosphor::regular as icons;

/// Shared field and menu styling. egui still owns popup placement, focus,
/// scrolling, click-away/Escape dismissal and keyboard activation.
pub struct Select<'a> {
    id: &'static str,
    text: &'a str,
    width: f32,
    icon: Option<&'static str>,
    accent: bool,
}
impl<'a> Select<'a> {
    pub fn new(id: &'static str, text: &'a str, width: f32) -> Self {
        Self {
            id,
            text,
            width,
            icon: None,
            accent: false,
        }
    }
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn accent(mut self, accent: bool) -> Self {
        self.accent = accent;
        self
    }
    pub fn show<R>(
        self,
        ui: &mut Ui,
        contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<Option<R>> {
        ui.scope(|ui| {
            let width = self.width.min(ui.available_width());
            ui.set_width(width);
            field_style(ui.style_mut(), self.icon.is_some(), self.accent);
            let color = if self.accent { WHITE } else { INK };
            let mut text = text::LayoutJob::default();
            if let Some(icon) = self.icon {
                text.append(
                    icon,
                    0.0,
                    TextFormat {
                        font_id: FontId::proportional(20.0),
                        color,
                        ..Default::default()
                    },
                );
            }
            text.append(
                self.text,
                if self.icon.is_some() { 12.0 } else { 0.0 },
                TextFormat {
                    font_id: FontId::proportional(if self.icon.is_some() { 14.0 } else { 13.0 }),
                    color,
                    ..Default::default()
                },
            );
            let mut result = ComboBox::from_id_salt(self.id)
                .width(width)
                .height(252.0)
                .selected_text(text)
                .truncate()
                .icon(|ui, rect, visuals, open| {
                    let c = rect.center();
                    let dy = if open { -2.0 } else { 2.0 };
                    ui.painter().add(Shape::line(
                        vec![
                            pos2(c.x - 4.0, c.y - dy),
                            pos2(c.x, c.y + dy),
                            pos2(c.x + 4.0, c.y - dy),
                        ],
                        Stroke::new(1.5_f32, visuals.fg_stroke.color),
                    ));
                })
                .popup_style(egui::style::StyleModifier::new(menu_style))
                .show_ui(ui, |ui| {
                    // ComboBox defaults to extending menus to fit labels. Keep
                    // long device/window names inside the floating panel instead.
                    ui.set_width((width - 16.0).max(112.0));
                    ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                    contents(ui)
                });
            if result.response.has_focus() {
                ui.painter().rect_stroke(
                    result.response.rect.expand(2.0),
                    12,
                    Stroke::new(2.0_f32, BLUE),
                    StrokeKind::Outside,
                );
            }
            result.response = result.response.on_hover_text(self.text);
            result
        })
        .inner
    }
}

fn field_style(style: &mut Style, large: bool, accent: bool) {
    style.spacing.button_padding = vec2(12.0, if large { 14.0 } else { 9.0 });
    style.spacing.interact_size.y = if large { 48.0 } else { 36.0 };
    style.spacing.icon_width = 14.0;
    style.spacing.icon_spacing = 10.0;
    style.visuals.override_text_color = None;
    let color = if accent { WHITE } else { MUTED };
    let base = if accent { BLUE } else { SURFACE };
    let hover = if accent {
        Color32::from_rgb(22, 91, 206)
    } else {
        Color32::from_rgb(233, 236, 241)
    };
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(if large { 12 } else { 10 });
        widget.bg_stroke = Stroke::NONE;
        widget.expansion = 0.0;
        widget.fg_stroke = Stroke::new(1.5_f32, color);
        widget.bg_fill = base;
        widget.weak_bg_fill = base;
    }
    style.visuals.widgets.hovered.weak_bg_fill = hover;
    style.visuals.widgets.active.weak_bg_fill = hover;
    style.visuals.widgets.open.weak_bg_fill = hover;
    style.visuals.widgets.open.bg_stroke = Stroke::new(
        1.0_f32,
        if accent {
            Color32::from_rgb(16, 78, 184)
        } else {
            Color32::from_rgb(191, 207, 233)
        },
    );
}

fn menu_style(style: &mut Style) {
    style.visuals.override_text_color = Some(INK);
    style.visuals.window_fill = WHITE;
    style.visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(226, 230, 236));
    style.visuals.menu_corner_radius = CornerRadius::same(13);
    style.visuals.popup_shadow = Shadow {
        offset: [0, 5],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(28),
    };
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.button_padding = vec2(10.0, 9.0);
    style.spacing.interact_size.y = 36.0;
    style.spacing.item_spacing = vec2(6.0, 3.0);
    style.visuals.selection.bg_fill = Color32::from_rgb(233, 241, 254);
    style.visuals.selection.stroke = Stroke::NONE;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(7);
        widget.bg_stroke = Stroke::NONE;
        widget.expansion = 0.0;
        widget.bg_fill = SURFACE;
        widget.weak_bg_fill = SURFACE;
        widget.fg_stroke = Stroke::new(1.0_f32, INK);
    }
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(242, 245, 249);
}

pub fn select_option<T: PartialEq>(ui: &mut Ui, value: &mut T, option: T, text: &str) -> Response {
    let selected = *value == option;
    let mut response = ui.add_sized(
        [ui.available_width(), 36.0],
        Button::selectable(
            selected,
            RichText::new(text)
                .size(13.0)
                .color(if selected { BLUE } else { INK }),
        )
        .right_text(
            RichText::new(if selected { icons::CHECK } else { " " })
                .size(16.0)
                .color(BLUE),
        )
        .truncate(),
    );
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), selected, text)
    });
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            7,
            Stroke::new(1.5_f32, BLUE),
            StrokeKind::Inside,
        );
    }
    if response.clicked() {
        if !selected {
            *value = option;
            response.mark_changed();
        }
        ui.close();
    }
    response.on_hover_text(text)
}

pub fn select_hint(ui: &mut Ui, text: &str) {
    ui.add_space(3.0);
    ui.add(Label::new(RichText::new(text).size(12.0).color(MUTED)).wrap());
    ui.add_space(3.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Harness {
        ctx: Context,
        value: usize,
        trigger: Option<Response>,
        rows: Vec<Response>,
        enabled: bool,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = Context::default();
            super::super::configure(&ctx);
            Self {
                ctx,
                value: 0,
                trigger: None,
                rows: Vec::new(),
                enabled: true,
            }
        }
        fn frame(&mut self, events: Vec<Event>) {
            let ctx = self.ctx.clone();
            let _ = ctx.run(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(340.0,624.0))), events, focused: true, ..Default::default() }, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    super::super::window_drag(ui, ui.max_rect());
                    ui.add_enabled_ui(self.enabled, |ui| {
                        self.rows.clear();
                        self.trigger = Some(Select::new("test", "A very long camera device name that should never widen the floating panel", 288.0).icon(icons::VIDEO_CAMERA).show(ui, |ui| {
                            for (i, label) in ["Camera off", "USB camera with an extremely long name that must stay within the menu", "Built-in camera"].into_iter().enumerate() {
                                self.rows.push(select_option(ui, &mut self.value, i, label));
                            }
                        }).response);
                    });
                });
            });
        }
        fn click(&mut self, pos: Pos2) {
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![]);
        }
        fn key(&mut self, key: Key) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
        }
        fn open(&self) -> bool {
            Popup::is_id_open(&self.ctx, self.trigger.as_ref().unwrap().id.with("popup"))
        }
    }
    #[test]
    fn selection_and_dismissal_work_with_bounded_long_labels() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.frame(vec![]);
        let trigger = h.trigger.as_ref().unwrap().rect;
        assert!(trigger.width() <= 289.0);
        assert!(trigger.height() >= 48.0);
        h.click(trigger.center());
        assert!(h.open());
        assert_eq!(h.rows.len(), 3);
        assert!(
            h.rows
                .iter()
                .all(|r| r.rect.right() <= 340.0 && r.rect.height() >= 36.0)
        );
        h.click(h.rows[1].rect.center());
        assert_eq!(h.value, 1);
        assert!(!h.open());
        h.click(trigger.center());
        h.key(Key::Escape);
        assert!(!h.open());
        h.click(trigger.center());
        h.click(pos2(330.0, 600.0));
        assert!(!h.open());
    }
    #[test]
    fn keyboard_and_disabled_fields_preserve_selection() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.frame(vec![]);
        let trigger = h.trigger.as_ref().unwrap().clone();
        trigger.request_focus();
        h.key(Key::Enter);
        assert!(h.open());
        h.frame(vec![]);
        h.rows[2].request_focus();
        h.key(Key::Enter);
        assert_eq!(h.value, 2);
        assert!(!h.open());
        h.enabled = false;
        h.frame(vec![]);
        h.click(trigger.rect.center());
        assert!(!h.open());
        assert_eq!(h.value, 2);
    }
}
