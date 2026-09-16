//! WGC, MediaFrameReference and WASAPI report QPC time in 100 ns units.
use anyhow::{Result, ensure};
use std::time::{Duration, Instant};
use windows::Win32::System::{
    Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};

pub struct Apartment;
impl Apartment {
    pub fn new() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

#[derive(Clone, Copy)]
pub struct HostClock {
    host: Instant,
    units: i64,
}
impl HostClock {
    pub fn new() -> Result<Self> {
        let (mut ticks, mut frequency) = (0, 0);
        unsafe {
            QueryPerformanceFrequency(&mut frequency)?;
        }
        let before = Instant::now();
        unsafe {
            QueryPerformanceCounter(&mut ticks)?;
        }
        let after = Instant::now();
        ensure!(frequency > 0 && ticks >= 0, "Invalid Windows host clock");
        Ok(Self {
            host: before + (after - before) / 2,
            units: (ticks as i128 * 10_000_000 / frequency as i128) as i64,
        })
    }
    pub fn instant(&self, units: i64) -> Result<Instant> {
        ensure!(
            units >= 0,
            "The device returned an invalid capture timestamp"
        );
        let delta = Duration::from_nanos(
            units
                .abs_diff(self.units)
                .checked_mul(100)
                .ok_or_else(|| anyhow::anyhow!("Capture timestamp overflow"))?,
        );
        let time = if units >= self.units {
            self.host.checked_add(delta)
        } else {
            self.host.checked_sub(delta)
        };
        time.ok_or_else(|| anyhow::anyhow!("Capture timestamp is outside the host clock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qpc_mapping_keeps_acquisition_time_across_delayed_delivery() {
        let clock = HostClock::new().unwrap();
        assert_eq!(clock.instant(clock.units).unwrap(), clock.host);
        assert_eq!(
            clock.instant(clock.units - 1_000_000).unwrap() + Duration::from_millis(100),
            clock.host
        );
        assert_eq!(
            clock.instant(clock.units + 10_000_000).unwrap() - clock.host,
            Duration::from_secs(1)
        );
        assert!(clock.instant(-1).is_err());
    }
}
