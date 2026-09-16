#![cfg(target_os = "linux")]
//! Xvfb exercises the real XComposite adapter without cameras or Docker.
use loomik::{
    capture::{self, ScreenCapture},
    model::{Bounds, Source, SourceKind},
    recording::frame::LatestFrame,
};
use std::time::{Duration, Instant};
use x11rb::{
    COPY_DEPTH_FROM_PARENT,
    connection::Connection,
    protocol::xproto::{
        AtomEnum, ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux, PropMode,
        Rectangle, WindowClass,
    },
    wrapper::ConnectionExt as _,
};

#[test]
#[ignore = "requires X11 with Composite; run under xvfb-run"]
fn x11_window_capture_excludes_occluding_controls_and_reports_closure() {
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = &conn.setup().roots[screen];
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let source_id = conn.generate_id().unwrap();
    let controls = conn.generate_id().unwrap();
    for (id, color) in [(source_id, 0xff0000), (controls, 0xff00ff)] {
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            id,
            root.root,
            20,
            20,
            128,
            96,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().background_pixel(color),
        )
        .unwrap()
        .check()
        .unwrap();
        conn.change_property8(
            PropMode::REPLACE,
            id,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"Loomik integration fixture",
        )
        .unwrap();
        conn.map_window(id).unwrap().check().unwrap();
    }
    conn.change_property32(
        PropMode::REPLACE,
        controls,
        atom(b"_NET_WM_PID"),
        AtomEnum::CARDINAL,
        &[std::process::id()],
    )
    .unwrap();
    conn.change_property32(
        PropMode::REPLACE,
        root.root,
        atom(b"_NET_CLIENT_LIST"),
        AtomEnum::WINDOW,
        &[source_id, controls],
    )
    .unwrap();
    conn.flush().unwrap();
    let sources = capture::discover_sources().unwrap();
    assert!(sources.iter().any(|s| s.id == source_id as u64));
    assert!(!sources.iter().any(|s| s.id == controls as u64));
    let source = Source {
        id: source_id as u64,
        kind: SourceKind::Window,
        name: "fixture".into(),
        bounds: Bounds {
            x: 0.0,
            y: 0.0,
            width: 128.0,
            height: 96.0,
        },
        pixel_width: 128,
        pixel_height: 96,
    };
    let frames = LatestFrame::default();
    let recording = ScreenCapture::start(&source, 128, 96, 30, false, frames.clone()).unwrap();
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, source_id, &CreateGCAux::new().foreground(0xff0000))
        .unwrap();
    for moved in [false, true] {
        if moved {
            conn.configure_window(source_id, &ConfigureWindowAux::new().x(24).y(24))
                .unwrap();
        }
        let start = Instant::now();
        loop {
            recording.check().unwrap();
            // Redraw after automatic redirection. Magenta controls remain above
            // the target; root-window capture would return their pixels.
            conn.poly_fill_rectangle(
                source_id,
                gc,
                &[Rectangle {
                    x: 0,
                    y: 0,
                    width: 128,
                    height: 96,
                }],
            )
            .unwrap();
            conn.flush().unwrap();
            if let Some(frame) = frames.get()
                && frame.captured_at > start
                && frame
                    .bgra
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p == &[0, 0, 255, 255])
            {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "Isolated window pixels did not arrive"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    conn.destroy_window(source_id).unwrap().check().unwrap();
    let start = Instant::now();
    while recording.check().is_ok() {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Closed-window capture did not stop"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
