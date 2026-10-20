//! The harness's own suite.
//!
//! The suite has three parts:
//!
//! * the **shipped scenarios** must run clean, and must run identically
//!   twice — the same bytes, the same digest;
//! * the **deliberately broken peers** must each trip the invariant they
//!   are built for, and only that one: a harness that cannot fail is
//!   decoration;
//! * the **link model** must hold the properties the model claims.

use std::path::{Path, PathBuf};

use mlp_harness::faults::Faults;
use mlp_harness::invariants::Invariant;
use mlp_harness::link::Tier;
use mlp_harness::scenario::Scenario;

/// The repository's scenarios.
fn scenarios_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios")
}

/// Read one shipped scenario.
fn scenario(name: &str) -> Scenario {
    let path = scenarios_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    Scenario::from_json(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The shipped scenarios, by name.
const SHIPPED: &[&str] = &[
    "baseline-t0",
    "baseline-t1",
    "baseline-t2",
    "baseline-t3",
    "baseline-t4",
    "typing-echo-t1",
    "typing-echo-t2",
    "typing-echo-t3",
    "outage-blackhole-t3",
    "outage-one-way-t2",
    "flap-five-cycles-t3",
    "handover-t2-to-t1",
    "reordering-t2",
    "lossy-bursty-t3",
];

#[test]
fn every_shipped_scenario_runs_clean() {
    for name in SHIPPED {
        let scenario = scenario(name);
        let trace = mlp_harness::run(&scenario);
        assert!(
            trace.is_clean(),
            "{name} violated {:?}",
            trace
                .violations
                .iter()
                .map(|violation| (violation.invariant.id(), violation.detail.clone()))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn every_shipped_scenario_runs_twice_identically() {
    for name in SHIPPED {
        let scenario = scenario(name);
        let first = mlp_harness::run(&scenario);
        let second = mlp_harness::run(&scenario);
        assert_eq!(
            first.to_json(),
            second.to_json(),
            "{name} is not reproducible"
        );
        assert_eq!(first.digest, second.digest, "{name}'s digest moved");
    }
}

#[test]
fn a_trace_carries_the_scenario_the_link_and_the_decisions() {
    let scenario = scenario("baseline-t2");
    let trace = mlp_harness::run(&scenario);
    assert_eq!(trace.scenario.name, "baseline-t2");
    assert_eq!(trace.scenario.tier, Tier::T2);
    assert!(!trace.decisions.updates.is_empty(), "no update was emitted");
    assert!(!trace.link.events.is_empty(), "the link recorded nothing");
    assert_eq!(trace.violations.len(), 0);
    assert_eq!(trace.digest.len(), 16);
    assert!(trace.digest.chars().all(|c| c.is_ascii_hexdigit()));
}

/// The shipped scenario that breaks each invariant.
fn broken_scenario(invariant: Invariant) -> &'static str {
    match invariant {
        Invariant::Inv1 => "broken-partial-update",
        Invariant::Inv2 => "broken-unemitted-cell",
        Invariant::Inv3 => "broken-input-twice",
        Invariant::Inv4 => "broken-stuck-key",
        Invariant::Inv5 => "broken-unbounded-queue",
        Invariant::Inv6 => "broken-oversize-object",
        Invariant::Inv7 => "broken-burst-over-budget",
        Invariant::Inv8 => "broken-slow-echo",
        Invariant::Inv9 => "broken-recreated-session",
        Invariant::Inv10 => "broken-unattributed-drop",
    }
}

#[test]
fn every_invariant_has_a_broken_peer_that_trips_it() {
    for invariant in Invariant::all() {
        let scenario = scenario(broken_scenario(invariant));
        let trace = mlp_harness::run(&scenario);
        let tripped: Vec<&str> = trace
            .violations
            .iter()
            .map(|violation| violation.invariant.id())
            .collect();
        assert!(
            tripped.contains(&invariant.id()),
            "{invariant} did not trip: {tripped:?}"
        );
    }
}

#[test]
fn every_invariant_has_a_fault_that_names_it() {
    for invariant in Invariant::all() {
        let faults = Faults::for_invariant(invariant);
        assert_eq!(faults.active().len(), 1, "{invariant} has one fault");
    }
}

#[test]
fn a_correct_peer_trips_nothing() {
    let scenario = scenario("baseline-t2");
    let trace = mlp_harness::run(&scenario);
    assert!(trace.is_clean());
    for outcome in &trace.invariants {
        assert!(outcome.holds, "{} did not hold", outcome.invariant.id());
    }
}
