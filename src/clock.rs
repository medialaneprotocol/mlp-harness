//! The virtual clock.
//!
//! The harness has no wall clock. Every millisecond it spends is one it was
//! told to spend, and the machines it drives receive time as a parameter. A
//! scenario that runs twice runs identically, on any machine, in any order,
//! in any process.
//!
//! # The unit
//!
//! Time is counted in **microseconds**. The protocol's own budgets are
//! expressed in milliseconds and its jitter is a continuous distribution, so
//! microseconds are the finest whole unit that keeps every time in the model
//! an exact integer: no float ever carries a timestamp, and no two runs can
//! disagree about a millisecond because of a rounding difference.
//!
//! # The epoch
//!
//! The protocol's machines are handed an [`Instant`], and they measure
//! everything as a difference from the instant they were started at. The
//! harness therefore needs an origin, and it takes one from the process at
//! startup. That origin is a *base*, not a reading: it never appears in a
//! trace, it never reaches a decision, and every number the harness reports
//! is a difference from it. Two runs of the same scenario produce the same
//! bytes whatever the origin was.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The harness's time unit: one microsecond.
pub const MICROS_PER_MILLI: u64 = 1_000;

/// The process-wide origin the harness's clocks start from.
///
/// It is captured once so that every clock in a process shares one origin;
/// nothing derived from it is observable.
fn process_origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// The origin every clock in this process starts from.
///
/// A consumer's own machines are handed an [`Instant`], and they measure
/// everything as a difference from the instant they were started at. This
/// is that instant: one per process, so that several machines in one test
/// binary agree on where they began.
pub fn epoch() -> Instant {
    process_origin()
}

/// A clock the harness owns.
#[derive(Debug, Clone)]
pub struct VirtualClock {
    origin: Instant,
    elapsed_us: u64,
}

impl VirtualClock {
    /// A clock that starts now, at the process origin.
    pub fn new() -> Self {
        Self {
            origin: process_origin(),
            elapsed_us: 0,
        }
    }

    /// A clock that starts at a given origin.
    pub fn at(origin: Instant) -> Self {
        Self {
            origin,
            elapsed_us: 0,
        }
    }

    /// The origin the clock's instants are measured from.
    pub fn origin(&self) -> Instant {
        self.origin
    }

    /// The elapsed time, in microseconds.
    pub fn elapsed_us(&self) -> u64 {
        self.elapsed_us
    }

    /// The elapsed time, in milliseconds (truncated).
    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_us / MICROS_PER_MILLI
    }

    /// The current instant, for the machines the harness drives.
    pub fn instant(&self) -> Instant {
        self.origin + Duration::from_micros(self.elapsed_us)
    }

    /// The current time in milliseconds, for the machines that count in
    /// milliseconds.
    pub fn now_ms(&self) -> u64 {
        self.elapsed_ms()
    }

    /// Move the clock forward by a number of microseconds.
    pub fn advance_us(&mut self, micros: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(micros);
    }

    /// Move the clock forward by a number of milliseconds.
    pub fn advance_ms(&mut self, millis: u64) {
        self.advance_us(millis.saturating_mul(MICROS_PER_MILLI));
    }

    /// Move the clock forward by a duration.
    pub fn advance(&mut self, duration: Duration) {
        self.advance_us(duration.as_micros() as u64);
    }

    /// Move the clock to an absolute elapsed time.
    ///
    /// A time in the past is ignored: the clock never runs backwards.
    pub fn advance_to_us(&mut self, micros: u64) {
        if micros > self.elapsed_us {
            self.elapsed_us = micros;
        }
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_starts_at_zero_and_advances_exactly() {
        let mut clock = VirtualClock::new();
        assert_eq!(clock.elapsed_us(), 0);
        assert_eq!(clock.now_ms(), 0);

        clock.advance_ms(1);
        assert_eq!(clock.elapsed_us(), 1_000);
        assert_eq!(clock.now_ms(), 1);

        clock.advance_us(500);
        assert_eq!(clock.elapsed_us(), 1_500);
        assert_eq!(clock.now_ms(), 1);
    }

    #[test]
    fn the_clock_never_runs_backwards() {
        let mut clock = VirtualClock::new();
        clock.advance_to_us(10_000);
        clock.advance_to_us(1_000);
        assert_eq!(clock.elapsed_us(), 10_000);
    }

    #[test]
    fn the_instant_tracks_the_elapsed_time() {
        let mut clock = VirtualClock::new();
        let start = clock.instant();
        clock.advance_ms(250);
        assert_eq!(clock.instant() - start, Duration::from_millis(250));
    }

    #[test]
    fn two_clocks_agree_on_their_differences() {
        let mut first = VirtualClock::new();
        let mut second = VirtualClock::new();
        first.advance_ms(17);
        second.advance_ms(17);
        assert_eq!(first.elapsed_us(), second.elapsed_us());
        assert_eq!(first.now_ms(), second.now_ms());
    }
}
