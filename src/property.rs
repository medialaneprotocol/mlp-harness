//! The property suite: randomised profiles, and a counterexample that is
//! small enough to read.
//!
//! A property suite asks the same question of many random cases: *do the
//! invariants hold?* The cases are drawn from a seeded generator, so a
//! failing case is reproducible from its seed alone.
//!
//! # Shrinking
//!
//! A counterexample that takes four minutes to reproduce and has two
//! thousand steps in it is not a counterexample anybody acts on. When a case
//! fails, the suite **shrinks** it: it removes steps, shortens durations,
//! and simplifies profiles, keeping only the changes that preserve the
//! failure. The result is the smallest case the suite can find, printed as
//! a scenario document.
//!
//! # What is randomised
//!
//! * the **profile**: the rates, the delays, the jitter, the loss model and
//!   its parameters, the reordering, the duplication, the MTU;
//! * the **work**: where the capture changes, how much, and when;
//! * the **outages**: when they start, how long they last, and what they
//!   do;
//! * the **handovers**: when the carrier changes and what it costs.

use serde::{Deserialize, Serialize};

use crate::driver::Driver;
use crate::link::{
    Direction, Handover, Jitter, LinkProfile, LossModel, OutageKind, OutageSchedule, OutageWindow,
    Tier,
};
use crate::rng::Generator;
use crate::scenario::{InputEvent, Region, Scenario, Step};

/// A randomised case and its outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    /// The case's seed.
    pub seed: u64,
    /// The scenario.
    pub scenario: Scenario,
    /// Whether the run was clean.
    pub clean: bool,
    /// The invariants that were violated.
    pub violated: Vec<String>,
    /// The trace's digest.
    pub digest: String,
}

/// The outcome of a property run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyReport {
    /// How many cases were run.
    pub cases: u64,
    /// How many were clean.
    pub clean: u64,
    /// The counterexamples, if any.
    pub counterexamples: Vec<Case>,
}

/// Build a random scenario for a seed.
///
/// The same seed always builds the same scenario.
pub fn random_scenario(seed: u64) -> Scenario {
    let mut generator = Generator::new(seed);
    let tier = Tier::all()[(generator.below(5)) as usize];
    let mut profile = LinkProfile::tier(tier).with_seed(generator.next_u64());

    // The rates: at least the tier's, so the profile stays inside the
    // tier's envelope and the tier's promises apply to it.
    profile.down.rate_bps = scale_rate(profile.down.rate_bps, &mut generator);
    profile.up.rate_bps = scale_rate(profile.up.rate_bps, &mut generator);
    profile.down.burst_bytes = profile.down.bytes_per_ms() * (100 + generator.below(1_900));
    profile.down.queue_cap_bytes = profile.down.burst_bytes * (1 + generator.below(4));
    profile.up.burst_bytes = profile.up.bytes_per_ms() * (100 + generator.below(1_900));
    profile.up.queue_cap_bytes = profile.up.burst_bytes * (1 + generator.below(4));

    // The jitter: no wider than the tier's own.
    let down_amplitude = profile.down.jitter.amplitude_us();
    let up_amplitude = profile.up.jitter.amplitude_us();
    profile.down.jitter = random_jitter(&mut generator, down_amplitude);
    profile.up.jitter = random_jitter(&mut generator, up_amplitude);

    // The loss, inside the tier's bound.
    let loss_bound = tier.loss_permille();
    profile.down.loss = random_loss(&mut generator, loss_bound);
    profile.up.loss = random_loss(&mut generator, loss_bound);

    // The reordering and the duplication.
    profile.down.reorder_permille = generator.below(200) as u32;
    profile.up.reorder_permille = generator.below(100) as u32;
    profile.down.duplicate_permille = generator.below(50) as u32;
    profile.up.duplicate_permille = generator.below(20) as u32;

    // The outages.
    let mut outages = Vec::new();
    let outage_count = generator.below(3);
    for _ in 0..outage_count {
        let start_us = generator.below(20_000_000);
        let duration_us = 100_000 + generator.below(5_000_000);
        let kind = match generator.below(3) {
            0 => OutageKind::Blackhole,
            1 => OutageKind::OneWayBlackhole {
                direction: if generator.chance_permille(500) {
                    Direction::Down
                } else {
                    Direction::Up
                },
            },
            _ => OutageKind::PartialLoss {
                permille: generator.below(1_000) as u32,
            },
        };
        outages.push(OutageWindow::new(start_us, duration_us, kind));
    }
    profile.outages = OutageSchedule::of(outages);

    let mut scenario = Scenario::new(format!("random-{seed}"), tier);
    scenario.seed = seed;
    scenario.link = profile.clone();

    // The work.
    scenario.steps.push(Step::Advance { ms: 500 });
    let changes = 1 + generator.below(4);
    for _ in 0..changes {
        scenario.steps.push(Step::CaptureChange {
            region: Region::new(
                u32::try_from(generator.below(1_920)).unwrap_or(0),
                u32::try_from(generator.below(1_080)).unwrap_or(0),
                64 + u32::try_from(generator.below(640)).unwrap_or(0),
                64 + u32::try_from(generator.below(400)).unwrap_or(0),
            ),
        });
        scenario.steps.push(Step::Advance {
            ms: 100 + generator.below(2_000),
        });
    }
    // Some typing.
    let typing = generator.below(6);
    for _ in 0..typing {
        let code = format!("Key{}", (b'A' + (generator.below(26) as u8)) as char);
        scenario.steps.push(Step::ViewerInput {
            event: Box::new(InputEvent::Key {
                code: code.clone(),
                pressed: true,
            }),
        });
        scenario.steps.push(Step::ViewerInput {
            event: Box::new(InputEvent::Key {
                code,
                pressed: false,
            }),
        });
        scenario.steps.push(Step::Advance {
            ms: 50 + generator.below(400),
        });
    }
    // A handover.
    if generator.chance_permille(300) {
        let mut next = profile.clone();
        next.down.rate_bps = scale_rate(next.down.rate_bps, &mut generator);
        next.up.rate_bps = scale_rate(next.up.rate_bps, &mut generator);
        scenario.steps.push(Step::Handover {
            handover: Box::new(Handover {
                profile: next,
                split_permille: generator.below(1_000) as u32,
                loss_permille: generator.below(500) as u32,
            }),
        });
    }
    // The settle phase is long enough for the estimator's round trip to
    // converge: a budget computed from a round trip that has not converged
    // yet is a budget of noise.
    scenario.steps.push(Step::Advance {
        ms: 4_000 + generator.below(10_000),
    });
    scenario
}

