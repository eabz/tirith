//! Injectable time.
//!
//! Every timestamp in Tirith comes from a [`Clock`] so lease expiry can be
//! tested by advancing a [`ManualClock`] instead of sleeping.

use std::fmt;
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Duration, Utc};

/// A source of the current time.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current instant in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// The real wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that only moves when told to. Used in tests.
///
/// ```
/// use chrono::{Duration, TimeZone, Utc};
/// use tirith::clock::{Clock, ManualClock};
///
/// let clock = ManualClock::new(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
/// clock.advance(Duration::seconds(30));
/// assert_eq!(clock.now().timestamp(), 1_767_225_630);
/// ```
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    /// Creates a clock frozen at `start`.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now += by;
    }

    /// Sets the clock to an absolute instant.
    pub fn set(&self, to: DateTime<Utc>) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = to;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
