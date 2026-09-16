use anyhow::{Context, Result, ensure};
use std::time::{Duration, Instant};

/// PipeWire header PTS and V4L2 MONOTONIC metadata share CLOCK_MONOTONIC.
#[derive(Clone, Copy)]
pub struct HostClock {
    host: Instant,
    nanos: i64,
}
impl HostClock {
    pub fn new() -> Result<Self> {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let before = Instant::now();
        // SAFETY: clock_gettime writes exactly one initialized timespec.
        ensure!(
            unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut value) } == 0,
            "Cannot read the Linux monotonic clock"
        );
        let after = Instant::now();
        Ok(Self {
            host: before + (after - before) / 2,
            nanos: value
                .tv_sec
                .checked_mul(1_000_000_000)
                .and_then(|s| s.checked_add(value.tv_nsec))
                .context("Host clock overflow")?,
        })
    }
    pub fn instant(&self, nanos: i64) -> Result<Instant> {
        ensure!(nanos > 0, "Device supplied no acquisition timestamp");
        let delta = Duration::from_nanos(nanos.abs_diff(self.nanos));
        let instant = if nanos >= self.nanos {
            self.host.checked_add(delta)
        } else {
            self.host.checked_sub(delta)
        };
        let instant = instant.context("Capture timestamp is outside the host clock")?;
        ensure!(
            instant <= Instant::now() + Duration::from_millis(50),
            "Device clock is not CLOCK_MONOTONIC"
        );
        Ok(instant)
    }
}

/// ALSA stream times are relative to stream creation, not boot time.
#[derive(Clone, Copy)]
pub struct AudioClock {
    host: Instant,
    stream: cpal::StreamInstant,
}
impl AudioClock {
    pub fn sample(stream: &cpal::Stream) -> Self {
        use cpal::traits::StreamTrait;
        let before = Instant::now();
        let timestamp = stream.now();
        Self {
            host: before + before.elapsed() / 2,
            stream: timestamp,
        }
    }
    pub fn instant(&self, stamp: cpal::StreamInstant) -> Result<Instant> {
        let value = if let Some(delta) = stamp.checked_duration_since(self.stream) {
            self.host.checked_add(delta)
        } else {
            self.host.checked_sub(
                self.stream
                    .checked_duration_since(stamp)
                    .context("Invalid audio timestamp")?,
            )
        };
        value.context("Audio timestamp overflow")
    }
}
