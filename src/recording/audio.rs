//! Timestamp-driven audio writer, independent of video encoder backpressure.
use super::metrics::Latency;
use crate::model::RecordingClock;
use anyhow::Result;
use std::{io::Write, time::Instant};

pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub start: Instant,
    pub end: Instant,
}
pub struct AudioWriter<W: Write> {
    output: W,
    rate: u32,
    written: u64,
    bytes: Vec<u8>,
    pub inserted_silence: u64,
    pub overlap_samples: u64,
    pub delivery: Latency,
}
impl<W: Write> AudioWriter<W> {
    pub fn new(output: W, rate: u32) -> Self {
        Self {
            output,
            rate,
            written: 0,
            bytes: Vec::new(),
            inserted_silence: 0,
            overlap_samples: 0,
            delivery: Latency::default(),
        }
    }
    pub fn write(&mut self, chunk: AudioChunk, clock: &RecordingClock) -> Result<()> {
        if chunk.samples.is_empty() || chunk.end <= chunk.start {
            return Ok(());
        }
        self.delivery.add(chunk.end.elapsed());
        let duration = chunk.end.duration_since(chunk.start).as_secs_f64();
        for slice in clock.active_slices(chunk.start, chunk.end) {
            let start = (slice.output_start.as_secs_f64() * self.rate as f64).round() as u64;
            let end = ((slice.output_start + slice.end.duration_since(slice.start)).as_secs_f64()
                * self.rate as f64)
                .round() as u64;
            self.overlap_samples += self
                .written
                .saturating_sub(start)
                .min(end.saturating_sub(start));
            while self.written < start {
                let count = (start - self.written).min(4096) as usize;
                self.output.write_all(&[0; 4096 * 4][..count * 4])?;
                self.written += count as u64;
                self.inserted_silence += count as u64;
            }
            self.bytes.clear();
            // Interpolate onto the host timeline on EVERY buffer. Converting the
            // native start and end PTS compensates device clock drift; corrections
            // do not accumulate over a long recording or across pauses.
            for position in self.written.max(start)..end {
                let seconds = slice.start.duration_since(chunk.start).as_secs_f64()
                    + (position - start) as f64 / self.rate as f64;
                let index = (seconds / duration * chunk.samples.len() as f64)
                    .clamp(0.0, (chunk.samples.len() - 1) as f64);
                let left = index as usize;
                let right = (left + 1).min(chunk.samples.len() - 1);
                let fraction = (index - left as f64) as f32;
                let value =
                    chunk.samples[left] + fraction * (chunk.samples[right] - chunk.samples[left]);
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            self.output.write_all(&self.bytes)?;
            self.written = self.written.max(end);
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        if self.written == 0 {
            self.output.write_all(&0f32.to_le_bytes())?;
        }
        self.output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn delayed_buffer_is_clipped_at_start_pause_resume_and_stop() {
        let base = Instant::now() - Duration::from_secs(10);
        let mut clock = RecordingClock::default();
        clock.start(base + Duration::from_secs(1));
        clock.pause(base + Duration::from_secs(2));
        clock.resume(base + Duration::from_secs(3));
        clock.pause(base + Duration::from_secs(4));
        let mut raw = Vec::new();
        let mut writer = AudioWriter::new(&mut raw, 1000);
        // This callback arrives after stop and straddles every boundary.
        writer
            .write(
                AudioChunk {
                    samples: (0..5000).map(|i| i as f32).collect(),
                    start: base,
                    end: base + Duration::from_secs(5),
                },
                &clock,
            )
            .unwrap();
        writer.finish().unwrap();
        let samples: Vec<_> = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| f32::from_le_bytes(*p))
            .collect();
        assert_eq!(samples.len(), 2000);
        assert_eq!(samples[0], 1000.0);
        assert_eq!(samples[999], 1999.0);
        assert_eq!(samples[1000], 3000.0);
        assert_eq!(samples[1999], 3999.0);
    }
    #[test]
    fn independent_device_clock_drift_does_not_accumulate() {
        let base = Instant::now() - Duration::from_secs(700);
        let mut clock = RecordingClock::default();
        clock.start(base);
        let mut raw = Vec::new();
        let mut writer = AudioWriter::new(&mut raw, 1000);
        // Device seconds are 100 ppm longer than host seconds. Timestamp mapping
        // must preserve a pulse after ten minutes within one output sample.
        for i in 0..600 {
            let start = base + Duration::from_secs_f64(i as f64 * 1.0001);
            let end = base + Duration::from_secs_f64((i + 1) as f64 * 1.0001);
            writer
                .write(
                    AudioChunk {
                        samples: vec![if i == 599 { 1.0 } else { 0.0 }; 1000],
                        start,
                        end,
                    },
                    &clock,
                )
                .unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(raw.len() / 4, 600060);
        let first_pulse = raw
            .as_chunks::<4>()
            .0
            .iter()
            .position(|p| f32::from_le_bytes(*p) > 0.5)
            .unwrap();
        assert_eq!(first_pulse, 599060);
    }
}
