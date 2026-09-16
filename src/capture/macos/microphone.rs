use crate::{
    model::{Device, RecordingClock},
    recording::audio::{AudioChunk, AudioWriter},
};
use anyhow::{Context, Result};
use crossbeam_channel::{Sender, bounded};
use objc2::{
    AllocAnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
};
use objc2_av_foundation::*;
use objc2_avf_audio::*;
use objc2_core_media::{CMAudioFormatDescriptionGetStreamBasicDescription, CMSampleBuffer, CMTime};
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};
use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    ptr::NonNull,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

const RATE: u32 = 48_000;
pub fn discover_microphones() -> Result<Vec<Device>> {
    #[allow(deprecated)]
    let devices = unsafe {
        AVCaptureDevice::devicesWithMediaType(
            AVMediaTypeAudio.context("Audio capture is unavailable")?,
        )
    };
    Ok(devices
        .iter()
        .map(|d| unsafe {
            Device {
                id: d.uniqueID().to_string(),
                name: d.localizedName().to_string(),
            }
        })
        .collect())
}
struct AudioState {
    session: Retained<AVCaptureSession>,
    chunks: Sender<AudioChunk>,
    error: Arc<Mutex<Option<String>>>,
}
impl AudioState {
    unsafe fn receive(&self, sample: &CMSampleBuffer) -> Result<()> {
        unsafe {
            let format = sample
                .format_description()
                .context("Missing microphone format")?;
            let format = CMAudioFormatDescriptionGetStreamBasicDescription(&format)
                .as_ref()
                .context("Invalid microphone format")?;
            anyhow::ensure!(
                format.mSampleRate == RATE as f64
                    && format.mChannelsPerFrame == 1
                    && format.mBitsPerChannel == 32
                    && format.mFormatID == u32::from_be_bytes(*b"lpcm")
                    && format.mFormatFlags & 3 == 1,
                "Microphone did not provide little-endian float PCM"
            );
            let count = sample.num_samples();
            anyhow::ensure!(
                count > 0 && count <= RATE as isize,
                "Invalid microphone buffer length"
            );
            let clock = self
                .session
                .synchronizationClock()
                .context("Missing microphone clock")?;
            let pts = sample.presentation_time_stamp();
            let end_pts = pts.add(CMTime::new(count as i64, RATE as i32));
            let start =
                super::time::capture_instant(pts, &clock).context("Invalid microphone PTS")?;
            let end = super::time::capture_instant(end_pts, &clock)
                .context("Invalid microphone end PTS")?;
            anyhow::ensure!(
                end > start && end <= Instant::now(),
                "Invalid future microphone timestamp"
            );
            let buffer = sample.data_buffer().context("Missing microphone samples")?;
            let mut samples = vec![0f32; count as usize];
            anyhow::ensure!(
                buffer.data_length() == samples.len() * 4,
                "Unexpected microphone data size"
            );
            let status = buffer.copy_data_bytes(
                0,
                samples.len() * 4,
                NonNull::new(samples.as_mut_ptr().cast()).unwrap(),
            );
            anyhow::ensure!(status == 0, "Cannot copy microphone samples: {status}");
            self.chunks.try_send(AudioChunk { samples, start, end }).context("Audio writer cannot keep up; recording stopped to avoid losing synchronization")?;
        }
        Ok(())
    }
}
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "LoomikAudioDelegate"]
    #[ivars = AudioState]
    struct AudioDelegate;
    unsafe impl NSObjectProtocol for AudioDelegate {}
    unsafe impl AVCaptureAudioDataOutputSampleBufferDelegate for AudioDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn capture_output(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            if let Err(error) = unsafe { self.ivars().receive(sample) } {
                *self.ivars().error.lock().unwrap() = Some(format!("{error:#}"));
            }
        }
    }
);

