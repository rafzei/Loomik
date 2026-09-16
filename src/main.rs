use loomik::{app::LoomikApp, ui};
fn main() -> eframe::Result {
    // Keep the hook alive on the GUI thread for every root/child viewport.
    // If installation fails, the capture backend reports the failure before
    // publishing frames; file backgrounds can still be used.
    #[cfg(target_os = "windows")]
    let _window_protection = loomik::platform::windows::install().ok();
    let args: Vec<String> = std::env::args().collect();
    let smoke = args
        .iter()
        .position(|s| s == "--ui-smoke")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);
    let verify = args
        .iter()
        .position(|s| s == "--verify-recording")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);
    let reset_settings = args.iter().any(|arg| arg == "--reset-settings");
    let options = eframe::NativeOptions {
        viewport: ui::floating("Loomik", eframe::egui::vec2(340.0, 530.0))
            .with_position(eframe::egui::pos2(110.0, 130.0)),
        renderer: eframe::Renderer::Glow,
        persist_window: smoke.is_none() && verify.is_none(),
        ..Default::default()
    };
    eframe::run_native(
        "Loomik",
        options,
        Box::new(move |cc| Ok(Box::new(LoomikApp::new(cc, smoke, verify, reset_settings)))),
    )
}