/// Run one random case.
pub fn run_case(seed: u64) -> Case {
    let scenario = random_scenario(seed);
    let trace = Driver::run(&scenario);
    let violated = trace
        .violations
        .iter()
        .map(|violation| violation.invariant.id().to_string())
        .collect();
    Case {
        seed,
        clean: trace.is_clean(),
        violated,
        scenario,
        digest: trace.digest,
    }
}

/// How many counterexamples the suite reports.
pub const MAX_COUNTEREXAMPLES: usize = 8;

/// Run a property suite over a range of seeds.
///
/// Every failing case is shrunk before it is reported. The suite runs the
/// whole range: a counterexample that only shows up on the five hundredth
/// case is a counterexample.
pub fn run(cases: u64, first_seed: u64) -> PropertyReport {
    let mut clean = 0;
    let mut counterexamples = Vec::new();
    for index in 0..cases {
        let seed = first_seed.wrapping_add(index);
        let case = run_case(seed);
        if case.clean {
            clean += 1;
            continue;
        }
        if counterexamples.len() < MAX_COUNTEREXAMPLES {
            counterexamples.push(shrink(&case));
        }
    }
    PropertyReport {
        cases,
        clean,
        counterexamples,
    }
}

/// How many candidate scenarios the shrinker may try.
pub const SHRINK_BUDGET: usize = 240;

/// Shrink a failing case to the smallest case that still fails.
pub fn shrink(case: &Case) -> Case {
    let mut best = case.clone();
    let mut changed = true;
    let mut tried = 0usize;
    while changed && tried < SHRINK_BUDGET {
        changed = false;
        // Remove one step at a time, from the end.
        for index in 0..best.scenario.steps.len() {
            if tried >= SHRINK_BUDGET {
                break;
            }
            tried += 1;
            let mut candidate = best.clone();
            candidate.scenario.steps.remove(index);
            if let Some(smaller) = try_case(&candidate) {
                best = smaller;
                changed = true;
                break;
            }
        }
        if changed {
            continue;
        }
        // Shorten the durations.
        for index in 0..best.scenario.steps.len() {
            if tried >= SHRINK_BUDGET {
                break;
            }
            tried += 1;
            let mut candidate = best.clone();
            if let Step::Advance { ms } = candidate.scenario.steps[index] {
                if ms > 10 {
                    candidate.scenario.steps[index] = Step::Advance { ms: ms / 2 };
                    if let Some(smaller) = try_case(&candidate) {
                        best = smaller;
                        changed = true;
                        break;
                    }
                }
            }
        }
    }
    best
}

/// Run a candidate and keep it if it still fails.
fn try_case(candidate: &Case) -> Option<Case> {
    let trace = Driver::run(&candidate.scenario);
    if trace.is_clean() {
        return None;
    }
    let violated = trace
        .violations
        .iter()
        .map(|violation| violation.invariant.id().to_string())
        .collect();
    Some(Case {
        seed: candidate.seed,
        clean: false,
        violated,
        scenario: candidate.scenario.clone(),
        digest: trace.digest,
    })
}

/// Scale a rate by a share, never below the tier's own.
fn scale_rate(rate: u64, generator: &mut Generator) -> u64 {
    let share = 100 + generator.below(101);
    (rate * share / 100).max(1_000)
}

/// A random jitter, no wider than the tier's own.
fn random_jitter(generator: &mut Generator, amplitude_us: u64) -> Jitter {
    let width = generator.below(amplitude_us + 1);
    match generator.below(3) {
        0 => Jitter::none(),
        1 => Jitter::uniform(width),
        _ => Jitter::normal(width),
    }
}

/// A random loss model, inside the tier's bound.
fn random_loss(generator: &mut Generator, bound: u32) -> LossModel {
    match generator.below(4) {
        0 => LossModel::None,
        1 => LossModel::Uniform {
            permille: generator.below(u64::from(bound) + 1) as u32,
        },
        2 => LossModel::Bursty {
            permille: generator.below(u64::from(bound) / 4 + 1) as u32,
        },
        _ => LossModel::Correlated {
            base_permille: generator.below(u64::from(bound) / 4 + 1) as u32,
            queue_permille: generator.below(u64::from(bound) + 1) as u32,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_scenario_is_reproducible() {
        let first = random_scenario(1234);
        let second = random_scenario(1234);
        assert_eq!(first, second);
    }

    #[test]
    fn different_seeds_give_different_scenarios() {
        let first = random_scenario(1);
        let second = random_scenario(2);
        assert_ne!(first.link, second.link);
    }

    #[test]
    fn a_random_case_is_reproducible() {
        let first = run_case(99);
        let second = run_case(99);
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.clean, second.clean);
    }

    #[test]
    fn a_property_run_reports_what_it_ran() {
        let report = run(3, 1_000);
        assert_eq!(report.cases, 3);
        assert!(report.clean + report.counterexamples.len() as u64 <= 3);
    }
}
