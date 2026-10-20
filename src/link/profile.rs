//! The link profile: what a link is, parameter by parameter.
//!
//! A profile is the *whole* description of a link at one moment. It is
//! serializable, so a scenario names the profiles it uses and the harness
//! reads them from the file. Every parameter has a default, and the default
//! is the tier's canonical point.
//!
//! # The parameters
//!
//! | Parameter | Meaning | Default |
//! |---|---|---|
//! | `rate_bps` | the direction's rate, in bits per second | the tier's floor |
//! | `propagation_us` | the one-way propagation delay | half the tier's RTT |
//! | `jitter` | the delay's random component | the tier's jitter bound |
//! | `loss` | how objects are lost | the tier's loss bound |
//! | `reorder_permille` | the share of objects displaced out of order | 0 |
//! | `duplicate_permille` | the share of objects delivered twice | 0 |
//! | `queue_cap_bytes` | the queue's cap | one second of the rate |
//! | `burst_bytes` | the token bucket's size | one second of the rate |
//! | `mtu_bytes` | the largest object the carrier admits | 65 536 |
//!
//! The uplink and the downlink are **separate** directions with separate
//! profiles: a link whose downlink is 1 Mbps and whose uplink is 256 kbps is
//! the normal case, not the exotic one.

use serde::{Deserialize, Serialize};

use super::jitter::Jitter;
use super::loss::LossModel;
use super::outage::OutageSchedule;

/// The protocol's target tiers (MLP spec §2.1).
///
/// A tier is a *point* in the target envelope, not the envelope itself: the
/// specification states the bands, and the harness's tier profiles sit at
/// the band edge the specification's own numbers name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// LAN: ≥100 Mbps / ≤10 ms / no loss.
    T0,
    /// Broadband: ≥5 Mbps / 50 ms / ≤0.1 %.
    T1,
    /// Poor Wi-Fi or hotspot: ≥1 Mbps / 150 ms / ≤2 %.
    T2,
    /// Long-haul degraded: 256–512 kbps / 400 ms / 5–10 %.
    T3,
    /// Extreme: 32–64 kbps / 800–2000 ms / 20–50 %.
    T4,
}

impl Tier {
    /// The stable metric label spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T0 => "t0",
            Self::T1 => "t1",
            Self::T2 => "t2",
            Self::T3 => "t3",
            Self::T4 => "t4",
        }
    }

    /// Every tier, in order.
    pub const fn all() -> [Tier; 5] {
        [Self::T0, Self::T1, Self::T2, Self::T3, Self::T4]
    }

    /// The tier named by a label, in either spelling.
    pub fn from_label(label: &str) -> Option<Self> {
        match label.to_ascii_lowercase().as_str() {
            "t0" => Some(Self::T0),
            "t1" => Some(Self::T1),
            "t2" => Some(Self::T2),
            "t3" => Some(Self::T3),
            "t4" => Some(Self::T4),
            _ => None,
        }
    }

    /// The round-trip time the tier's band names, in milliseconds.
    pub const fn rtt_ms(self) -> u64 {
        match self {
            Self::T0 => 10,
            Self::T1 => 50,
            Self::T2 => 150,
            Self::T3 => 400,
            Self::T4 => 1_400,
        }
    }

    /// The echo budget the tier promises: `2 × RTT + 250 ms` (MLP spec §2.1).
    pub const fn echo_budget_ms(self) -> u64 {
        2 * self.rtt_ms() + 250
    }
}

/// One direction of a link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectionProfile {
    /// The direction's rate in bits per second.
    pub rate_bps: u64,
    /// The one-way propagation delay in microseconds.
    pub propagation_us: u64,
    /// The delay's random component.
    pub jitter: Jitter,
    /// How the direction loses objects.
    pub loss: LossModel,
    /// The share of objects displaced out of order, in permille.
    pub reorder_permille: u32,
    /// The share of objects delivered twice, in permille.
    pub duplicate_permille: u32,
    /// The queue's cap in bytes. An object that does not fit is dropped.
    pub queue_cap_bytes: u64,
    /// The token bucket's size in bytes: the largest burst the link passes
    /// at once.
    pub burst_bytes: u64,
    /// The largest displacement a reordering may cause, in microseconds.
    pub reorder_window_us: u64,
    /// The delay between an object and its duplicate, in microseconds.
    pub duplicate_delay_us: u64,
    /// An extra delay this direction adds to every object.
    pub extra_delay_us: u64,
}

