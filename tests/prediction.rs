//! The prediction interface and the property suite.

use mlp_harness::link::Tier;
use mlp_harness::predict;
use mlp_harness::property;

#[test]
fn a_prediction_answers_the_question_it_was_asked() {
    let prediction = predict::predict_tier(Tier::T0);
    assert_eq!(prediction.tier, Tier::T0);
    assert_eq!(prediction.echo_p95_budget_ms, Tier::T0.echo_budget_ms());
    assert!(prediction.echo_samples > 0, "no echo was measured");
    assert!(prediction.echo_p95_ms.is_some());
    assert!(prediction.bytes_down > 0);
    assert!(prediction.digest.len() == 16);
    assert!(prediction.clean);
}

#[test]
fn a_prediction_is_the_same_twice() {
    let first = predict::predict_tier(Tier::T1);
    let second = predict::predict_tier(Tier::T1);
    assert_eq!(first, second);
}

#[test]
fn the_prediction_is_a_json_document() {
    let prediction = predict::predict_tier(Tier::T0);
    let json = prediction.to_json();
    assert!(json.contains("\"echo_p95_ms\""));
    assert!(json.contains("\"echo_p95_budget_ms\""));
}

#[test]
fn every_tier_answers() {
    for tier in Tier::all() {
        let prediction = predict::predict_tier(tier);
        assert_eq!(prediction.tier, tier);
        assert!(prediction.clean, "{} is not clean", tier.as_str());
    }
}

#[test]
fn a_random_case_is_reproducible() {
    let first = property::run_case(4_242);
    let second = property::run_case(4_242);
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.clean, second.clean);
}

#[test]
fn a_property_run_reports_what_it_ran() {
    let report = property::run(8, 7_000);
    assert_eq!(report.cases, 8);
    assert!(
        report.counterexamples.is_empty(),
        "the property suite found {:?}",
        report
            .counterexamples
            .iter()
            .map(|case| (case.seed, case.violated.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_counterexample_is_shrunk() {
    // A scenario that breaks the pressed set: the shrinker must find a
    // smaller case that still breaks it.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenarios")
        .join("broken-stuck-key.json");
    let text = std::fs::read_to_string(&path).expect("the scenario is there");
    let scenario = mlp_harness::Scenario::from_json(&text).expect("the scenario parses");
    let case = property::Case {
        seed: 1,
        clean: false,
        violated: vec!["INV-4".to_string()],
        scenario,
        digest: String::new(),
    };
    let shrunk = property::shrink(&case);
    assert!(shrunk.scenario.steps.len() <= case.scenario.steps.len());
    let trace = mlp_harness::run(&shrunk.scenario);
    assert!(
        trace
            .violations
            .iter()
            .any(|violation| violation.invariant.id() == "INV-4"),
        "the shrunk case no longer breaks the invariant"
    );
}
