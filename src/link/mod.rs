//! The link: a network, not a delay.
//!
//! A link here is a **network** with the properties a real one has:
//!
//! * a **rate**, shaped by a token bucket and a queue, so a burst waits its
//!   turn rather than travelling all at once;
//! * a **propagation delay** with a jitter distribution on top of it;
//! * **loss**, in one of four shapes, each of which a real network exhibits;
//! * **reordering** within a bounded window, and **duplication** at a rate;
//! * an **MTU**, above which an object is dropped rather than fragmented;
//! * **outages** on a schedule: a blackhole, a one-way blackhole, a
//!   partial-loss window;
//! * two **separate directions**, the uplink and the downlink, each with its
//!   own profile;
//! * a **carrier handover**, at which the objects in flight are split
//!   between the old carrier and the new one and some of them are lost.
//!
//! # The unit
//!
//! Everything is counted in microseconds and bytes. The rate is applied
//! through a token bucket held in **milli-bytes**, so that a rate of any
//! precision is represented exactly and no rounding ever decides a
//! delivery time.
//!
//! # The order of events
//!
//! An object offered to the link is, in this order:
//!
//! 1. measured against the MTU — an oversize object is dropped;
//! 2. measured against the outage schedule — a blackholed link takes
//!    nothing;
//! 3. rolled against the loss model, with the queue's occupancy in hand;
//! 4. measured against the queue's cap — a full queue drops it;
//! 5. queued, waiting for the bucket's tokens.
//!
//! When the bucket has the tokens, the object is **released**: it leaves the
//! queue, takes its propagation delay and its jitter, and is in flight. It
//! is delivered when the clock reaches its delivery time, unless an outage
//! has started in the meantime.

pub mod handover;
pub mod jitter;
pub mod loss;
pub mod outage;
pub mod profile;

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use mlp_core::media::pdu::Priority;

use crate::rng::Generator;

pub use handover::Handover;
pub use jitter::Jitter;
pub use loss::{LossModel, LossState};
pub use outage::{Direction, OutageKind, OutageSchedule, OutageWindow};
pub use profile::{DirectionProfile, LinkProfile, Tier, TierBand};

/// The token bucket's unit: a millionth of a byte.
///
/// The bucket counts in micro-bytes so that a rate of any precision is
/// represented exactly: one microsecond of a 1 Mbps link is 0.125 bytes,
/// and the bucket must be able to say so.
const MICRO_BYTES: u64 = 1_000_000;

/// Why an object did not arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DropReason {
    /// The object was larger than the carrier's MTU.
    TooLarge,
    /// The loss model lost it.
    Loss,
    /// An outage took it.
    Outage,
    /// The queue was full.
    QueueOverflow,
    /// The carrier handover lost it.
    Handover,
}

impl DropReason {
    /// The stable label spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too-large",
            Self::Loss => "loss",
            Self::Outage => "outage",
            Self::QueueOverflow => "queue-overflow",
            Self::Handover => "handover",
        }
    }
}

/// What happened to an object offered to the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferOutcome {
    /// The object is queued and will be released when the bucket allows.
    Queued {
        /// The object's sequence number.
        seq: u64,
    },
    /// The object was dropped.
    Dropped {
        /// The object's sequence number.
        seq: u64,
        /// Why.
        reason: DropReason,
    },
}

/// One delivered object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The direction the object travelled.
    pub direction: Direction,
    /// The object's bytes.
    pub bytes: Vec<u8>,
    /// When it arrived, in microseconds.
    pub at_us: u64,
    /// The object's sequence number.
    pub seq: u64,
    /// Whether this is the duplicate of an object already delivered.
    pub duplicate: bool,
}