impl Default for DirectionProfile {
    fn default() -> Self {
        Self {
            rate_bps: 1_000_000,
            propagation_us: 5_000,
            jitter: Jitter::none(),
            loss: LossModel::None,
            reorder_permille: 0,
            duplicate_permille: 0,
            queue_cap_bytes: 125_000,
            burst_bytes: 125_000,
            reorder_window_us: 10_000,
            duplicate_delay_us: 1_000,
            extra_delay_us: 0,
        }
    }
}

impl DirectionProfile {
    /// The bytes the link carries per second.
    pub fn bytes_per_second(&self) -> u64 {
        (self.rate_bps / 8).max(1)
    }

    /// The bytes the link's token bucket hands out per millisecond.
    pub fn bytes_per_ms(&self) -> u64 {
        (self.rate_bps / 8 / 1_000).max(1)
    }

    /// The time the link needs to carry `bytes` at its rate, in
    /// microseconds.
    pub fn serialization_us(&self, bytes: u64) -> u64 {
        if self.rate_bps == 0 {
            return u64::MAX / 4;
        }
        (bytes * 8 * 1_000_000).div_ceil(self.rate_bps)
    }
}

/// A whole link: two directions, an MTU, an outage schedule, a seed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkProfile {
    /// The sender → receiver direction.
    pub down: DirectionProfile,
    /// The receiver → sender direction.
    pub up: DirectionProfile,
    /// The largest object the carrier admits, in bytes.
    ///
    /// An object larger than this is **dropped**, not fragmented: the
    /// protocol fragments an update into parts that fit the carrier, so an
    /// oversize object is a breach of the protocol's own rules and the link
    /// does not paper over it.
    pub mtu_bytes: u64,
    /// The outages this link suffers.
    pub outages: OutageSchedule,
    /// The seed of the generator the link's randomness comes from.
    pub seed: u64,
}

impl Default for LinkProfile {
    fn default() -> Self {
        Self::tier(Tier::T0)
    }
}

impl LinkProfile {
    /// The canonical point of a tier's envelope.
    ///
    /// The rate is the tier's floor, the round trip is the tier's stated
    /// value, the loss is the tier's upper bound (the harness asserts at the
    /// worst case on purpose) and the jitter is the tier's bound.
    pub fn tier(tier: Tier) -> Self {
        let rtt_us = tier.rtt_ms() * 1_000;
        let (down_bps, up_bps, jitter, loss, mtu) = match tier {
            Tier::T0 => (
                100_000_000,
                10_000_000,
                Jitter::normal(2_000),
                LossModel::Uniform { permille: 0 },
                65_536,
            ),
            Tier::T1 => (
                5_000_000,
                1_000_000,
                Jitter::normal(10_000),
                LossModel::Uniform { permille: 1 },
                65_536,
            ),
            Tier::T2 => (
                1_000_000,
                256_000,
                Jitter::normal(40_000),
                LossModel::Uniform { permille: 20 },
                65_536,
            ),
            Tier::T3 => (
                256_000,
                128_000,
                Jitter::normal(100_000),
                LossModel::Uniform { permille: 100 },
                65_536,
            ),
            Tier::T4 => (
                32_000,
                32_000,
                Jitter::pareto(500_000),
                LossModel::Uniform { permille: 350 },
                1_150,
            ),
        };
        let mut down = DirectionProfile {
            rate_bps: down_bps,
            propagation_us: rtt_us / 2,
            jitter,
            loss,
            ..DirectionProfile::default()
        };
        let mut up = DirectionProfile {
            rate_bps: up_bps,
            propagation_us: rtt_us / 2,
            jitter,
            loss,
            ..DirectionProfile::default()
        };
        // The bucket and the queue hold one second of the direction's rate:
        // a burst of that size passes at once, and a burst beyond it waits.
        down.burst_bytes = down.bytes_per_second();
        down.queue_cap_bytes = down.burst_bytes * 2;
        up.burst_bytes = up.bytes_per_second();
        up.queue_cap_bytes = up.burst_bytes * 2;
        Self {
            down,
            up,
            mtu_bytes: mtu,
            outages: OutageSchedule::default(),
            seed: 0x5EED_1234_ABCD_0001,
        }
    }

    /// A profile with no randomness at all: no loss, no jitter, no
    /// reordering, no duplication.
    pub fn perfect() -> Self {
        let mut profile = Self::tier(Tier::T0);
        profile.down.jitter = Jitter::none();
        profile.up.jitter = Jitter::none();
        profile.down.loss = LossModel::None;
        profile.up.loss = LossModel::None;
        profile
    }

    /// The same profile with a different seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// The same profile with a different outage schedule.
    pub fn with_outages(mut self, outages: OutageSchedule) -> Self {
        self.outages = outages;
        self
    }
}
