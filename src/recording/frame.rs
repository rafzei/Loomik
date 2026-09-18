use super::metrics::Latency;
use crate::model::{Bounds, CameraPlacement};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub captured_at: Instant,
}

impl VideoFrame {
    pub fn from_strided(width: u32, height: u32, stride: usize, bytes: &[u8]) -> Option<Self> {
        Self::from_strided_at(width, height, stride, bytes, Instant::now())
    }
    pub fn from_strided_at(
        width: u32,
        height: u32,
        stride: usize,
        bytes: &[u8],
        captured_at: Instant,
    ) -> Option<Self> {
        let row = (width as usize).checked_mul(4)?;
        let len = row.checked_mul(height as usize)?;
        if width == 0
            || height == 0
            || stride < row
            || bytes.len() < stride.checked_mul(height as usize)?
        {
            return None;
        }
        let mut bgra = Vec::with_capacity(len);
        for y in 0..height as usize {
            bgra.extend_from_slice(&bytes[y * stride..y * stride + row]);
        }
        Some(Self {
            width,
            height,
            bgra,
            captured_at,
        })
    }
}

/// Latest frame for immediate preview, plus a short bounded acquisition history
/// for recording. Never substitute a future frame into a past output slot.
#[derive(Clone, Default)]
pub struct LatestFrame(Arc<Mutex<FrameStore>>);
#[derive(Default)]
struct FrameStore {
    frames: VecDeque<Arc<VideoFrame>>,
    bytes: usize,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    delivery: Latency,
}
impl LatestFrame {
    pub fn set(&self, frame: VideoFrame) {
        self.set_shared(Arc::new(frame));
    }
    pub fn set_shared(&self, frame: Arc<VideoFrame>) {
        let mut store = self.0.lock().unwrap();
        // Native outputs are serial. Reject out-of-order and invalid future PTS.
        if frame.captured_at > Instant::now()
            || store
                .frames
                .back()
                .is_some_and(|last| last.captured_at >= frame.captured_at)
        {
            return;
        }
        store.delivery.add(frame.captured_at.elapsed());
        store.bytes += frame.bgra.len();
        store.frames.push_back(frame);
        while store.frames.len() > 2
            && (store.frames.len() > 32
                || store.bytes > 128 * 1024 * 1024
                || store
                    .frames
                    .back()
                    .unwrap()
                    .captured_at
                    .duration_since(store.frames.front().unwrap().captured_at)
                    > Duration::from_millis(300))
        {
            store.bytes -= store.frames.pop_front().unwrap().bgra.len();
        }
        let wake = store.wake.clone();
        drop(store);
        if let Some(wake) = wake {
            wake();
        }
    }
    pub fn set_waker(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.0.lock().unwrap().wake = Some(wake);
    }
    pub fn get(&self) -> Option<Arc<VideoFrame>> {
        self.0.lock().unwrap().frames.back().cloned()
    }
    pub fn at_or_before(&self, target: Instant) -> Option<Arc<VideoFrame>> {
        self.0
            .lock()
            .unwrap()
            .frames
            .iter()
            .rev()
            .find(|f| f.captured_at <= target)
            .cloned()
    }
    pub fn delivery_report(&self) -> super::metrics::LatencyReport {
        self.0.lock().unwrap().delivery.report()
    }
    pub fn clear(&self) {
        let mut s = self.0.lock().unwrap();
        s.frames.clear();
        s.bytes = 0;
    }
}

