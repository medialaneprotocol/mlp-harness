//! The trace: one format a human can read and a machine can diff.
//!
//! A trace records five things:
//!
//! * the **scenario** that produced it;
//! * the **link's events** — what was offered, what was released, what
//!   arrived, what was lost and why;
//! * the **decisions** — the updates the sender emitted, the
//!   acknowledgements, the plans, the lane states, the inputs and the
//!   echoes;
//! * the **invariant outcomes**;
//! * a **digest** over the decisions.
//!
//! # The digest
//!
//! The digest covers the decisions and nothing else. It is a *change
//! detector*, not a commitment: it answers "did the protocol decide
//! something different?", and it deliberately ignores the timings, because
//! the timings depend on the link's profile while the decisions do not. Two
//! runs of the same scenario over the same profile produce the same digest
//! and byte-identical traces; a run over a different profile produces a
//! different trace and, if the protocol decided differently, a different
//! digest.
//!
//! The digest is FNV-1a over 64 bits, written as sixteen lowercase hex
//! digits — the same shape the protocol's corpus uses, so that a corpus
//! document's digest field keeps its meaning across the switch.

use serde::{Deserialize, Serialize};

use crate::invariants::{Invariant, Outcome};
use crate::link::{DirectionStats, DropReason, LinkEvent, LinkProfile, Tier};

/// The trace format's version.
pub const TRACE_SCHEMA: u32 = 1;

/// One update the sender emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateDecision {
    /// When, in microseconds.
    pub at_us: u64,
    /// The update's id.
    pub update_id: u64,
    /// The update's kind.
    pub kind: String,
    /// How many cells it covers.
    pub cells: u32,
    /// How many bytes it carries.
    pub bytes: u32,
    /// Whether it may apply onto any held state.
    pub anchor: bool,
    /// The plan it belongs to, when a plan was active.
    pub plan_id: Option<u32>,
    /// The highest input sequence the update's content reflects.
    pub reflects_input_seq: u64,
}

/// One acknowledgement the receiver produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckDecision {
    /// When, in microseconds.
    pub at_us: u64,
    /// The highest update id the receiver had applied.
    pub highest_applied_update_id: u64,
    /// The cells the sender still owed the receiver.
    pub pending_cells: u32,
    /// Whether the sender's accounting reported convergence.
    pub converged: bool,
}

/// One plan event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDecision {
    /// When, in microseconds.
    pub at_us: u64,
    /// The plan's id.
    pub plan_id: u32,
    /// What happened.
    pub event: String,
    /// How many cells it covers.
    pub cells: u32,
}

/// One lane-state change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDecision {
    /// When, in microseconds.
    pub at_us: u64,
    /// The new state.
    pub state: String,
}

/// One input the peer was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDecision {
    /// When, in microseconds.
    pub at_us: u64,
    /// The input's sequence number.
    pub seq: u64,
    /// Its kind.
    pub kind: String,
    /// Whether it was applied.
    pub applied: bool,
    /// Whether it expired before it was applied.
    pub expired: bool,
}

/// One measured echo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoSample {
    /// The input sequence the echo answers.
    pub input_seq: u64,
    /// The echo, in milliseconds.
    pub echo_ms: u64,
    /// When the echo was measured, in microseconds.
    pub at_us: u64,
}

/// The decisions a run produced.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Decisions {
    /// The updates the sender emitted.
    pub updates: Vec<UpdateDecision>,
    /// The acknowledgements.
    pub acks: Vec<AckDecision>,
    /// The plan events.
    pub plans: Vec<PlanDecision>,
    /// The lane-state changes.
    pub states: Vec<StateDecision>,
    /// The inputs.
    pub inputs: Vec<InputDecision>,
    /// The echoes, measured the way the receiver sees them.
    pub echoes: Vec<EchoSample>,
    /// The echoes, measured the way a **sender** sees them: from the sender's
    /// application of the input to the arrival of the acknowledgement that
    /// names it. This is the measurement a deployment makes
    /// (`rm_host_media_echo_seconds`), and the echo's promise (annex INV-8) is
    /// checked on it.
    pub sender_echoes: Vec<EchoSample>,
}

