//! The prediction interface: the harness answers questions.
//!
//! The harness is not only a test rig. It is a **prediction engine**: given
//! a link's profile and a scenario, it says what the session will do —
//! what the receiver's echo p95 will be, when the session will converge,
//! how many bytes will cross.
//!
//! A consumer that deploys the transport has live measurements; the
//! prediction is what those measurements are *checked against*. The
//! dependency runs one way: the consumer depends on the harness, never the
//! other way round.
//!
//! # Two numbers, not one
//!
//! Every prediction carries the **model's** answer and the **promise** it
//! is measured against:
//!
//! * `echo_p95_ms` — what the model says the echo's p95 will be;
//! * `echo_p95_budget_ms` — what the tier promises (`2 × RTT + 250 ms`).
//!
//! A consumer compares the two against what it measured. A model that
//! disagreed with the promise would be a model nobody could use, so the two
//! are always reported together.
//!
//! # The canonical scenario
//!
//! A prediction needs a scenario: the same link with different work gives
//! different answers. The harness ships one **canonical scenario per tier**
//! — the story the tiers are written about: a capture change, a short
//! burst of typing, and a settle — so that "what should T2's echo p95 be?"
//! has one answer.

use serde::{Deserialize, Serialize};

use crate::driver::Driver;
use crate::link::{LinkProfile, Tier};
use crate::scenario::{InputEvent, Region, Scenario};
use crate::trace::Trace;

/// The harness's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    /// The tier the prediction is about.
    pub tier: Tier,
    /// The scenario's name.
    pub scenario: String,
    /// The scenario's seed.
    pub seed: u64,
    /// What the model says the echo's p50 will be, in milliseconds.
    pub echo_p50_ms: Option<u64>,
    /// What the model says the echo's p95 will be, in milliseconds.
    pub echo_p95_ms: Option<u64>,
    /// What the model says the echo's largest value will be, in
    /// milliseconds.
    pub echo_max_ms: Option<u64>,
    /// How many echoes the model measured.
    pub echo_samples: u64,
    /// What the tier promises for the echo's p95, in milliseconds.
    pub echo_p95_budget_ms: u64,
    /// Whether the model's answer is inside the promise.
    pub echo_within_promise: bool,
    /// What the model says the time to convergence will be, in
    /// milliseconds.
    pub convergence_ms: Option<u64>,
    /// How many times the model saw the session converge.
    pub convergences: u64,
    /// What the model says will cross the downlink, in bytes.
    pub bytes_down: u64,
    /// What the model says will cross the uplink, in bytes.
    pub bytes_up: u64,
    /// How many objects the model says will be delivered.
    pub objects_delivered: u64,
    /// How many objects the model says the link will lose.
    pub objects_dropped: u64,
    /// The sender's measured round trip at the end, in milliseconds.
    pub rtt_ms: f64,
    /// The pace the sender ends at, in bits per second.
    pub pace_bps: u64,
    /// The trace's digest: the fingerprint of the decisions.
    pub digest: String,
    /// Whether the run was clean.
    pub clean: bool,
}

impl Prediction {
    /// A prediction from a trace.
    pub fn from_trace(trace: &Trace) -> Self {
        let budget = trace.scenario.tier.echo_budget_ms();
        let echo_p95_ms = trace.measurements.echo_p95_ms;
        let echo_within_promise = match echo_p95_ms {
            Some(value) => value <= budget,
            None => true,
        };
        Self {
            tier: trace.scenario.tier,
            scenario: trace.scenario.name.clone(),
            seed: trace.scenario.seed,
            echo_p50_ms: trace.measurements.echo_p50_ms,
            echo_p95_ms,
            echo_max_ms: trace.measurements.echo_max_ms,
            echo_samples: trace.measurements.echo_samples,
            echo_p95_budget_ms: budget,
            echo_within_promise,
            convergence_ms: trace.measurements.convergence_ms,
            convergences: trace.measurements.convergences,
            bytes_down: trace.measurements.bytes_down,
            bytes_up: trace.measurements.bytes_up,
            objects_delivered: trace.measurements.objects_delivered,
            objects_dropped: trace.measurements.objects_dropped,
            rtt_ms: trace.measurements.rtt_ms,
            pace_bps: trace.measurements.pace_bps,
            digest: trace.digest.clone(),
            clean: trace.is_clean(),
        }
    }

    /// The prediction as a JSON document.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a prediction is serializable")
    }
}

/// The canonical scenario for a tier.
///
/// The story is the one the tiers are written about: the link settles, the
/// capture changes, the user types a few characters, and the session
/// settles again. Every phase is scaled by the tier's round trip, because a
/// scenario that is too short for the tier measures the clock rather than
/// the protocol.
pub fn canonical_scenario(tier: Tier) -> Scenario {
    let mut scenario = Scenario::new(format!("canonical-{}", tier.as_str()), tier);
    scenario.link = LinkProfile::tier(tier);
    let rtt_ms = tier.rtt_ms();
    scenario
        .steps
        .push(crate::scenario::Step::Advance { ms: 2 * rtt_ms });
    scenario.steps.push(crate::scenario::Step::CaptureChange {
        region: Region::new(320, 240, 640, 200),
    });
    scenario
        .steps
        .push(crate::scenario::Step::Advance { ms: rtt_ms });
    for code in ["KeyH", "KeyE", "KeyL", "KeyO", "KeyW", "KeyO"] {
        scenario.steps.push(crate::scenario::Step::ViewerInput {
            event: Box::new(InputEvent::Key {
                code: code.to_string(),
                pressed: true,
            }),
        });
        scenario.steps.push(crate::scenario::Step::ViewerInput {
            event: Box::new(InputEvent::Key {
                code: code.to_string(),
                pressed: false,
            }),
        });
        scenario.steps.push(crate::scenario::Step::Advance {
            ms: (rtt_ms / 2).max(50),
        });
    }
    // The settle phase is long enough for the slowest tier: the echo's
    // round trip is two one-way delays plus the jitter's tail.
    let settle_ms = (4 * rtt_ms).max(2_000) + 2_000;
    scenario
        .steps
        .push(crate::scenario::Step::Advance { ms: settle_ms });
    scenario
}

/// Predict what a tier's canonical scenario does.
pub fn predict_tier(tier: Tier) -> Prediction {
    let scenario = canonical_scenario(tier);
    let trace = Driver::run(&scenario);
    Prediction::from_trace(&trace)
}

/// Predict what a scenario does.
pub fn predict(scenario: &Scenario) -> Prediction {
    let trace = Driver::run(scenario);
    Prediction::from_trace(&trace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prediction_carries_the_model_and_the_promise() {
        let prediction = predict_tier(Tier::T0);
        assert_eq!(prediction.tier, Tier::T0);
        assert_eq!(prediction.echo_p95_budget_ms, 270);
    }

    #[test]
    fn a_prediction_is_stable() {
        let first = predict_tier(Tier::T0);
        let second = predict_tier(Tier::T0);
        assert_eq!(first, second);
    }

    #[test]
    fn the_canonical_scenario_types() {
        let scenario = canonical_scenario(Tier::T0);
        let typing = scenario
            .steps
            .iter()
            .filter(|step| matches!(step, crate::scenario::Step::ViewerInput { .. }))
            .count();
        assert_eq!(typing, 12);
    }
}