/// One thing the link did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkEvent {
    /// An object was offered to the link.
    Offered {
        /// When, in microseconds.
        at_us: u64,
        /// The direction it was offered in.
        direction: Direction,
        /// The object's sequence number.
        seq: u64,
        /// Its size, in bytes.
        bytes: u64,
    },
    /// An object left the queue and entered the pipe.
    Released {
        /// When, in microseconds.
        at_us: u64,
        /// The direction it travelled in.
        direction: Direction,
        /// The object's sequence number.
        seq: u64,
        /// Its size, in bytes.
        bytes: u64,
    },
    /// An object arrived.
    Delivered {
        /// When, in microseconds.
        at_us: u64,
        /// The direction it travelled in.
        direction: Direction,
        /// The object's sequence number.
        seq: u64,
        /// Its size, in bytes.
        bytes: u64,
        /// Whether it is the duplicate of an object already delivered.
        duplicate: bool,
    },
    /// An object was lost.
    Dropped {
        /// When, in microseconds.
        at_us: u64,
        /// The direction it was offered in.
        direction: Direction,
        /// The object's sequence number.
        seq: u64,
        /// Its size, in bytes.
        bytes: u64,
        /// Why it was lost.
        reason: DropReason,
    },
    /// The link's profile was replaced.
    ProfileChanged {
        /// When, in microseconds.
        at_us: u64,
    },
    /// The carrier changed.
    Handover {
        /// When, in microseconds.
        at_us: u64,
        /// The share of the objects in flight that stayed on the old
        /// carrier, in permille.
        split_permille: u32,
        /// The share of the objects in flight the switch lost, in permille.
        loss_permille: u32,
        /// How many objects were in flight.
        in_flight: usize,
    },
}

/// One direction's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DirectionStats {
    /// Objects offered.
    pub offered: u64,
    /// Objects delivered.
    pub delivered: u64,
    /// Objects lost to the loss model.
    pub dropped_loss: u64,
    /// Objects lost to an outage.
    pub dropped_outage: u64,
    /// Objects lost because they exceeded the MTU.
    pub dropped_too_large: u64,
    /// Objects lost because the queue was full.
    pub dropped_queue_overflow: u64,
    /// Objects lost at a carrier handover.
    pub dropped_handover: u64,
    /// Duplicates delivered.
    pub duplicated: u64,
    /// Objects displaced out of order.
    pub reordered: u64,
    /// Bytes offered.
    pub bytes_offered: u64,
    /// Bytes delivered.
    pub bytes_delivered: u64,
    /// The queue's highest occupancy, in bytes.
    pub queue_peak_bytes: u64,
    /// The total time objects spent waiting for the bucket, in
    /// microseconds.
    pub queue_delay_us: u64,
    /// The highest number of objects in flight at once.
    pub in_flight_peak: usize,
}

/// A link's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkStats {
    /// The sender → receiver direction.
    pub down: DirectionStats,
    /// The receiver → sender direction.
    pub up: DirectionStats,
}

impl LinkStats {
    /// The counters of one direction.
    pub fn of(&self, direction: Direction) -> &DirectionStats {
        match direction {
            Direction::Down => &self.down,
            Direction::Up => &self.up,
        }
    }

    /// The counters of one direction, mutably.
    pub fn of_mut(&mut self, direction: Direction) -> &mut DirectionStats {
        match direction {
            Direction::Down => &mut self.down,
            Direction::Up => &mut self.up,
        }
    }

    /// Every object offered, in both directions.
    pub fn offered(&self) -> u64 {
        self.down.offered + self.up.offered
    }

    /// Every object delivered, in both directions.
    pub fn delivered(&self) -> u64 {
        self.down.delivered + self.up.delivered
    }

    /// Every object lost, in both directions.
    pub fn dropped(&self) -> u64 {
        self.down.dropped_loss
            + self.down.dropped_outage
            + self.down.dropped_too_large
            + self.down.dropped_queue_overflow
            + self.down.dropped_handover
            + self.up.dropped_loss
            + self.up.dropped_outage
            + self.up.dropped_too_large
            + self.up.dropped_queue_overflow
            + self.up.dropped_handover
    }

    /// Every byte offered, in both directions.
    pub fn bytes_offered(&self) -> u64 {
        self.down.bytes_offered + self.up.bytes_offered
    }

    /// Every byte delivered, in both directions.
    pub fn bytes_delivered(&self) -> u64 {
        self.down.bytes_delivered + self.up.bytes_delivered
    }
}

