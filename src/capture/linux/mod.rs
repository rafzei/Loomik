//! Linux captures isolated windows, never the desktop containing recorder controls.
mod camera;
mod microphone;
mod portal;
mod pw_stream;
mod screen;
mod time;
mod x11;
pub use camera::{Camera, discover_cameras};
pub use microphone::{Microphone, discover_microphones};
pub use screen::{ScreenCapture, discover_sources, request_screen_permission, screen_permission};

use crate::recording::frame::VideoFrame;
use anyhow::Result;

#[derive(Default)]
struct Scaler(fast_image_resize::Resizer);
impl Scaler {
    fn resize(&mut self, frame: VideoFrame, width: u32, height: u32) -> Result<VideoFrame> {
        if (frame.width, frame.height) == (width, height) {
            return Ok(frame);
        }
        use fast_image_resize::{
            FilterType, PixelType, ResizeAlg, ResizeOptions,
            images::{Image, ImageRef},
        };
        let input = ImageRef::new(frame.width, frame.height, &frame.bgra, PixelType::U8x4)?;
        let mut output = Image::new(width, height, PixelType::U8x4);
        self.0.resize(
            &input,
            &mut output,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                .use_alpha(false),
        )?;
        Ok(VideoFrame {
            width,
            height,
            bgra: output.into_vec(),
            captured_at: frame.captured_at,
        })
    }
}
