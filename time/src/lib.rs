// Copyright 2021 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use time::OffsetDateTime;

pub use std::time::SystemTime;

mod datetime;
mod duration;
mod instant;

pub use datetime::*;
pub use duration::*;
pub use instant::*;

const MILLIS_PER_SEC: u64 = 1_000;
const MICROS_PER_SEC: u64 = 1_000_000;
const NANOS_PER_SEC: u64 = 1_000_000_000;
const NANOS_PER_MILLI: u64 = 1_000_000;
const NANOS_PER_MICRO: u64 = 1_000;

// We initialize the clock for the static lifetime.
static CLOCK: Clock = Clock::new();

// convenience functions

/// Refresh the clock and return the current instant with high precision.
pub fn now_precise() -> Instant {
    CLOCK.refresh();
    CLOCK.recent_precise()
}

/// Refresh the clock and return the current instant with reduced precision.
pub fn now_coarse() -> CoarseInstant {
    // CLOCK.refresh();
    CLOCK.recent_coarse()
}

/// Refresh the clock and return the current system time.
pub fn now_system() -> SystemTime {
    CLOCK.refresh();
    CLOCK.recent_system()
}

/// Refresh the clock and return the current unix time in seconds.
pub fn now_unix() -> u32 {
    CLOCK.refresh();
    CLOCK.recent_unix()
}

/// Refresh the clock and return the current `DateTime` in the UTC timezone.
pub fn now_utc() -> DateTime {
    CLOCK.refresh();
    CLOCK.recent_utc()
}

/// Returns a recent precise instant by reading a cached view of the clock.
pub fn recent_precise() -> Instant {
    CLOCK.recent_precise()
}

/// Returns a recent coarse instant by reading a cached view of the clock.
pub fn recent_coarse() -> CoarseInstant {
    CLOCK.recent_coarse()
}

/// Returns the system time by reaching a cached view of the clock.
pub fn recent_system() -> SystemTime {
    CLOCK.recent_system()
}

/// Returns the unix time by reading a cached view of the clock.
pub fn recent_unix() -> u32 {
    CLOCK.recent_unix()
}

/// Returns a `DateTime` in UTC from a cached view of the clock.
pub fn recent_utc() -> DateTime {
    CLOCK.recent_utc()
}

/// Update the cached view of the clock by reading the underlying clock.
pub fn refresh_clock() {
    CLOCK.refresh()
}

pub fn refresh_with_sec_timestamp(timestamp: u32) -> bool {
    CLOCK.refresh_with_sec_timestamp(timestamp)
}

// Clock provides functionality to get current and recent times
struct Clock {
    initialized: AtomicBool,
    recent_coarse: AtomicCoarseInstant,
    recent_precise: AtomicInstant,
    recent_unix: AtomicU32,
}

impl Clock {
    fn initialize(&self) {
        if !self.initialized.load(Ordering::Relaxed) {
            // self.refresh();
            self.refresh_with_sec_timestamp(0);
        }
    }

    /// Return a cached precise time
    fn recent_precise(&self) -> Instant {
        self.initialize();
        self.recent_precise.load(Ordering::Relaxed)
    }

    /// Return a cached coarse time
    fn recent_coarse(&self) -> CoarseInstant {
        // self.initialize();
        self.recent_coarse.load(Ordering::Relaxed)
    }

    /// Return a cached SystemTime
    fn recent_system(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(self.recent_unix().into())
    }

    /// Return a cached UNIX time
    fn recent_unix(&self) -> u32 {
        self.initialize();
        self.recent_unix.load(Ordering::Relaxed)
    }

    /// Return a cached UTC DateTime
    fn recent_utc(&self) -> DateTime {
        // This unwrap is safe, because we use a 32bit number of seconds. Tests
        // enforce the correctness of this below.
        let recent = OffsetDateTime::from_unix_timestamp(self.recent_unix() as i64).unwrap()
            + time::Duration::nanoseconds(0);
        DateTime { inner: recent }
    }

    /// Refresh the cached time
    fn refresh(&self) {
        let precise = Instant::now();
        let coarse = CoarseInstant {
            secs: (precise.nanos / NANOS_PER_SEC) as u32,
        };

        self.recent_precise.store(precise, Ordering::Relaxed);

        // special case initializing the recent unix time
        if self.initialized.load(Ordering::Relaxed) {
            let last = self.recent_coarse.swap(coarse, Ordering::Relaxed);
            if last < coarse {
                let delta = (coarse - last).as_secs();
                self.recent_unix.fetch_add(delta, Ordering::Relaxed);
            }
        } else {
            self.recent_coarse.store(coarse, Ordering::Relaxed);
            let unix = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32;
            self.recent_unix.store(unix, Ordering::Relaxed);
        }
        self.initialized.store(true, Ordering::Relaxed);
    }

    /// Refresh the cached time
    pub fn refresh_with_sec_timestamp(&self, timestamp: u32) -> bool {
        let precise = Instant{nanos: timestamp as u64 * NANOS_PER_SEC};
        let coarse = CoarseInstant {secs: timestamp};

        self.recent_precise.store(precise, Ordering::Relaxed);
        let last = self.recent_coarse.swap(coarse, Ordering::Relaxed);
        if last + CoarseDuration::SECOND < coarse {
            true
        } else if coarse < last {
            let delta = (last - coarse).as_secs();
            if delta > 1 {
                panic!("timestamp becomes smaller, previous {:?} now {:?}", last, coarse)
            }
            false
        }
        else {
            false
        } 
 
        // special case initializing the recent unix time
//        if self.initialized.load(Ordering::Relaxed) {
//            let last = self.recent_coarse.swap(coarse, Ordering::Relaxed);
//            if last < coarse {
//                true
//            } else {
//                false
//            }
//        } else {
//            self.recent_coarse.store(coarse, Ordering::Relaxed);
//            self.initialized.store(true, Ordering::Relaxed);
//            true
//	}
    }
}

impl Clock {
    const fn new() -> Self {
        Clock {
            initialized: AtomicBool::new(false),
            recent_coarse: AtomicCoarseInstant {
                secs: AtomicU32::new(0),
            },
            recent_precise: AtomicInstant {
                nanos: AtomicU64::new(0),
            },
            recent_unix: AtomicU32::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    // This tests the direct interface to `Instant` and `Duration`
    fn basic() {
        let now = Instant::now();
        std::thread::sleep(std::time::Duration::new(1, 0));
        let elapsed = now.elapsed();
        assert!(elapsed.as_secs_f64() >= 1.0);
        assert!(elapsed.as_secs() >= 1);
        assert!(elapsed.as_nanos() >= NANOS_PER_SEC.into());
    }

    #[test]
    /// This tests the system time handling
    fn system() {
        let recent = recent_system();
        let now = std::time::SystemTime::now();
        assert!((now.duration_since(recent).unwrap()).as_secs() <= 1);
    }

    #[test]
    // This tests the 'clock' interface which is hidden behind macros
    fn clock() {
        let now = Instant::now();
        std::thread::sleep(std::time::Duration::new(1, 0));
        let elapsed = now.elapsed();
        assert!(elapsed.as_secs() >= 1);
        assert!(elapsed.as_nanos() >= NANOS_PER_SEC.into());

        let t0 = Instant::recent();
        let t0_c = Instant::recent();
        std::thread::sleep(std::time::Duration::new(1, 0));
        assert_eq!(Instant::recent(), t0);
        refresh_clock();
        let t1 = Instant::recent();
        let t1_c = Instant::recent();
        assert!((t1 - t0).as_secs_f64() >= 1.0);
        assert!((t1_c - t0_c).as_secs() >= 1);
    }
}