impl DirectionStats {
    /// Every object lost, in this direction.
    pub fn dropped(&self) -> u64 {
        self.dropped_loss
            + self.dropped_outage
            + self.dropped_too_large
            + self.dropped_queue_overflow
            + self.dropped_handover
    }
}

/// The token bucket: the shape of the link's rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TokenBucket {
    /// The tokens held, in milli-bytes.
    tokens: u64,
    /// The time the bucket was last refilled.
    last_refill_us: u64,
}

impl TokenBucket {
    /// A full bucket.
    fn full(capacity_milli_bytes: u64) -> Self {
        Self {
            tokens: capacity_milli_bytes,
            last_refill_us: 0,
        }
    }
}

/// An object waiting for the bucket's tokens.
#[derive(Debug, Clone)]
struct Queued {
    seq: u64,
    bytes: Vec<u8>,
    offered_us: u64,
    /// The MLP priority the object carries (MLP spec §3.2 rule 2: "the MLP
    /// priority field is the single authority").
    priority: u8,
    /// Whether the object is the echo's carrier (MLP spec §6.3.5): the
    /// object that carries the input sequence the receiver answers.
    echo_critical: bool,
}

/// An object in the pipe.
#[derive(Debug, Clone)]
struct Flight {
    seq: u64,
    bytes: Vec<u8>,
    deliver_at_us: u64,
    duplicate: bool,
}

/// One direction's state.
#[derive(Debug, Clone)]
struct DirectionState {
    profile: DirectionProfile,
    bucket: TokenBucket,
    queue: VecDeque<Queued>,
    queued_bytes: u64,
    in_flight: Vec<Flight>,
    loss: LossState,
    stats: DirectionStats,
}

impl DirectionState {
    /// The object the writer releases next (MLP spec §3.2 rule 2).
    ///
    /// The writer drains by the MLP priority, and within a priority by age.
    /// The **echo's carrier** (MLP spec §6.3.5) is released before the bulk it
    /// would otherwise wait behind: it carries the input sequence the receiver
    /// answers, and the echo's promise is a quarter of a round trip (annex
    /// INV-8). The carrier is not the bulk, and the bulk is the traffic that
    /// may wait.
    fn pick_next(&self) -> Option<usize> {
        self.queue
            .iter()
            .enumerate()
            .min_by_key(|(_, queued)| {
                (
                    // The carrier first: it is a fact the sender cannot wait for.
                    !queued.echo_critical,
                    queued.priority,
                    queued.offered_us,
                    queued.seq,
                )
            })
            .map(|(index, _)| index)
    }

    fn new(profile: DirectionProfile) -> Self {
        let capacity = profile.burst_bytes * MICRO_BYTES;
        Self {
            bucket: TokenBucket::full(capacity),
            profile,
            queue: VecDeque::new(),
            queued_bytes: 0,
            in_flight: Vec::new(),
            loss: LossState::new(),
            stats: DirectionStats::default(),
        }
    }

    /// The rate in micro-bytes per microsecond.
    fn rate_micro_bytes_per_us(&self) -> u64 {
        (self.profile.rate_bps / 8).max(1)
    }

    /// The bucket's capacity in micro-bytes.
    fn capacity_micro_bytes(&self) -> u64 {
        self.profile.burst_bytes * MICRO_BYTES
    }
}

/// The link.
#[derive(Debug, Clone)]
pub struct Link {
    profile: LinkProfile,
    generator: Generator,
    now_us: u64,
    down: DirectionState,
    up: DirectionState,
    events: Vec<LinkEvent>,
    next_seq: u64,
}

impl Link {
    /// A link with a profile.
    pub fn new(profile: LinkProfile) -> Self {
        let generator = Generator::new(profile.seed);
        Self {
            down: DirectionState::new(profile.down.clone()),
            up: DirectionState::new(profile.up.clone()),
            now_us: 0,
            profile,
            generator,
            events: Vec::new(),
            next_seq: 1,
        }
    }

    /// The profile in force.
    pub fn profile(&self) -> &LinkProfile {
        &self.profile
    }

    /// The link's current time, in microseconds.
    pub fn now_us(&self) -> u64 {
        self.now_us
    }