/// One thing the harness saw that an invariant forbids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// When, in microseconds.
    pub at_us: u64,
    /// The invariant.
    pub invariant: Invariant,
    /// What the harness saw.
    pub detail: String,
}

/// The measurements a run produced.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Measurements {
    /// The echo's median, in milliseconds.
    pub echo_p50_ms: Option<u64>,
    /// The echo's 95th percentile, in milliseconds.
    pub echo_p95_ms: Option<u64>,
    /// The echo's largest value, in milliseconds.
    pub echo_max_ms: Option<u64>,
    /// How many echoes were measured.
    pub echo_samples: u64,
    /// The **sender-side** echo's median, in milliseconds: the round trip the
    /// sender measures (the input's application to the acknowledgement's
    /// arrival). This is the measurement a deployment makes, and the echo's
    /// promise (annex INV-8) is checked on it.
    pub sender_echo_p50_ms: Option<u64>,
    /// The sender-side echo's 95th percentile, in milliseconds.
    pub sender_echo_p95_ms: Option<u64>,
    /// The sender-side echo's largest value, in milliseconds.
    pub sender_echo_max_ms: Option<u64>,
    /// How many sender-side echoes were measured.
    pub sender_echo_samples: u64,
    /// The time from the last capture change to convergence, in
    /// milliseconds.
    pub convergence_ms: Option<u64>,
    /// How many times the session converged.
    pub convergences: u64,
    /// The bytes the sender put on the link.
    pub bytes_down: u64,
    /// The bytes the receiver put on the link.
    pub bytes_up: u64,
    /// The objects the receiver applied.
    pub updates_applied: u64,
    /// The updates the sender emitted.
    pub updates_sent: u64,
    /// The objects that arrived at the receiver.
    pub objects_delivered: u64,
    /// The objects the link lost.
    pub objects_dropped: u64,
    /// The media updates the receiver reported as lost.
    ///
    /// The receiver's loss is counted in the **update-id** space (MLP spec
    /// §5.2): the media updates it never saw. It mirrors the protocol's
    /// `lost_objects` field, whose space the specification states.
    pub objects_lost: u64,
    /// The sender's measured round trip at the end, in milliseconds.
    pub rtt_ms: f64,
    /// The sender's pace at the end, in bits per second.
    pub pace_bps: u64,
    /// The sender's target frame rate at the end, in frames per second.
    pub target_fps: f64,
    /// The feedback reports the sender received.
    pub feedbacks_received: u64,
    /// The acknowledgements the sender received.
    pub acks_received: u64,
    /// The input envelopes the sender received.
    pub inputs_received: u64,
    /// The objects the sender accepted into its queue.
    pub objects_enqueued: u64,
    /// The fragments the sender released.
    pub fragments_sent: u64,
    /// The objects the sender dropped when its release queue hit its bound.
    pub queue_overflow_drops: u64,
    /// The objects the sender force-freed after the in-flight wait.
    pub in_flight_evictions_total: u64,
    /// The updates the sender's write queue refused (MLP spec §3.2 rule 4).
    pub in_flight_deferred_total: u64,
    /// The echo's carriers that took a queued object's place instead of being
    /// refused by the write queue (MLP spec §6.3.5).
    pub echo_critical_replacements_total: u64,
    /// The objects the sender dropped because a newer plan covered them.
    pub superseded: u64,
    /// The objects the sender dropped for exceeding the object cap.
    pub oversize_total: u64,
    /// The probes the sender sent.
    pub probes_sent: u64,
    /// The objects the receiver counted as reordered.
    pub receiver_reordered: u64,
    /// The objects the receiver counted as duplicated.
    pub receiver_duplicated: u64,
    /// The objects the receiver dropped in assembly.
    pub receiver_dropped_objects: u64,
    /// How many cells the sender still owed the receiver at the end.
    ///
    /// The convergence promise is stated in cells (MLP spec §2.1), so the
    /// residue is a measurement like any other.
    pub pending_cells: u32,
    /// How many cells the surface carries.
    pub cells: u32,
    /// The scenario's length, in milliseconds.
    pub duration_ms: u64,
}

