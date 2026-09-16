//! XComposite reads an isolated redirected window pixmap. Root-window capture
//! would include Loomik's controls and is intentionally never used.
use super::Scaler;
use crate::{
    model::{Bounds, Source, SourceKind},
    recording::frame::{LatestFrame, VideoFrame},
};
use anyhow::{Context, Result, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        composite::{ConnectionExt as _, Redirect},
        xproto::{AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder, MapState, Window},
    },
    rust_connection::RustConnection,
};

fn atom(conn: &RustConnection, name: &[u8]) -> Result<u32> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}
fn own_window(conn: &RustConnection, window: Window) -> Result<bool> {
    let pid = conn
        .get_property(
            false,
            window,
            atom(conn, b"_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            0,
            1,
        )?
        .reply()?;
    if pid.value32().and_then(|mut v| v.next()) == Some(std::process::id()) {
        return Ok(true);
    }
    let class = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
        .reply()?;
    Ok(String::from_utf8_lossy(&class.value)
        .to_lowercase()
        .contains("loomik"))
}
pub fn discover() -> Result<Vec<Source>> {
    let (conn, screen) =
        x11rb::connect(None).context("No X11 display available; use an image/video background")?;
    conn.composite_query_version(0, 4)?
        .reply()
        .context("XComposite is required for isolated window capture")?;
    let windows = conn
        .get_property(
            false,
            conn.setup().roots[screen].root,
            atom(&conn, b"_NET_CLIENT_LIST")?,
            AtomEnum::WINDOW,
            0,
            65536,
        )?
        .reply()?;
    let mut sources = Vec::new();
    for window in windows.value32().into_iter().flatten() {
        let result = (|| -> Result<Source> {
            ensure!(!own_window(&conn, window)?, "Recorder window");
            let attributes = conn.get_window_attributes(window)?.reply()?;
            ensure!(
                attributes.map_state == MapState::VIEWABLE,
                "Window is minimized"
            );
            let geometry = conn.get_geometry(window)?.reply()?;
            ensure!(
                geometry.width >= 100 && geometry.height >= 60,
                "Window too small"
            );
            let utf8 = conn
                .get_property(
                    false,
                    window,
                    atom(&conn, b"_NET_WM_NAME")?,
                    atom(&conn, b"UTF8_STRING")?,
                    0,
                    4096,
                )?
                .reply()?
                .value;
            let title = if utf8.is_empty() {
                conn.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 4096)?
                    .reply()?
                    .value
            } else {
                utf8
            };
            let title = String::from_utf8_lossy(&title).trim().to_owned();
            ensure!(!title.is_empty(), "Untitled window");
            Ok(Source {
                id: window as u64,
                kind: SourceKind::Window,
                name: title,
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: geometry.width as f64,
                    height: geometry.height as f64,
                },
                pixel_width: geometry.width as u32,
                pixel_height: geometry.height as u32,
            })
        })();
        if let Ok(source) = result {
            sources.push(source);
        }
    }
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sources)
}

pub fn run(
    window: Window,
    width: u32,
    height: u32,
    fps: u32,
    frames: LatestFrame,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let (conn, _) = x11rb::connect(None)?;
    ensure!(
        !own_window(&conn, window)?,
        "Loomik cannot record its own controls"
    );
    conn.composite_query_version(0, 4)?.reply()?;
    // A compositor may already own the redirection; naming its pixmap still
    // works. Without a compositor this connection requests automatic redirection.
    let _ = conn
        .composite_redirect_window(window, Redirect::AUTOMATIC)?
        .check();
    let mut pixmap = None;
    let mut dimensions = (0, 0);
    let mut scaler = Scaler::default();
    let interval = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let attributes = conn
            .get_window_attributes(window)?
            .reply()
            .context("The recorded window was closed")?;
        ensure!(
            attributes.map_state == MapState::VIEWABLE,
            "The recorded window was minimized; restore it and start a new take"
        );
        let geometry = conn.get_geometry(window)?.reply()?;
        let size = (geometry.width, geometry.height);
        ensure!(size.0 > 0 && size.1 > 0, "Empty window");
        if size != dimensions {
            if let Some(old) = pixmap.take() {
                conn.free_pixmap(old)?.check()?;
            }
            let id = conn.generate_id()?;
            conn.composite_name_window_pixmap(window,id)?.check().context("XComposite cannot isolate this window. Choose another window or a media background")?;
            pixmap = Some(id);
            dimensions = size;
        }
        let visual = conn
            .setup()
            .roots
            .iter()
            .flat_map(|r| r.allowed_depths.iter())
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.visual_id == attributes.visual)
            .context("Unknown X11 visual")?;
        ensure!(
            visual.red_mask == 0xff0000
                && visual.green_mask == 0x00ff00
                && visual.blue_mask == 0x0000ff,
            "X11 capture requires an RGB8 display visual"
        );
        let format = conn
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == geometry.depth)
            .context("Unknown X11 pixel format")?;
        ensure!(
            [24, 32].contains(&format.bits_per_pixel)
                && conn.setup().image_byte_order == ImageOrder::LSB_FIRST,
            "Unsupported X11 pixel layout"
        );
        // X11 has no acquisition timestamp here. The synchronous read midpoint
        // is an estimate; diagnostics/validation must not call it hardware PTS.
        let before = Instant::now();
        let image = conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                pixmap.context("No window pixmap")?,
                0,
                0,
                size.0,
                size.1,
                u32::MAX,
            )?
            .reply()?;
        let at = before + before.elapsed() / 2;
        let bytes_per_pixel = format.bits_per_pixel as usize / 8;
        let pad = format.scanline_pad as usize;
        ensure!(pad > 0, "Invalid X11 scanline padding");
        let stride = (size.0 as usize * format.bits_per_pixel as usize).div_ceil(pad) * pad / 8;
        ensure!(
            image.data.len() >= stride * size.1 as usize,
            "Truncated X11 pixmap"
        );
        let mut bgra = Vec::with_capacity(size.0 as usize * size.1 as usize * 4);
        for row in image.data.chunks_exact(stride).take(size.1 as usize) {
            for pixel in row[..size.0 as usize * bytes_per_pixel].chunks_exact(bytes_per_pixel) {
                bgra.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        frames.set(scaler.resize(
            VideoFrame {
                width: size.0 as u32,
                height: size.1 as u32,
                bgra,
                captured_at: at,
            },
            width,
            height,
        )?);
        next = (next + interval).max(Instant::now());
        // Small waits keep stop responsive, including at the lowest frame rate.
        while !stop.load(Ordering::Relaxed) && Instant::now() < next {
            std::thread::sleep(
                next.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
    }
    Ok(()) // Connection drop frees pixmaps and our automatic redirection.
}