    /// The link's counters.
    pub fn stats(&self) -> LinkStats {
        LinkStats {
            down: self.down.stats,
            up: self.up.stats,
        }
    }

    /// Everything the link has done, in order.
    pub fn events(&self) -> &[LinkEvent] {
        &self.events
    }

    /// Take everything the link has done since the last call.
    pub fn drain_events(&mut self) -> Vec<LinkEvent> {
        std::mem::take(&mut self.events)
    }

    /// How many objects are in the pipe, in both directions.
    pub fn in_flight(&self) -> usize {
        self.down.in_flight.len() + self.up.in_flight.len()
    }

    /// How many bytes are queued, in both directions.
    pub fn queued_bytes(&self) -> u64 {
        self.down.queued_bytes + self.up.queued_bytes
    }

    /// Replace the profile.
    ///
    /// A profile with a different seed restarts the generator's stream; one
    /// with the same seed keeps it, so a scenario can change a link's
    /// conditions without teleporting the random sequence.
    pub fn set_profile(&mut self, profile: LinkProfile) {
        if profile.seed != self.profile.seed {
            self.generator = Generator::new(profile.seed);
        }
        self.profile = profile.clone();
        self.down.profile = profile.down;
        self.up.profile = profile.up;
        self.down.bucket.tokens = self
            .down
            .bucket
            .tokens
            .min(self.down.capacity_micro_bytes());
        self.up.bucket.tokens = self.up.bucket.tokens.min(self.up.capacity_micro_bytes());
        self.events
            .push(LinkEvent::ProfileChanged { at_us: self.now_us });
    }

    /// Offer an object to the link.
    /// Offer one object to the link.
    ///
    /// The priority and the echo-critical mark are the object's own: the link
    /// models the **hop's writer**, and the writer drains by the MLP priority
    /// (MLP spec §3.2 rule 2). An object offered without them is a
    /// media-bulk object at the bulk's priority.
    pub fn offer(&mut self, direction: Direction, bytes: Vec<u8>) -> OfferOutcome {
        self.offer_with(direction, bytes, Priority::MEDIA.get(), false)
    }

    /// Offer one object with the priority the fragment header carries.
    pub fn offer_with(
        &mut self,
        direction: Direction,
        bytes: Vec<u8>,
        priority: u8,
        echo_critical: bool,
    ) -> OfferOutcome {
        let seq = self.next_seq;
        self.next_seq += 1;
        let length = bytes.len() as u64;
        let now_us = self.now_us;
        let mtu_bytes = self.profile.mtu_bytes;
        let outage = self
            .profile
            .outages
            .covering(now_us, direction)
            .map(|window| window.kind);
        let queue_cap_bytes = self.state(direction).profile.queue_cap_bytes;
        let queued_bytes = self.state(direction).queued_bytes;
        let loss_model = self.state(direction).profile.loss;

        {
            let state = self.state_mut(direction);
            state.stats.offered += 1;
            state.stats.bytes_offered += length;
        }

        if length > mtu_bytes {
            self.drop(direction, seq, length, DropReason::TooLarge);
            return OfferOutcome::Dropped {
                seq,
                reason: DropReason::TooLarge,
            };
        }

        if let Some(kind) = outage {
            match kind {
                OutageKind::PartialLoss { permille } => {
                    if self.generator.chance_permille(permille) {
                        self.drop(direction, seq, length, DropReason::Outage);
                        return OfferOutcome::Dropped {
                            seq,
                            reason: DropReason::Outage,
                        };
                    }
                }
                _ => {
                    self.drop(direction, seq, length, DropReason::Outage);
                    return OfferOutcome::Dropped {
                        seq,
                        reason: DropReason::Outage,
                    };
                }
            }
        }

        let occupancy_permille = if queue_cap_bytes == 0 {
            1_000u32
        } else {
            u32::try_from(
                (queued_bytes * 1_000)
                    .checked_div(queue_cap_bytes)
                    .unwrap_or(1_000),
            )
            .unwrap_or(1_000)
        };
        let lost = {
            // The loss state and the generator are disjoint fields, so both
            // can be borrowed at once.
            let (loss_state, generator) = match direction {
                Direction::Down => (&mut self.down.loss, &mut self.generator),
                Direction::Up => (&mut self.up.loss, &mut self.generator),
            };
            loss_state.roll_with(&loss_model, occupancy_permille, generator)
        };
        if lost {
            self.drop(direction, seq, length, DropReason::Loss);
            return OfferOutcome::Dropped {
                seq,
                reason: DropReason::Loss,
            };
        }

        if queued_bytes + length > queue_cap_bytes {
            self.drop(direction, seq, length, DropReason::QueueOverflow);
            return OfferOutcome::Dropped {
                seq,
                reason: DropReason::QueueOverflow,
            };
        }

        self.events.push(LinkEvent::Offered {
            at_us: now_us,
            direction,
            seq,
            bytes: length,
        });
        let state = self.state_mut(direction);
        state.queued_bytes += length;
        state.stats.queue_peak_bytes = state.stats.queue_peak_bytes.max(state.queued_bytes);
        state.queue.push_back(Queued {
            seq,
            bytes,
            offered_us: now_us,
            priority,
            echo_critical,
        });
        OfferOutcome::Queued { seq }
    }

