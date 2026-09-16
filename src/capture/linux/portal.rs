use crate::{
    model::{Bounds, Source, SourceKind},
    recording::frame::LatestFrame,
};
use anyhow::{Context, Result, ensure};
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SourceType},
};
use std::{
    os::fd::OwnedFd,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

pub struct Selection {
    proxy: Screencast<'static>,
    session: Session<'static, Screencast<'static>>,
    pub node: u32,
    pub source: Source,
}
impl Selection {
    pub fn remote(&self) -> Result<OwnedFd> {
        // Every consumer needs a fresh PipeWire connection, not a dup of an
        // already connected socket with a second protocol handshake.
        Ok(async_io::block_on(
            self.proxy.open_pipe_wire_remote(&self.session),
        )?)
    }
}
impl Drop for Selection {
    fn drop(&mut self) {
        let _ = async_io::block_on(self.session.close());
    }
}
static SELECTED: Mutex<Option<Arc<Selection>>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}
pub fn request() -> bool {
    let result = async_io::block_on(select());
    match result {
        Ok(selection) => {
            let old = SELECTED.lock().unwrap().replace(Arc::new(selection));
            drop(old);
            *ERROR.lock().unwrap() = None;
            true
        }
        Err(error) => {
            *ERROR.lock().unwrap() = Some(format!("Window selection: {error:#}"));
            false
        }
    }
}
pub fn discover() -> Result<Vec<Source>> {
    if let Some(error) = ERROR.lock().unwrap().take() {
        anyhow::bail!("{error}");
    }
    Ok(SELECTED
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.source.clone())
        .collect())
}
pub fn selected(id: u64) -> Result<Arc<Selection>> {
    SELECTED
        .lock()
        .unwrap()
        .as_ref()
        .filter(|s| s.source.id == id)
        .cloned()
        .context("Choose a window again in the screen sharing dialog")
}
async fn select() -> Result<Selection> {
    let proxy = Screencast::new()
        .await
        .context("Install and start xdg-desktop-portal and your desktop's portal backend")?;
    ensure!(
        proxy
            .available_source_types()
            .await?
            .contains(SourceType::Window),
        "This portal cannot share an isolated window. Use an image/video background or a desktop with window sharing support"
    );
    let session = proxy.create_session().await?;
    // Construct the guard before fallible requests so cancellation closes it.
    let mut selected = Selection {
        proxy,
        session,
        node: 0,
        source: Source {
            id: 0,
            kind: SourceKind::Window,
            name: "Shared window · Wayland".into(),
            bounds: Bounds::default(),
            pixel_width: 0,
            pixel_height: 0,
        },
    };
    selected
        .proxy
        .select_sources(
            &selected.session,
            CursorMode::Hidden,
            SourceType::Window.into(),
            false,
            None,
            PersistMode::DoNot,
        )
        .await?
        .response()?;
    let response = selected
        .proxy
        .start(&selected.session, None)
        .await?
        .response()?;
    let stream = response.streams().first().context("No window selected")?;
    ensure!(
        stream
            .source_type()
            .is_none_or(|kind| kind == SourceType::Window),
        "Portal returned a display instead of a window"
    );
    selected.node = stream.pipe_wire_node_id();
    // Probe real pixel dimensions: portal size is in compositor coordinates and
    // may be missing or differ on HiDPI displays. The probe is short and bounded.
    let fd = selected
        .proxy
        .open_pipe_wire_remote(&selected.session)
        .await?;
    let frames = LatestFrame::default();
    super::pw_stream::run(
        fd,
        selected.node,
        None,
        30,
        frames.clone(),
        Arc::new(AtomicBool::new(false)),
        true,
    )?;
    let frame = frames.get().context("Portal supplied no video frame")?;
    selected.source.id = selected.node as u64;
    selected.source.pixel_width = frame.width;
    selected.source.pixel_height = frame.height;
    selected.source.bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: frame.width as f64,
        height: frame.height as f64,
    };
    Ok(selected)
}
