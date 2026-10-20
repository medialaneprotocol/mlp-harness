//! The Media Lane Protocol's verification layer.
//!
//! MLP is a transport for real-time interactive media over links that are
//! slow, lossy, or both. This crate is the transport's **verification
//! layer**: a deterministic, sans-IO network harness that anyone can drive,
//! and that answers questions.
//!
//! # What it is
//!
//! * a **link model** that is a network, not a delay: a token bucket and a
//!   queue, a propagation delay with a jitter distribution, four loss
//!   models, reordering, duplication, an MTU, scheduled outages, two
//!   separate directions, and a carrier handover that costs what a real one
//!   costs;
//! * a **scenario model**: a program against a virtual clock — advance the
//!   clock, offer an object, change the profile, hand over the carrier,
//!   assert something;
//! * the protocol's **real machines**, driven over that link: the sender and
//!   the receiver of [`mlp_core`], byte for byte, with the protocol's own
//!   fragments, acknowledgements, feedback reports, probes and input
//!   envelopes;
//! * the protocol's **invariants**, named, versioned, stated in one place
//!   and checked;
//! * a **trace** a human can read and a machine can diff;
//! * a **property suite** that shrinks a failing case to a counterexample;
//! * a **prediction interface**: given a tier's profile and a scenario,
//!   what should the receiver's echo p95 be?;
//! * a **sans-IO driver** for a consumer's own machines, so that a
//!   consumer's state machines are driven the way the protocol's are.
//!
//! # What it is not
//!
//! It is not a second implementation of the protocol. The harness never
//! decides what an update means, never assembles an object, never paces a
//! byte: it drives the protocol's machines and watches what they decide.
//!
//! # Determinism
//!
//! A run reads nothing: no wall clock, no randomness, no sockets, no
//! filesystem, no environment. Time is injected; randomness comes from a
//! seeded generator the scenario carries. A scenario that runs twice runs
//! identically, on any machine, in any order, in any process — and its
//! trace is byte-identical.
//!
//! The origin the machines are handed comes from the process, once, and it
//! is a *base* rather than a reading: it never appears in a trace, it never
//! reaches a decision, and every number the harness reports is a difference
//! from it.
//!
//! # Example
//!
//! ```
//! use mlp_harness::link::Tier;
//! use mlp_harness::predict;
//!
//! let prediction = predict::predict_tier(Tier::T2);
//! assert_eq!(prediction.tier, Tier::T2);
//! assert_eq!(prediction.echo_p95_budget_ms, 550);
//! ```

pub mod checks;
pub mod clock;
pub mod driver;
pub mod faults;
pub mod invariants;
pub mod link;
pub mod predict;
pub mod property;
pub mod rng;
pub mod sans_io;
pub mod scenario;
pub mod tiers;
pub mod trace;
pub mod wire;

pub use clock::{VirtualClock, epoch};
pub use driver::{Driver, carrier_for};
pub use faults::Faults;
pub use invariants::{DEFINITIONS, Definition, Invariant, Outcome};
pub use link::{
    Direction, DropReason, Handover, Jitter, Link, LinkProfile, LossModel, Tier, TierBand,
};
pub use predict::{Prediction, canonical_scenario, predict, predict_tier};
pub use sans_io::{GoldenTrace, TestHarness};
pub use scenario::{GridSpec, InputEvent, Region, Scenario, Step};
pub use tiers::{TierRow, TierTable, table as tier_table};
pub use trace::Trace;

/// Run a scenario and return its trace.
pub fn run(scenario: &Scenario) -> Trace {
    Driver::run(scenario)
}
