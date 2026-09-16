use super::time::HostClock;
use crate::{
    model::{Device, RecordingClock},
    recording::audio::{AudioChunk, AudioWriter},
};
use anyhow::{Context, Result, ensure};
use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossbeam_channel::{Sender, bounded};
use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

pub fn discover_microphones() -> Result<Vec<Device>> {
    cpal::default_host()
        .input_devices()?
        .map(|device| {
            Ok(Device {
                id: device.id()?.to_string(),
                name: device.description()?.name().to_owned(),
            })
        })
        .collect()
}
pub struct Microphone {
    stream: Option<cpal::Stream>,
    writer: Option<JoinHandle<Result<serde_json::Value>>>,
    error: Arc<Mutex<Option<String>>>,
}
impl Microphone {
    pub fn start(id: &str, path: &Path, clock: Arc<Mutex<RecordingClock>>) -> Result<Self> {
        let device = cpal::default_host()
            .input_devices()?
            .find(|d| d.id().is_ok_and(|i| i.to_string() == id))
            .context("Microphone was disconnected")?;
        let config = device.default_input_config()?;
        let (tx, rx) = bounded::<AudioChunk>(32);
        let error = Arc::new(Mutex::new(None));
        let host_clock = HostClock::new()?;
        let stream=match config.sample_format() {
            SampleFormat::F32=>build::<f32>(&device,config.config(),host_clock,tx,error.clone()),
            SampleFormat::I16=>build::<i16>(&device,config.config(),host_clock,tx,error.clone()),
            SampleFormat::I32=>build::<i32>(&device,config.config(),host_clock,tx,error.clone()),
            SampleFormat::U16=>build::<u16>(&device,config.config(),host_clock,tx,error.clone()),
            format=>anyhow::bail!("Unsupported microphone sample format: {format}"),
        }.context("Cannot open microphone. Enable Microphone access for desktop apps in Windows Privacy settings")?;
        let file = File::create(path.join("microphone.f32"))?;
        let writer_error = error.clone();
        let writer = std::thread::spawn(move || {
            let result = (|| -> Result<_> {
                let mut writer = AudioWriter::new(BufWriter::new(file), 48_000);
                // Infer each buffer's duration from the next hardware timestamp,
                // compensating device drift. Retain only one pending buffer.
                let mut pending: Option<AudioChunk> = None;
                for chunk in rx {
                    if let Some(mut previous) = pending.take() {
                        let nominal = previous.end.duration_since(previous.start);
                        let measured = chunk.start.saturating_duration_since(previous.start);
                        if measured.abs_diff(nominal) < nominal / 20 {
                            previous.end = chunk.start;
                        }
                        writer.write(previous, &clock.lock().unwrap())?;
                    }
                    pending = Some(chunk);
                }
                if let Some(chunk) = pending {
                    writer.write(chunk, &clock.lock().unwrap())?;
                }
                writer.finish()?;
                Ok(
                    serde_json::json!({"delivery":writer.delivery.report(),"inserted_silence_samples":writer.inserted_silence,"overlap_samples":writer.overlap_samples}),
                )
            })();
            if let Err(e) = &result {
                *writer_error.lock().unwrap() = Some(format!("{e:#}"));
            }
            result
        });
        let mut microphone = Self {
            stream: Some(stream),
            writer: Some(writer),
            error,
        };
        if let Err(e) = microphone.stream.as_ref().unwrap().play() {
            let _ = microphone.stop();
            return Err(e.into());
        }
        Ok(microphone)
    }
    pub fn check(&mut self) -> Result<()> {
        if let Some(e) = self.error.lock().unwrap().as_ref() {
            anyhow::bail!("{e}");
        }
        Ok(())
    }
    pub fn sample_rate(&self) -> u32 {
        48_000
    }
    pub fn stop(&mut self) -> Result<serde_json::Value> {
        self.stream.take(); // Stop WASAPI, releasing callback senders before draining.
        let report = if let Some(writer) = self.writer.take() {
            writer
                .join()
                .map_err(|_| anyhow::anyhow!("Audio writer panicked"))??
        } else {
            serde_json::json!({})
        };
        self.check()?;
        Ok(report)
    }
}
impl Drop for Microphone {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    clock: HostClock,
    tx: Sender<AudioChunk>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Send,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate as f64;
    ensure!(channels > 0 && rate > 0.0, "Invalid microphone format");
    let failed = error.clone();
    Ok(device.build_input_stream::<T, _, _>(
        config,
        move |samples, info| {
            let result = (|| -> Result<()> {
                let pts = i64::try_from(info.timestamp().capture.as_nanos() / 100)?;
                let start = clock.instant(pts)?;
                let samples = samples
                    .chunks_exact(channels)
                    .map(|frame| {
                        frame.iter().map(|v| f32::from_sample(*v)).sum::<f32>() / channels as f32
                    })
                    .collect::<Vec<_>>();
                if samples.is_empty() {
                    return Ok(());
                }
                let end = start + Duration::from_secs_f64(samples.len() as f64 / rate);
                tx.try_send(AudioChunk {
                    samples,
                    start,
                    end,
                })
                .context("Microphone queue overflow: recording cannot keep up")?;
                Ok(())
            })();
            if let Err(e) = result {
                *error.lock().unwrap() = Some(format!("{e:#}"));
            }
        },
        move |e| {
            *failed.lock().unwrap() =
                Some(format!("Microphone disconnected or capture failed: {e}"));
        },
        None,
    )?)
}