    /// Advance the clock and take everything that arrives by then.
    pub fn advance_to(&mut self, at_us: u64) -> Vec<Delivery> {
        self.now_us = self.now_us.max(at_us);
        let mut deliveries = Vec::new();
        deliveries.extend(self.drain_direction(Direction::Down));
        deliveries.extend(self.drain_direction(Direction::Up));
        deliveries.sort_by_key(|delivery| (delivery.at_us, delivery.seq));
        deliveries
    }

    /// Advance the clock by a number of microseconds.
    pub fn advance(&mut self, micros: u64) -> Vec<Delivery> {
        let target = self.now_us.saturating_add(micros);
        self.advance_to(target)
    }

    /// Change the carrier under the link's feet.
    ///
    /// The objects already in the pipe are split: `split_permille` of them
    /// continue on the old carrier and the rest continue on the new one, and
    /// `loss_permille` of them are lost at the switch. The objects still
    /// queued are released on the new carrier.
    pub fn handover(&mut self, handover: &Handover) {
        let in_flight = self.in_flight();
        let old_down = self.down.profile.clone();
        let old_up = self.up.profile.clone();
        self.set_profile(handover.profile.clone());
        self.events.push(LinkEvent::Handover {
            at_us: self.now_us,
            split_permille: handover.split_permille,
            loss_permille: handover.loss_permille,
            in_flight,
        });
        self.split_flight(Direction::Down, &old_down, handover);
        self.split_flight(Direction::Up, &old_up, handover);
    }

    /// The direction's state.
    fn state(&self, direction: Direction) -> &DirectionState {
        match direction {
            Direction::Down => &self.down,
            Direction::Up => &self.up,
        }
    }

    /// The direction's state.
    fn state_mut(&mut self, direction: Direction) -> &mut DirectionState {
        match direction {
            Direction::Down => &mut self.down,
            Direction::Up => &mut self.up,
        }
    }

    /// Record a drop and count it.
    fn drop(&mut self, direction: Direction, seq: u64, bytes: u64, reason: DropReason) {
        let state = self.state_mut(direction);
        match reason {
            DropReason::TooLarge => state.stats.dropped_too_large += 1,
            DropReason::Loss => state.stats.dropped_loss += 1,
            DropReason::Outage => state.stats.dropped_outage += 1,
            DropReason::QueueOverflow => state.stats.dropped_queue_overflow += 1,
            DropReason::Handover => state.stats.dropped_handover += 1,
        }
        self.events.push(LinkEvent::Dropped {
            at_us: self.now_us,
            direction,
            seq,
            bytes,
            reason,
        });
    }