/// Full camera frame, preserving its rectangular field of view. Cache repeated
/// frames so exporting at a higher frame rate does not repeat resizing work.
#[derive(Default)]
pub struct CameraFrameRenderer {
    cached: Option<VideoFrame>,
    mirror: bool,
}
impl CameraFrameRenderer {
    pub fn render(
        &mut self,
        camera: &VideoFrame,
        width: u32,
        height: u32,
        mirror: bool,
    ) -> &VideoFrame {
        if self.cached.as_ref().is_none_or(|frame| {
            frame.captured_at != camera.captured_at
                || frame.width != width
                || frame.height != height
                || self.mirror != mirror
        }) {
            // Resizing treats all four BGRA channels equally; no color conversion.
            let image = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                camera.width,
                camera.height,
                camera.bgra.as_slice(),
            )
            .expect("Camera frames contain packed BGRA pixels");
            let mut image = if (camera.width, camera.height) == (width, height) {
                image::RgbaImage::from_raw(width, height, camera.bgra.clone()).unwrap()
            } else {
                image::imageops::resize(
                    &image,
                    width,
                    height,
                    image::imageops::FilterType::Triangle,
                )
            };
            if mirror {
                image::imageops::flip_horizontal_in_place(&mut image);
            }
            self.cached = Some(VideoFrame {
                width,
                height,
                bgra: image.into_raw(),
                captured_at: camera.captured_at,
            });
            self.mirror = mirror;
        }
        self.cached.as_ref().unwrap()
    }
}

/// Cache the circular crop/mirror/edge geometry. Rebuild only when dimensions or
/// placement change, not for every camera frame; reuse the screen scratch buffer.
#[derive(Default)]
pub struct CameraCompositor {
    key: Option<[u64; 12]>,
    pixels: Vec<(usize, usize, u16)>,
}
impl CameraCompositor {
    pub fn apply(
        &mut self,
        screen: &mut VideoFrame,
        camera: &VideoFrame,
        placement: CameraPlacement,
        source: Bounds,
    ) {
        if !placement.visible
            || placement.diameter <= 0.0
            || source.width <= 0.0
            || source.height <= 0.0
        {
            return;
        }
        let key = [
            screen.width as u64,
            screen.height as u64,
            camera.width as u64,
            camera.height as u64,
            placement.x.to_bits(),
            placement.y.to_bits(),
            placement.diameter.to_bits(),
            placement.mirror as u64,
            source.x.to_bits(),
            source.y.to_bits(),
            source.width.to_bits(),
            source.height.to_bits(),
        ];
        if self.key != Some(key) {
            self.key = Some(key);
            self.pixels.clear();
            let sx = screen.width as f64 / source.width;
            let sy = screen.height as f64 / source.height;
            let left = (placement.x - source.x) * sx;
            let top = (placement.y - source.y) * sy;
            let dw = placement.diameter * sx;
            let dh = placement.diameter * sy;
            let side = camera.width.min(camera.height) as f64;
            let crop_x = (camera.width as f64 - side) * 0.5;
            let crop_y = (camera.height as f64 - side) * 0.5;
            for y in (top.floor().max(0.0) as u32)
                ..((top + dh).ceil().clamp(0.0, screen.height as f64) as u32)
            {
                for x in (left.floor().max(0.0) as u32)
                    ..((left + dw).ceil().clamp(0.0, screen.width as f64) as u32)
                {
                    let u = (x as f64 + 0.5 - left) / dw;
                    let v = (y as f64 + 0.5 - top) / dh;
                    let distance = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
                    let alpha = (((0.5 - distance) * dw.min(dh) + 0.5).clamp(0.0, 1.0) * 256.0)
                        .round() as u16;
                    if alpha == 0 {
                        continue;
                    }
                    let u = if placement.mirror { 1.0 - u } else { u };
                    let cx = (crop_x + u * side).clamp(0.0, camera.width.saturating_sub(1) as f64)
                        as usize;
                    let cy = (crop_y + v * side).clamp(0.0, camera.height.saturating_sub(1) as f64)
                        as usize;
                    self.pixels.push((
                        (y as usize * screen.width as usize + x as usize) * 4,
                        (cy * camera.width as usize + cx) * 4,
                        alpha,
                    ));
                }
            }
        }
        for &(dst, src, alpha) in &self.pixels {
            if alpha == 256 {
                screen.bgra[dst..dst + 4].copy_from_slice(&camera.bgra[src..src + 4]);
            } else {
                for c in 0..3 {
                    screen.bgra[dst + c] = ((camera.bgra[src + c] as u16 * alpha
                        + screen.bgra[dst + c] as u16 * (256 - alpha)
                        + 128)
                        >> 8) as u8;
                }
            }
            screen.bgra[dst + 3] = 255;
        }
    }
}