/// The link's half of the trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkSummary {
    /// The profile in force at the end.
    pub profile: LinkProfile,
    /// The sender → receiver counters.
    pub down: DirectionStats,
    /// The receiver → sender counters.
    pub up: DirectionStats,
    /// Everything the link did, in order.
    pub events: Vec<LinkEvent>,
}

/// The scenario's summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioSummary {
    /// The scenario's name.
    pub name: String,
    /// The tier it targeted.
    pub tier: Tier,
    /// Its seed.
    pub seed: u64,
    /// How many steps it had.
    pub steps: usize,
    /// Its length, in milliseconds.
    pub duration_ms: u64,
    /// The faults the peer carried.
    pub faults: Vec<String>,
}

/// The trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// The trace format's version.
    pub schema: u32,
    /// The scenario.
    pub scenario: ScenarioSummary,
    /// The link.
    pub link: LinkSummary,
    /// The decisions.
    pub decisions: Decisions,
    /// The invariant outcomes.
    pub invariants: Vec<Outcome>,
    /// Everything the harness saw that an invariant forbids.
    pub violations: Vec<Violation>,
    /// The measurements.
    pub measurements: Measurements,
    /// The digest over the decisions.
    pub digest: String,
}

impl Trace {
    /// Whether the run was clean.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }

    /// The trace as a JSON document.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a trace is serializable")
    }

    /// The digest over the decisions.
    ///
    /// FNV-1a over 64 bits, written as sixteen lowercase hex digits. The
    /// timings are deliberately not part of it.
    pub fn compute_digest(decisions: &Decisions) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for update in &decisions.updates {
            mix(format!(
                "u:{}:{}:{}:{};",
                update.update_id, update.kind, update.cells, update.anchor
            )
            .as_bytes());
        }
        for ack in &decisions.acks {
            mix(format!("a:{};", ack.highest_applied_update_id).as_bytes());
        }
        for plan in &decisions.plans {
            mix(format!("p:{}:{};", plan.plan_id, plan.event).as_bytes());
        }
        for state in &decisions.states {
            mix(format!("s:{};", state.state).as_bytes());
        }
        for input in &decisions.inputs {
            mix(format!(
                "i:{}:{}:{}:{};",
                input.seq, input.kind, input.applied, input.expired
            )
            .as_bytes());
        }
        for echo in &decisions.echoes {
            mix(format!("e:{};", echo.input_seq).as_bytes());
        }
        format!("{hash:016x}")
    }

    /// How many objects the link lost, by reason.
    pub fn drops_by_reason(&self) -> Vec<(DropReason, u64)> {
        let mut counts: Vec<(DropReason, u64)> = Vec::new();
        for event in &self.link.events {
            if let LinkEvent::Dropped { reason, .. } = event {
                match counts.iter_mut().find(|(known, _)| *known == *reason) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((*reason, 1)),
                }
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decisions() -> Decisions {
        Decisions {
            updates: vec![UpdateDecision {
                at_us: 1_000,
                update_id: 1,
                kind: "anchor".to_string(),
                cells: 4,
                bytes: 128,
                anchor: true,
                plan_id: None,
                reflects_input_seq: 0,
            }],
            acks: vec![AckDecision {
                at_us: 2_000,
                highest_applied_update_id: 1,
                pending_cells: 0,
                converged: true,
            }],
            ..Decisions::default()
        }
    }

    #[test]
    fn the_digest_ignores_the_timings() {
        let first = decisions();
        let mut second = decisions();
        second.updates[0].at_us = 999_999;
        second.acks[0].at_us = 888_888;
        assert_eq!(
            Trace::compute_digest(&first),
            Trace::compute_digest(&second)
        );
    }

    #[test]
    fn the_digest_notices_a_different_decision() {
        let first = decisions();
        let mut second = decisions();
        second.updates[0].cells = 5;
        assert_ne!(
            Trace::compute_digest(&first),
            Trace::compute_digest(&second)
        );
    }

    #[test]
    fn the_digest_is_sixteen_lowercase_hex_digits() {
        let digest = Trace::compute_digest(&decisions());
        assert_eq!(digest.len(), 16);
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn the_digest_is_stable() {
        assert_eq!(
            Trace::compute_digest(&decisions()),
            Trace::compute_digest(&decisions())
        );
    }
}