    /// Refill a direction's bucket and release whatever it can carry.
    fn drain_direction(&mut self, direction: Direction) -> Vec<Delivery> {
        let now = self.now_us;
        self.release_ready(direction, now);
        let mut due: Vec<Flight> = Vec::new();
        let mut remaining: Vec<Flight> =
            Vec::with_capacity(self.state_mut(direction).in_flight.len());
        {
            let state = self.state_mut(direction);
            for flight in state.in_flight.drain(..) {
                if flight.deliver_at_us <= now {
                    due.push(flight);
                } else {
                    remaining.push(flight);
                }
            }
            state.in_flight = remaining;
        }
        let mut deliveries = Vec::with_capacity(due.len());
        for flight in due {
            // An outage that starts while the object is in the pipe takes it.
            if let Some(window) = self
                .profile
                .outages
                .covering(flight.deliver_at_us, direction)
            {
                match window.kind {
                    OutageKind::PartialLoss { permille } => {
                        if self.generator.chance_permille(permille) {
                            self.drop(
                                direction,
                                flight.seq,
                                flight.bytes.len() as u64,
                                DropReason::Outage,
                            );
                            continue;
                        }
                    }
                    _ => {
                        self.drop(
                            direction,
                            flight.seq,
                            flight.bytes.len() as u64,
                            DropReason::Outage,
                        );
                        continue;
                    }
                }
            }
            {
                let state = self.state_mut(direction);
                state.stats.delivered += 1;
                state.stats.bytes_delivered += flight.bytes.len() as u64;
                if flight.duplicate {
                    state.stats.duplicated += 1;
                }
            }
            self.events.push(LinkEvent::Delivered {
                at_us: flight.deliver_at_us,
                direction,
                seq: flight.seq,
                bytes: flight.bytes.len() as u64,
                duplicate: flight.duplicate,
            });
            deliveries.push(Delivery {
                direction,
                bytes: flight.bytes,
                at_us: flight.deliver_at_us,
                seq: flight.seq,
                duplicate: flight.duplicate,
            });
        }
        deliveries
    }

    /// Release the objects the bucket can carry, at the exact time it can
    /// carry them.
    fn release_ready(&mut self, direction: Direction, now_us: u64) {
        loop {
            let (front, release_at_us) = {
                let state = self.state_mut(direction);
                let Some(index) = state.pick_next() else {
                    break;
                };
                let front = state.queue[index].clone();
                let need = (front.bytes.len() as u64) * MICRO_BYTES;
                let rate = state.rate_micro_bytes_per_us();
                if rate == 0 {
                    break;
                }
                let elapsed = now_us.saturating_sub(state.bucket.last_refill_us);
                let available = state.bucket.tokens + rate * elapsed;
                if available < need {
                    break;
                }
                // The exact moment the bucket holds enough: no rounding, and
                // independent of how often the driver polls the link. The
                // object cannot leave before it was offered.
                let needed_at = if state.bucket.tokens >= need {
                    state.bucket.last_refill_us
                } else {
                    state.bucket.last_refill_us + (need - state.bucket.tokens).div_ceil(rate)
                };
                let release_at_us = needed_at.max(front.offered_us);
                state.queue.remove(index);
                state.queued_bytes -= front.bytes.len() as u64;
                let tokens_at_release =
                    state.bucket.tokens + rate * (release_at_us - state.bucket.last_refill_us);
                state.bucket.tokens = tokens_at_release.saturating_sub(need);
                state.bucket.last_refill_us = release_at_us;
                (front, release_at_us)
            };

            let mut flights = self.build_flights(direction, &front, release_at_us);
            let state = self.state_mut(direction);
            state.stats.queue_delay_us = state
                .stats
                .queue_delay_us
                .saturating_add(release_at_us.saturating_sub(front.offered_us));
            state.in_flight.append(&mut flights);
            state.stats.in_flight_peak = state.stats.in_flight_peak.max(state.in_flight.len());
            self.events.push(LinkEvent::Released {
                at_us: release_at_us,
                direction,
                seq: front.seq,
                bytes: front.bytes.len() as u64,
            });
        }
    }