/// Center-crop the camera to a circle in global desktop coordinates. The same
/// geometry is used by the native preview. Anti-alias only the circular edge.
pub fn composite_camera(
    screen: &mut VideoFrame,
    camera: &VideoFrame,
    placement: CameraPlacement,
    source: Bounds,
) {
    CameraCompositor::default().apply(screen, camera, placement, source);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn solid(w: u32, h: u32, pixel: [u8; 4]) -> VideoFrame {
        VideoFrame {
            width: w,
            height: h,
            bgra: pixel.repeat((w * h) as usize),
            captured_at: Instant::now(),
        }
    }
    #[test]
    fn removes_native_stride_padding() {
        let f =
            VideoFrame::from_strided(1, 2, 8, &[1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0])
                .unwrap();
        assert_eq!(f.bgra, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(VideoFrame::from_strided(2, 2, 8, &[0; 8]).is_none());
    }
    #[test]
    fn circle_has_no_square_background_and_respects_negative_monitor_origin() {
        let mut s = solid(100, 100, [0, 0, 0, 255]);
        let c = solid(20, 10, [0, 0, 255, 255]);
        composite_camera(
            &mut s,
            &c,
            CameraPlacement {
                x: -90.0,
                y: 10.0,
                diameter: 40.0,
                visible: true,
                mirror: false,
            },
            Bounds {
                x: -100.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        assert_eq!(
            &s.bgra[(30 * 100 + 30) * 4..(30 * 100 + 30) * 4 + 4],
            &[0, 0, 255, 255]
        );
        assert_eq!(
            &s.bgra[(10 * 100 + 10) * 4..(10 * 100 + 10) * 4 + 4],
            &[0, 0, 0, 255]
        );
        assert_eq!(
            &s.bgra[(80 * 100 + 80) * 4..(80 * 100 + 80) * 4 + 4],
            &[0, 0, 0, 255]
        );
    }
    #[test]
    fn mirroring_and_offscreen_clipping_work() {
        let mut cam = solid(10, 10, [0, 0, 255, 255]);
        for y in 0..10 {
            for x in 5..10 {
                cam.bgra[(y * 10 + x) * 4..(y * 10 + x) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let source = Bounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let mut s = solid(10, 10, [0, 0, 0, 255]);
        composite_camera(
            &mut s,
            &cam,
            CameraPlacement {
                x: 0.0,
                y: 0.0,
                diameter: 10.0,
                mirror: true,
                visible: true,
            },
            source,
        );
        assert_eq!(
            &s.bgra[(5 * 10 + 2) * 4..(5 * 10 + 2) * 4 + 4],
            &[255, 0, 0, 255]
        );
        composite_camera(
            &mut s,
            &cam,
            CameraPlacement {
                x: -5.0,
                y: -5.0,
                diameter: 10.0,
                mirror: false,
                visible: true,
            },
            source,
        );
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    #[test]
    fn delayed_encoder_never_pulls_a_future_frame_into_a_past_slot() {
        let base = Instant::now() - Duration::from_secs(2);
        let frames = LatestFrame::default();
        for i in 0..60 {
            let time = base + Duration::from_millis(i * 16);
            frames.set(VideoFrame::from_strided_at(1, 1, 4, &[i as u8, 0, 0, 255], time).unwrap());
        }
        assert!(
            frames
                .at_or_before(base + Duration::from_millis(100))
                .is_none()
        );
        let target = base + Duration::from_millis(850);
        let frame = frames.at_or_before(target).unwrap();
        assert_eq!(frame.bgra[0], 53);
        assert!(frame.captured_at <= target);
        assert!(frames.0.lock().unwrap().frames.len() <= 20);
    }
}
