use eframe::egui::*;
use std::time::Duration;

#[derive(Clone, Copy)]
struct DragAnchor {
    pointer: Pos2,
    window: Pos2,
}

impl DragAnchor {
    fn position(self, pointer: Pos2) -> Pos2 {
        self.window + (pointer - self.pointer)
    }
}

/// Register before the contents so buttons, selects and sliders win hit testing.
/// A click-only background deliberately cannot take a drag from a child button.
pub fn window_drag(ui: &mut Ui, rect: Rect) -> Response {
    let response = ui
        .interact(rect, ui.id().with("window-drag"), Sense::click())
        .on_hover_cursor(CursorIcon::Grab);
    move_window(&response, || desktop_pointer(ui.ctx()));
    response
}

fn move_window(response: &Response, pointer: impl Fn() -> Option<Pos2>) {
    let ctx = &response.ctx;
    let id = Id::new((ctx.viewport_id(), "window-drag-anchor"));
    let (pressed, down, focused, origin) = ctx.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.focused,
            i.viewport().outer_rect.map(|r| r.min),
        )
    });
    if !down || !focused {
        ctx.data_mut(|d| d.remove::<DragAnchor>(id));
        return;
    }
    if pressed && response.is_pointer_button_down_on() {
        if let (Some(pointer), Some(window)) = (pointer(), origin) {
            ctx.data_mut(|d| d.insert_temp(id, DragAnchor { pointer, window }));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }
    if let Some(anchor) = ctx.data(|d| d.get_temp::<DragAnchor>(id)) {
        if let Some(pointer) = pointer() {
            let position = anchor.position(pointer);
            if origin.is_none_or(|origin| origin.distance(position) >= 0.5) {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
            }
        }
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        ctx.request_repaint_after(Duration::from_millis(8));
    }
}

#[cfg(target_os = "macos")]
fn desktop_pointer(ctx: &Context) -> Option<Pos2> {
    use core_graphics::{
        event::CGEvent,
        event_source::{CGEventSource, CGEventSourceStateID},
    };
    // Read the global cursor, never post events. Window-local motion changes
    // underneath the cursor as the window moves and would introduce feedback.
    // Moving directly also avoids AppKit's blocking native window-drag loop:
    // camera preview and countdown continue repainting throughout the gesture.
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let point = CGEvent::new(source).ok()?.location();
    let scale = ctx
        .input(|i| i.viewport().native_pixels_per_point)
        .unwrap_or(1.0)
        / ctx.pixels_per_point();
    Some(pos2(point.x as f32 * scale, point.y as f32 * scale))
}

#[cfg(target_os = "windows")]
fn desktop_pointer(ctx: &Context) -> Option<Pos2> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetPhysicalCursorPos};
    let mut point = POINT::default();
    unsafe { GetPhysicalCursorPos(&mut point) }.ok()?;
    Some(pos2(point.x as f32, point.y as f32) / ctx.pixels_per_point())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn desktop_pointer(_ctx: &Context) -> Option<Pos2> {
    None // Other platforms use the window manager's native drag operation.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_anchor_preserves_offset_and_negative_monitor_positions() {
        let anchor = DragAnchor {
            pointer: pos2(-500.0, 120.0),
            window: pos2(-540.0, 100.0),
        };
        assert_eq!(anchor.position(pos2(-300.0, 80.0)), pos2(-340.0, 60.0));
        assert_eq!(anchor.position(pos2(-500.0, 120.0)), pos2(-540.0, 100.0));
    }

    #[test]
    fn backgrounds_own_presses_but_controls_keep_theirs() {
        let ctx = Context::default();
        let mut button = Rect::NOTHING;
        let mut background_pressed = false;
        let mut clicked = false;
        let mut frame = |events: Vec<Event>| {
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(340.0, 460.0))),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    CentralPanel::default().show(ctx, |ui| {
                        let background = window_drag(ui, ui.max_rect());
                        background_pressed = background.is_pointer_button_down_on();
                        let response = ui.button("Recording control");
                        button = response.rect;
                        clicked = response.clicked();
                    });
                },
            );
            (background_pressed, clicked, button)
        };
        frame(vec![]);
        let (_, _, button) = frame(vec![]);
        let pointer = |pos, pressed| {
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]
        };
        assert!(!frame(pointer(button.center(), true)).0);
        assert!(frame(pointer(button.center(), false)).1);
        assert!(frame(pointer(pos2(200.0, 300.0), true)).0);
        assert!(!frame(pointer(pos2(200.0, 300.0), false)).0);
    }

    #[test]
    fn dragging_moves_the_viewport_without_feedback_and_stops_on_release() {
        let ctx = Context::default();
        let mut origin = pos2(100.0, 100.0);
        let mut frame = |events, global_pointer, focused| {
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(340.0, 460.0))),
                focused,
                events,
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&ViewportId::ROOT)
                .unwrap()
                .outer_rect = Some(Rect::from_min_size(origin, vec2(340.0, 460.0)));
            let output = ctx.run(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let response = ui.interact(ui.max_rect(), Id::new("drag-test"), Sense::click());
                    move_window(&response, || Some(global_pointer));
                });
            });
            let moved = output.viewport_output[&ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|cmd| {
                    if let ViewportCommand::OuterPosition(pos) = cmd {
                        Some(*pos)
                    } else {
                        None
                    }
                });
            if let Some(pos) = moved {
                origin = pos;
            }
            moved
        };
        let press = |pressed| Event::PointerButton {
            pos: pos2(40.0, 30.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(vec![], pos2(140.0, 130.0), true);
        frame(vec![], pos2(140.0, 130.0), true);
        assert_eq!(
            frame(
                vec![Event::PointerMoved(pos2(40.0, 30.0)), press(true)],
                pos2(140.0, 130.0),
                true
            ),
            None
        );
        assert_eq!(
            frame(
                vec![Event::PointerMoved(pos2(80.0, 50.0))],
                pos2(180.0, 150.0),
                true
            ),
            Some(pos2(140.0, 120.0))
        );
        // A moved window changes local coordinates; the global pointer is fixed.
        assert_eq!(
            frame(
                vec![Event::PointerMoved(pos2(40.0, 30.0))],
                pos2(180.0, 150.0),
                true
            ),
            None
        );
        assert_eq!(
            frame(vec![], pos2(600.0, 420.0), true),
            Some(pos2(560.0, 390.0))
        );
        assert_eq!(frame(vec![press(false)], pos2(700.0, 520.0), true), None);
        assert_eq!(frame(vec![], pos2(800.0, 620.0), true), None);
        frame(vec![press(true)], pos2(600.0, 420.0), true);
        assert_eq!(frame(vec![], pos2(700.0, 520.0), false), None);
    }
}
