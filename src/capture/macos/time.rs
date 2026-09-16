//! Translate native capture timestamps into one monotonic clock. Callback arrival
//! time is never used as the presentation time of a captured sample.
use objc2_core_media::{CMClock, CMSyncConvertTime, CMTime};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

struct Anchor {
    host: f64,
    instant: Instant,
}
static ANCHOR: OnceLock<Anchor> = OnceLock::new();

pub fn host_instant(seconds: f64) -> Option<Instant> {
    if !seconds.is_finite() {
        return None;
    }
    let anchor = ANCHOR.get_or_init(|| {
        // Minimize scheduling error while bridging Apple's host clock and Rust.
        (0..8)
            .map(|_| {
                let before = Instant::now();
                let host = unsafe { CMClock::host_time_clock().time().seconds() };
                let span = before.elapsed();
                (
                    span,
                    Anchor {
                        host,
                        instant: before + span / 2,
                    },
                )
            })
            .min_by_key(|(span, _)| *span)
            .unwrap()
            .1
    });
    let delta = seconds - anchor.host;
    let duration = Duration::try_from_secs_f64(delta.abs()).ok()?;
    if delta >= 0.0 {
        anchor.instant.checked_add(duration)
    } else {
        anchor.instant.checked_sub(duration)
    }
}

pub fn capture_instant(time: CMTime, clock: &CMClock) -> Option<Instant> {
    // Both objects are CMClock instances. CoreMedia also compensates for drift
    // between a device's synchronization clock and the host clock.
    let host = unsafe { CMClock::host_time_clock() };
    let time = unsafe { CMSyncConvertTime(time, clock.as_ref(), host.as_ref()) };
    host_instant(unsafe { time.seconds() })
}
