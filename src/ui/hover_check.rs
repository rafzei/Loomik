//! Explicit UI-only regression fixture. It never posts native mouse events.
use eframe::egui::*;
use std::{path::PathBuf, time::Instant};

pub const POINTS: [(&str, Pos2); 13] = [
    ("outside", pos2(-10.0, -10.0)),
    ("grip", pos2(39.0, 23.0)),
    ("record", pos2(39.0, 60.0)),
    ("timer", pos2(39.0, 96.0)),
    ("pause", pos2(39.0, 128.0)),
    ("restart", pos2(39.0, 176.0)),
    ("trash", pos2(39.0, 222.0)),
    ("settings", pos2(39.0, 268.0)),
    ("collapse", pos2(39.0, 320.0)),
    ("quit", pos2(39.0, 354.0)),
    ("edge", pos2(10.0, 180.0)),
    ("outside-again", pos2(-10.0, -10.0)),
    ("record-again", pos2(39.0, 60.0)),
];

pub struct HoverCheck {
    dir: PathBuf,
    started: Instant,
    samples: Vec<serde_json::Value>,
    step: usize,
}
impl HoverCheck {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            started: Instant::now(),
            samples: vec![],
            step: usize::MAX,
        }
    }
}
impl Plugin for HoverCheck {
    fn debug_name(&self) -> &'static str {
        "toolbar-hover-check"
    }
    fn input_hook(&mut self, input: &mut RawInput) {
        if input.viewport_id != ViewportId::from_hash_of("toolbar") {
            return;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        let step = ((elapsed - 1.0).max(0.0) / 0.5) as usize;
        let (name, point) = POINTS[step.min(POINTS.len() - 1)];
        input.events.clear();
        input.events.push(Event::PointerMoved(point));
        if let Some(viewport) = input.viewports.get(&input.viewport_id)
            && let Some(rect) = viewport.outer_rect
            && (1.0..8.0).contains(&elapsed)
            && self.samples.len() < 2048
        {
            self.samples.push(serde_json::json!({"hover":name,"rect":[rect.min.x,rect.min.y,rect.width(),rect.height()]}));
        }
        if step != self.step {
            self.step = step;
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(
                self.dir.join("hover-geometry.json"),
                serde_json::to_vec_pretty(&self.samples).unwrap(),
            );
        }
    }
}