    /// Build the flights of a released object: its delay, its jitter, its
    /// possible reordering, and its possible duplicate.
    fn build_flights(
        &mut self,
        direction: Direction,
        queued: &Queued,
        released_at_us: u64,
    ) -> Vec<Flight> {
        let profile = match direction {
            Direction::Down => self.down.profile.clone(),
            Direction::Up => self.up.profile.clone(),
        };
        let jitter_us = profile.jitter.sample_us(&mut self.generator);
        let mut deliver_at_us =
            released_at_us + profile.propagation_us + jitter_us + profile.extra_delay_us;
        if self.generator.chance_permille(profile.reorder_permille) {
            let displacement = self.generator.range_us(1, profile.reorder_window_us);
            deliver_at_us = deliver_at_us.saturating_add(displacement);
            self.state_mut(direction).stats.reordered += 1;
        }
        let mut flights = vec![Flight {
            seq: queued.seq,
            bytes: queued.bytes.clone(),
            deliver_at_us,
            duplicate: false,
        }];
        if self.generator.chance_permille(profile.duplicate_permille) {
            self.state_mut(direction).stats.duplicated += 1;
            flights.push(Flight {
                seq: queued.seq,
                bytes: queued.bytes.clone(),
                deliver_at_us: deliver_at_us.saturating_add(profile.duplicate_delay_us),
                duplicate: true,
            });
        }
        flights
    }

    /// Split the objects in flight between the old carrier and the new one.
    fn split_flight(
        &mut self,
        direction: Direction,
        old_profile: &DirectionProfile,
        handover: &Handover,
    ) {
        let flights = std::mem::take(&mut self.state_mut(direction).in_flight);
        for flight in flights {
            if self.generator.chance_permille(handover.loss_permille) {
                self.drop(
                    direction,
                    flight.seq,
                    flight.bytes.len() as u64,
                    DropReason::Handover,
                );
                continue;
            }
            let stays = self.generator.chance_permille(handover.split_permille);
            let profile = if stays {
                old_profile.clone()
            } else {
                match direction {
                    Direction::Down => self.down.profile.clone(),
                    Direction::Up => self.up.profile.clone(),
                }
            };
            let jitter_us = profile.jitter.sample_us(&mut self.generator);
            let mut flight = flight;
            flight.deliver_at_us = self
                .now_us
                .saturating_add(profile.propagation_us)
                .saturating_add(jitter_us)
                .saturating_add(profile.extra_delay_us);
            self.state_mut(direction).in_flight.push(flight);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> LinkProfile {
        LinkProfile::perfect()
    }

    #[test]
    fn a_perfect_link_delivers_everything_in_order() {
        let mut link = Link::new(profile());
        link.offer(Direction::Down, vec![1; 100]);
        link.offer(Direction::Down, vec![2; 100]);
        let deliveries = link.advance_to(1_000_000);
        assert_eq!(deliveries.len(), 2);
        assert_eq!(deliveries[0].bytes, vec![1; 100]);
        assert_eq!(deliveries[1].bytes, vec![2; 100]);
        assert_eq!(link.stats().dropped(), 0);
    }

    #[test]
    fn the_link_delays_by_the_propagation_delay() {
        let mut profile = profile();
        profile.down.propagation_us = 50_000;
        profile.down.jitter = Jitter::none();
        let mut link = Link::new(profile);
        link.offer(Direction::Down, vec![1; 10]);
        assert!(link.advance_to(49_000).is_empty());
        let deliveries = link.advance_to(50_000);
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].at_us, 50_000);
    }

    #[test]
    fn the_token_bucket_shapes_a_burst() {
        let mut profile = profile();
        profile.down.rate_bps = 8_000; // 1 000 bytes per second
        profile.down.burst_bytes = 1_000;
        profile.down.queue_cap_bytes = 10_000;
        profile.down.propagation_us = 0;
        profile.down.jitter = Jitter::none();
        let mut link = Link::new(profile);
        // Ten objects of 1 000 bytes each: the first passes on the bucket's
        // initial tokens, the rest wait for the rate.
        for index in 0..10u8 {
            link.offer(Direction::Down, vec![index; 1_000]);
        }
        let deliveries = link.advance_to(20_000_000);
        assert_eq!(deliveries.len(), 10);
        // The first is immediate, the last waits for nine seconds of rate.
        assert_eq!(deliveries[0].at_us, 0);
        assert!(
            deliveries[9].at_us >= 9_000_000,
            "last at {}",
            deliveries[9].at_us
        );
    }

