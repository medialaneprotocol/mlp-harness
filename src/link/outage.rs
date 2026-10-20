//! The outage schedule.
//!
//! An outage is a window of time during which the link misbehaves in a
//! stated way. Three kinds, because a network fails in three ways:
//!
//! * a **blackhole** — nothing gets through, in either direction;
//! * a **one-way blackhole** — one direction is dead and the other works,
//!   which is what a half-open path or a broken return route looks like;
//! * a **partial-loss window** — the link works, but badly: objects are
//!   dropped at a stated rate.
//!
//! An outage is checked at two moments: when an object is offered (a link
//! that is blackholed does not accept it) and when an object would be
//! delivered (a link that goes black while the object is in flight loses
//! it). Both are attributed to the outage, so a drop is never unexplained.

use serde::{Deserialize, Serialize};

/// Which direction an outage applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// The sender → receiver direction.
    Down,
    /// The receiver → sender direction.
    Up,
}

impl Direction {
    /// The stable label spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Up => "up",
        }
    }

    /// The other direction.
    pub const fn peer(self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
        }
    }
}

/// What an outage does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutageKind {
    /// Nothing gets through, in either direction.
    Blackhole,
    /// One direction is dead; the other works.
    OneWayBlackhole {
        /// The dead direction.
        direction: Direction,
    },
    /// The link works, but drops objects at a stated rate.
    PartialLoss {
        /// The drop rate, in permille.
        permille: u32,
    },
}

impl OutageKind {
    /// The stable label spelling.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Blackhole => "blackhole",
            Self::OneWayBlackhole { .. } => "one-way-blackhole",
            Self::PartialLoss { .. } => "partial-loss",
        }
    }

    /// Whether the outage affects a direction.
    pub fn affects(&self, direction: Direction) -> bool {
        match self {
            Self::Blackhole => true,
            Self::OneWayBlackhole { direction: dead } => *dead == direction,
            Self::PartialLoss { .. } => true,
        }
    }

    /// Whether the outage stops the link entirely for a direction.
    pub fn is_blackhole(&self, direction: Direction) -> bool {
        match self {
            Self::Blackhole => true,
            Self::OneWayBlackhole { direction: dead } => *dead == direction,
            Self::PartialLoss { .. } => false,
        }
    }
}

/// One outage window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutageWindow {
    /// When the window starts, in microseconds.
    pub start_us: u64,
    /// How long the window lasts, in microseconds.
    pub duration_us: u64,
    /// What the outage does.
    pub kind: OutageKind,
}

impl OutageWindow {
    /// A window that starts at a time and lasts a while.
    pub fn new(start_us: u64, duration_us: u64, kind: OutageKind) -> Self {
        Self {
            start_us,
            duration_us,
            kind,
        }
    }

    /// Whether the window covers a time.
    pub fn covers(&self, at_us: u64) -> bool {
        at_us >= self.start_us && at_us < self.start_us.saturating_add(self.duration_us)
    }

    /// When the window ends, in microseconds.
    pub fn end_us(&self) -> u64 {
        self.start_us.saturating_add(self.duration_us)
    }
}

/// A link's outages.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OutageSchedule {
    /// The windows, in the order they were declared.
    pub windows: Vec<OutageWindow>,
}

impl OutageSchedule {
    /// An empty schedule.
    pub fn new() -> Self {
        Self::default()
    }

    /// A schedule with the given windows.
    pub fn of(windows: Vec<OutageWindow>) -> Self {
        Self { windows }
    }

    /// A schedule of blackholes: `count` windows of `duration_us`, spaced
    /// `up_us` apart, starting at `start_us`.
    pub fn flap(count: u32, start_us: u64, down_us: u64, up_us: u64) -> Self {
        let windows = (0..count)
            .map(|index| {
                OutageWindow::new(
                    start_us + u64::from(index) * (down_us + up_us),
                    down_us,
                    OutageKind::Blackhole,
                )
            })
            .collect();
        Self { windows }
    }

    /// The window covering a time for a direction, if any.
    pub fn covering(&self, at_us: u64, direction: Direction) -> Option<&OutageWindow> {
        self.windows
            .iter()
            .find(|window| window.covers(at_us) && window.kind.affects(direction))
    }

    /// The next window that starts at or after a time.
    pub fn next_start_after(&self, at_us: u64) -> Option<u64> {
        self.windows
            .iter()
            .filter(|window| window.start_us > at_us)
            .map(|window| window.start_us)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_covers_the_times_inside_it() {
        let window = OutageWindow::new(1_000, 500, OutageKind::Blackhole);
        assert!(!window.covers(999));
        assert!(window.covers(1_000));
        assert!(window.covers(1_499));
        assert!(!window.covers(1_500));
    }

    #[test]
    fn a_blackhole_affects_both_directions() {
        let schedule = OutageSchedule::of(vec![OutageWindow::new(
            0,
            1_000,
            OutageKind::Blackhole,
        )]);
        assert!(schedule.covering(500, Direction::Down).is_some());
        assert!(schedule.covering(500, Direction::Up).is_some());
    }

    #[test]
    fn a_one_way_blackhole_affects_one_direction() {
        let schedule = OutageSchedule::of(vec![OutageWindow::new(
            0,
            1_000,
            OutageKind::OneWayBlackhole {
                direction: Direction::Up,
            },
        )]);
        assert!(schedule.covering(500, Direction::Down).is_none());
        assert!(schedule.covering(500, Direction::Up).is_some());
    }

    #[test]
    fn a_partial_loss_window_is_not_a_blackhole() {
        let window = OutageWindow::new(
            0,
            1_000,
            OutageKind::PartialLoss {
                permille: 500,
            },
        );
        assert!(window.kind.affects(Direction::Down));
        assert!(!window.kind.is_blackhole(Direction::Down));
    }

    #[test]
    fn a_flap_schedule_spaces_its_windows() {
        let schedule = OutageSchedule::flap(5, 1_000, 10_000, 10_000);
        assert_eq!(schedule.windows.len(), 5);
        assert!(schedule.covering(1_500, Direction::Down).is_some());
        assert!(schedule.covering(11_500, Direction::Down).is_none());
        assert!(schedule.covering(21_500, Direction::Down).is_some());
    }
}
