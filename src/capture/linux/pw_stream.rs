use super::{Scaler, time::HostClock};
use crate::recording::frame::{LatestFrame, VideoFrame};
use anyhow::{Context as _, Result, ensure};
use pipewire::{self as pw, spa};
use spa::{
    param::{
        ParamType,
        format::{FormatProperties as F, MediaSubtype, MediaType},
        video::{VideoFormat, VideoInfoRaw},
    },
    pod::{Pod, Value},
    utils::{Direction, Fraction, Rectangle, SpaTypes},
};
use std::{
    cell::RefCell,
    os::fd::OwnedFd,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct State {
    format: VideoInfoRaw,
    scaler: Scaler,
    clock: HostClock,
    last: Option<Instant>,
    connected: bool,
}
fn serialize(value: Value) -> Result<Vec<u8>> {
    Ok(
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .map_err(|e| anyhow::anyhow!("PipeWire parameter encoding: {e:?}"))?
            .0
            .into_inner(),
    )
}
fn raw_object(kind: u32, id: u32, properties: Vec<(u32, Value)>) -> Result<Vec<u8>> {
    serialize(Value::Object(spa::pod::Object {
        type_: kind,
        id,
        properties: properties
            .into_iter()
            .map(|(key, value)| spa::pod::Property {
                key,
                flags: spa::pod::PropertyFlags::empty(),
                value,
            })
            .collect(),
    }))
}

pub fn run(
    fd: OwnedFd,
    node: u32,
    output: Option<(u32, u32)>,
    fps: u32,
    frames: LatestFrame,
    stop: Arc<AtomicBool>,
    first_only: bool,
) -> Result<()> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(fd, None)?;
    let failed = Rc::new(RefCell::new(None::<String>));
    let failure = failed.clone();
    let _core_listener = core
        .add_listener_local()
        .error(move |_, _, _, message| {
            *failure.borrow_mut() = Some(format!("PipeWire connection: {message}"));
        })
        .register();
    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "Loomik window",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video", *pw::keys::MEDIA_CATEGORY => "Capture", *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let (state_failure, format_failure, process_failure) =
        (failed.clone(), failed.clone(), failed.clone());
    let out = frames.clone();
    let _listener = stream.add_local_listener_with_user_data(State {format: VideoInfoRaw::default(), scaler: Scaler::default(), clock: HostClock::new()?, last: None, connected: false})
        .state_changed(move |_,data,_,state| {
            if let pw::stream::StreamState::Error(error) = &state { *state_failure.borrow_mut() = Some(format!("Window sharing ended: {error}")); }
            if matches!(state,pw::stream::StreamState::Streaming) {data.connected=true;}
            if data.connected && matches!(state,pw::stream::StreamState::Unconnected) { *state_failure.borrow_mut()=Some("Window sharing was disconnected. Choose the window again.".into()); }
        })
        .param_changed(move |stream, data, id, param| {
            let result = (|| -> Result<()> {
                if id != ParamType::Format.as_raw() {return Ok(());}
                let Some(param) = param else {return Ok(())};
                data.format.parse(param)?;
                ensure!([VideoFormat::BGRA,VideoFormat::BGRx].contains(&data.format.format()), "Portal must supply BGRA/BGRx pixels");
                // Explicitly request acquisition timestamps and mapped system
                // memory. No unhandled DMA-BUF is interpreted as CPU pixels.
                let header = raw_object(spa::sys::SPA_TYPE_OBJECT_ParamMeta, spa::sys::SPA_PARAM_Meta, vec![
                    (spa::sys::SPA_PARAM_META_type, Value::Id(spa::utils::Id(spa::sys::SPA_META_Header))),
                    (spa::sys::SPA_PARAM_META_size, Value::Int(std::mem::size_of::<spa::sys::spa_meta_header>() as i32)),
                ])?;
                let buffers = raw_object(spa::sys::SPA_TYPE_OBJECT_ParamBuffers, spa::sys::SPA_PARAM_Buffers, vec![
                    (spa::sys::SPA_PARAM_BUFFERS_dataType, Value::Int((1 << spa::sys::SPA_DATA_MemPtr) | (1 << spa::sys::SPA_DATA_MemFd))),
                ])?;
                stream.update_params(&mut [Pod::from_bytes(&header).context("Invalid metadata parameter")?, Pod::from_bytes(&buffers).context("Invalid buffer parameter")?])?;
                Ok(())
            })();
            if let Err(error) = result { *format_failure.borrow_mut() = Some(format!("{error:#}")); }
        })
        .process(move |stream, data| {
            if process_failure.borrow().is_some() {return;}
            let result = (|| -> Result<()> {
                let Some(mut buffer) = stream.dequeue_buffer() else {return Ok(())};
                let header = buffer.find_meta::<spa::buffer::meta::MetaHeader>().context("Portal supplies no acquisition timestamps")?;
                if header.flags().intersects(spa::buffer::meta::MetaHeaderFlags::CORRUPTED | spa::buffer::meta::MetaHeaderFlags::GAP) { return Ok(()); }
                let at = data.clock.instant(header.pts())?;
                if data.last.is_some_and(|last| at <= last || at.saturating_duration_since(last) < Duration::from_secs_f64(0.8 / fps.max(1) as f64)) { return Ok(()); }
                let size = data.format.size();
                ensure!(size.width > 0 && size.height > 0 && size.width <= 16384 && size.height <= 16384, "Invalid portal video dimensions");
                let plane = buffer.datas_mut().first_mut().context("Portal returned no video plane")?;
                if plane.chunk().size() == 0 {return Ok(());}
                ensure!(plane.chunk().stride() > 0, "Portal returned an unsupported negative row stride");
                let (offset, stride, length) = (plane.chunk().offset() as usize, plane.chunk().stride() as usize, plane.chunk().size() as usize);
                let end = offset.checked_add(length).context("Portal buffer overflow")?;
                let pixels = plane.data().context("Portal cannot provide mapped video memory; try another desktop portal backend")?
                    .get(offset..end).context("Truncated portal video buffer")?;
                // Copy before returning the PipeWire buffer to its producer.
                let mut frame = VideoFrame::from_strided_at(size.width, size.height, stride, pixels, at).context("Invalid portal video rows")?;
                for pixel in frame.bgra.as_chunks_mut::<4>().0 {pixel[3] = 255;}
                if let Some((w,h)) = output { frame = data.scaler.resize(frame,w,h)?; }
                out.set(frame);
                data.last = Some(at);
                Ok(())
            })();
            if let Err(error) = result { *process_failure.borrow_mut() = Some(format!("{error:#}")); }
        }).register()?;
    let format = serialize(Value::Object(spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(F::MediaType, Id, MediaType::Video),
        spa::pod::property!(F::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            F::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA
        ),
        spa::pod::property!(
            F::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 1920,
                height: 1080
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 16384,
                height: 16384
            }
        ),
        spa::pod::property!(
            F::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: fps, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
        spa::pod::property!(
            F::VideoMaxFramerate,
            Fraction,
            Fraction { num: fps, denom: 1 }
        )
    )))?;
    stream.connect(
        Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut [Pod::from_bytes(&format).context("Invalid format parameter")?],
    )?;
    let began = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        // Loop and listeners remain on this worker. Cancellation is bounded even
        // when the shared window is static or its portal disappears.
        mainloop
            .loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(20)));
        if let Some(error) = failed.borrow_mut().take() {
            anyhow::bail!("{error}");
        }
        if frames.get().is_some() {
            if first_only {
                break;
            }
        } else {
            ensure!(
                began.elapsed() < Duration::from_secs(10),
                "Window sharing timed out before the first frame"
            );
        }
    }
    Ok(())
}