    #[test]
    fn a_full_queue_drops_the_object() {
        let mut profile = profile();
        profile.down.rate_bps = 8_000;
        profile.down.burst_bytes = 1_000;
        profile.down.queue_cap_bytes = 2_000;
        let mut link = Link::new(profile);
        link.offer(Direction::Down, vec![0; 1_000]);
        link.offer(Direction::Down, vec![1; 1_000]);
        let outcome = link.offer(Direction::Down, vec![2; 1_000]);
        assert_eq!(
            outcome,
            OfferOutcome::Dropped {
                seq: 3,
                reason: DropReason::QueueOverflow
            }
        );
        assert_eq!(link.stats().down.dropped_queue_overflow, 1);
    }

    #[test]
    fn an_oversize_object_is_dropped() {
        let mut profile = profile();
        profile.mtu_bytes = 100;
        let mut link = Link::new(profile);
        let outcome = link.offer(Direction::Down, vec![0; 101]);
        assert_eq!(
            outcome,
            OfferOutcome::Dropped {
                seq: 1,
                reason: DropReason::TooLarge
            }
        );
    }

    #[test]
    fn a_blackhole_takes_what_it_covers() {
        let mut profile = profile();
        profile.outages = OutageSchedule::of(vec![OutageWindow::new(
            100_000,
            100_000,
            OutageKind::Blackhole,
        )]);
        let mut link = Link::new(profile);
        link.advance_to(150_000);
        let outcome = link.offer(Direction::Down, vec![0; 8]);
        assert!(matches!(outcome, OfferOutcome::Dropped { .. }));
        link.advance_to(300_000);
        link.offer(Direction::Down, vec![1; 8]);
        assert_eq!(link.advance_to(400_000).len(), 1);
    }

    #[test]
    fn a_one_way_blackhole_spares_the_other_direction() {
        let mut profile = profile();
        profile.outages = OutageSchedule::of(vec![OutageWindow::new(
            0,
            1_000_000,
            OutageKind::OneWayBlackhole {
                direction: Direction::Up,
            },
        )]);
        let mut link = Link::new(profile);
        assert!(matches!(
            link.offer(Direction::Up, vec![0; 8]),
            OfferOutcome::Dropped { .. }
        ));
        assert!(matches!(
            link.offer(Direction::Down, vec![0; 8]),
            OfferOutcome::Queued { .. }
        ));
    }

    #[test]
    fn a_handover_splits_the_objects_in_flight() {
        let mut profile = profile();
        profile.down.propagation_us = 1_000_000;
        profile.down.jitter = Jitter::none();
        let mut link = Link::new(profile.clone());
        for _ in 0..10 {
            link.offer(Direction::Down, vec![0; 8]);
        }
        link.advance_to(1_000);
        assert_eq!(link.in_flight(), 10);
        let mut next = profile.clone();
        next.down.propagation_us = 1_000;
        link.handover(&Handover {
            profile: next,
            split_permille: 500,
            loss_permille: 200,
        });
        assert_eq!(link.stats().down.dropped_handover, 2);
        // The survivors are still in the pipe.
        assert_eq!(link.in_flight(), 8);
    }

    #[test]
    fn the_same_seed_gives_the_same_run() {
        let mut profile = profile();
        profile.down.jitter = Jitter::normal(5_000);
        profile.down.loss = LossModel::Uniform { permille: 100 };
        profile.down.duplicate_permille = 50;
        profile.down.reorder_permille = 50;
        let run = || {
            let mut link = Link::new(profile.clone());
            for index in 0..50u8 {
                link.offer(Direction::Down, vec![index; 16]);
            }
            let deliveries = link.advance_to(10_000_000);
            (
                deliveries
                    .iter()
                    .map(|delivery| (delivery.seq, delivery.at_us, delivery.duplicate))
                    .collect::<Vec<_>>(),
                link.stats(),
                link.events().to_vec(),
            )
        };
        let first = run();
        let second = run();
        assert_eq!(first.0, second.0);
        assert_eq!(first.1, second.1);
        assert_eq!(first.2, second.2);
    }
}