pub struct Microphone {
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureAudioDataOutput>,
    delegate: Option<Retained<AudioDelegate>>,
    writer: Option<JoinHandle<Result<serde_json::Value>>>,
    error: Arc<Mutex<Option<String>>>,
}
impl Microphone {
    pub fn start(id: &str, folder: &Path, clock: Arc<Mutex<RecordingClock>>) -> Result<Self> {
        unsafe {
            super::request_media_permission(
                AVMediaTypeAudio.context("Audio capture unavailable")?,
                "Microphone",
            )?;
            // Accept old CPAL preference IDs during migration when the native UID
            // is not found. New preferences use the stable native device UID.
            let device = AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(id))
                .or_else(|| {
                    let name = id.split_once(':')?.1;
                    #[allow(deprecated)]
                    AVCaptureDevice::devicesWithMediaType(AVMediaTypeAudio?)
                        .iter()
                        .find(|d| d.localizedName().to_string() == name)
                })
                .context("The selected microphone is disconnected. Choose it again in settings.")?;
            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let session = AVCaptureSession::new();
            let output = AVCaptureAudioDataOutput::new();
            let keys = [
                AVFormatIDKey,
                AVSampleRateKey,
                AVNumberOfChannelsKey,
                AVLinearPCMBitDepthKey,
                AVLinearPCMIsFloatKey,
                AVLinearPCMIsBigEndianKey,
                AVLinearPCMIsNonInterleaved,
            ];
            let keys = keys
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .context("Audio settings unavailable")?;
            let values = [
                NSNumber::new_u32(u32::from_be_bytes(*b"lpcm")),
                NSNumber::new_u32(RATE),
                NSNumber::new_u32(1),
                NSNumber::new_u32(32),
                NSNumber::new_bool(true),
                NSNumber::new_bool(false),
                NSNumber::new_bool(false),
            ];
            let refs: Vec<_> = values.iter().map(|n| &**n).collect();
            let settings = NSDictionary::from_slices(&keys, &refs);
            output.setAudioSettings(Some(settings.cast_unchecked()));
            session.beginConfiguration();
            anyhow::ensure!(
                session.canAddInput(&input) && session.canAddOutput(&output),
                "Microphone unavailable or in use"
            );
            session.addInput(&input);
            session.addOutput(&output);
            session.commitConfiguration();
            let file = BufWriter::new(File::create(folder.join("microphone.f32"))?);
            let (tx, rx) = bounded::<AudioChunk>(32);
            let error = Arc::new(Mutex::new(None));
            let writer_error = error.clone();
            let writer = thread::spawn(move || {
                let result = (|| {
                    let mut writer = AudioWriter::new(file, RATE);
                    for chunk in rx {
                        let timeline = clock.lock().unwrap().clone();
                        writer.write(chunk, &timeline)?;
                    }
                    writer.finish()?;
                    Ok(
                        serde_json::json!({"delivery": writer.delivery.report(), "inserted_silence_samples": writer.inserted_silence, "overlap_samples": writer.overlap_samples}),
                    )
                })();
                if let Err(error) = &result {
                    *writer_error.lock().unwrap() = Some(format!("{error:#}"));
                }
                result
            });
            let delegate: Retained<AudioDelegate> = msg_send![
                super(AudioDelegate::alloc().set_ivars(AudioState {
                    session: session.clone(),
                    chunks: tx,
                    error: error.clone()
                })),
                init
            ];
            let queue = dispatch2::DispatchQueue::new("com.loomik.audio", None);
            output.setSampleBufferDelegate_queue(
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            );
            session.startRunning();
            Ok(Self {
                session,
                output,
                delegate: Some(delegate),
                writer: Some(writer),
                error,
            })
        }
    }
    pub fn sample_rate(&self) -> u32 {
        RATE
    }
    pub fn check(&self) -> Result<()> {
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            anyhow::bail!("Microphone failed: {error}");
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<serde_json::Value> {
        if self.delegate.is_some() {
            unsafe {
                self.session.stopRunning();
                self.output.setSampleBufferDelegate_queue(None, None);
            }
            self.delegate.take();
        }
        let report = self
            .writer
            .take()
            .map(|w| {
                w.join()
                    .map_err(|_| anyhow::anyhow!("Audio writer panicked"))?
            })
            .transpose()?
            .unwrap_or_default();
        self.check()?;
        Ok(report)
    }
}
impl Drop for Microphone {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
