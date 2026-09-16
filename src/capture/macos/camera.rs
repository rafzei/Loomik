use crate::{
    model::Device,
    recording::frame::{LatestFrame, VideoFrame},
};
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, bounded};
use objc2::{
    AllocAnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
};
use objc2_av_foundation::*;
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::*;
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};
use std::{thread, time::Duration};

pub fn discover_cameras() -> Result<Vec<Device>> {
    // Enumeration does not start a session or request access to camera frames.
    #[allow(deprecated)]
    let devices = unsafe {
        AVCaptureDevice::devicesWithMediaType(
            AVMediaTypeVideo.context("Video capture is unavailable")?,
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

struct CameraState {
    frames: LatestFrame,
    session: Retained<AVCaptureSession>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "LoomikCameraDelegate"]
    #[ivars = CameraState]
    struct CameraDelegate;
    unsafe impl NSObjectProtocol for CameraDelegate {}
    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for CameraDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn capture_output(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            // SAFETY: AVFoundation owns this sample through the callback. Copy
            // packed BGRA under a matching lock/unlock; no sample escapes.
            unsafe {
                let Some(clock) = self.ivars().session.synchronizationClock() else {
                    return;
                };
                let Some(captured_at) =
                    super::time::capture_instant(sample.presentation_time_stamp(), &clock)
                else {
                    return;
                };
                let Some(pixel) = sample.image_buffer() else {
                    return;
                };
                let flags = CVPixelBufferLockFlags::ReadOnly;
                if CVPixelBufferLockBaseAddress(&pixel, flags) != 0 {
                    return;
                }
                let base = CVPixelBufferGetBaseAddress(&pixel).cast::<u8>();
                let stride = CVPixelBufferGetBytesPerRow(&pixel);
                let height = CVPixelBufferGetHeight(&pixel);
                if !base.is_null() {
                    let bytes = std::slice::from_raw_parts(base, stride * height);
                    if let Some(frame) = VideoFrame::from_strided_at(
                        CVPixelBufferGetWidth(&pixel) as u32,
                        height as u32,
                        stride,
                        bytes,
                        captured_at,
                    ) {
                        self.ivars().frames.set(frame);
                    }
                }
                CVPixelBufferUnlockBaseAddress(&pixel, flags);
            }
        }
    }
);

pub struct Camera {
    stop: Sender<()>,
    pub events: Receiver<Result<(), String>>,
    pub frames: LatestFrame,
}
impl Camera {
    pub fn start(id: String) -> Self {
        let (stop, rx) = bounded(1);
        let (tx, events) = bounded(2);
        let frames = LatestFrame::default();
        let latest = frames.clone();
        thread::spawn(move || {
            let result = run(id, latest.clone(), rx, &tx);
            latest.clear();
            if let Err(e) = result {
                let _ = tx.try_send(Err(format!("{e:#}")));
            }
        });
        Self {
            stop,
            events,
            frames,
        }
    }
}
impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
    }
}

fn run(
    id: String,
    latest: LatestFrame,
    stop: Receiver<()>,
    events: &Sender<Result<(), String>>,
) -> Result<()> {
    // Every AVFoundation object is created, configured, and stopped on this
    // worker. Only the delegate's mutex-protected latest frame crosses threads.
    unsafe {
        super::request_media_permission(
            AVMediaTypeVideo.context("Video capture is unavailable")?,
            "Camera",
        )?;
        if stop.try_recv().is_ok() {
            return Ok(());
        }
        let device = AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(&id))
            .context("The camera is disconnected")?;
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let session = AVCaptureSession::new();
        let output = AVCaptureVideoDataOutput::new();
        output.setAlwaysDiscardsLateVideoFrames(true);
        let key = NSString::from_str("PixelFormatType");
        let value = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
        let settings = NSDictionary::from_slices(&[&*key], &[&*value]);
        output.setVideoSettings(Some(settings.cast_unchecked()));
        session.beginConfiguration();
        if session.canSetSessionPreset(AVCaptureSessionPreset640x480) {
            session.setSessionPreset(AVCaptureSessionPreset640x480);
        }
        anyhow::ensure!(
            session.canAddInput(&input),
            "This camera is unavailable or in use"
        );
        session.addInput(&input);
        anyhow::ensure!(
            session.canAddOutput(&output),
            "This camera cannot provide live video"
        );
        session.addOutput(&output);
        session.commitConfiguration();
        let delegate: Retained<CameraDelegate> = msg_send![
            super(CameraDelegate::alloc().set_ivars(CameraState {
                frames: latest.clone(),
                session: session.clone()
            })),
            init
        ];
        let queue = dispatch2::DispatchQueue::new("com.loomik.camera", None);
        output.setSampleBufferDelegate_queue(
            Some(ProtocolObject::from_ref(&*delegate)),
            Some(&queue),
        );
        session.startRunning();
        // A fixed supported cadence prevents auto-exposure from silently choosing
        // very long frames. Prefer up to 60 fps without changing capture size.
        let range = device
            .activeFormat()
            .videoSupportedFrameRateRanges()
            .iter()
            .filter(|r| r.minFrameRate() <= 60.0)
            .max_by(|a, b| {
                a.maxFrameRate()
                    .min(60.0)
                    .total_cmp(&b.maxFrameRate().min(60.0))
            });
        if let Some(range) = range {
            let fps = range.maxFrameRate().min(60.0);
            if device.lockForConfiguration().is_ok() {
                let duration = if fps == range.maxFrameRate() {
                    range.minFrameDuration()
                } else {
                    CMTime::new(1, 60)
                };
                device.setActiveVideoMinFrameDuration(duration);
                device.setActiveVideoMaxFrameDuration(duration);
                device.unlockForConfiguration();
            }
        }
        let _ = events.send(Ok(()));
        let started = std::time::Instant::now();
        let mut failure = None;
        while stop.recv_timeout(Duration::from_millis(100)).is_err() {
            if started.elapsed() > Duration::from_secs(12)
                && latest
                    .get()
                    .is_none_or(|f| f.captured_at.elapsed() > Duration::from_secs(5))
            {
                failure = Some(anyhow::anyhow!(
                    "Camera stopped delivering frames. Check the connection or choose another camera."
                ));
                break;
            }
        }
        session.stopRunning();
        output.setSampleBufferDelegate_queue(None, None);
        drop(delegate);
        if let Some(error) = failure {
            return Err(error);
        }
    }
    Ok(())
}
