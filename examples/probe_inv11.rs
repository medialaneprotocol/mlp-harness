//! Probe the parity-lies scenario's attach path.
fn main() {
    let text = std::fs::read_to_string("scenarios/broken-parity-lies.json").unwrap();
    let scenario: mlp_harness::Scenario = serde_json::from_str(&text).unwrap();
    let trace = mlp_harness::run(&scenario);
    eprintln!(
        "violations: {:?}",
        trace
            .violations
            .iter()
            .map(|v| v.invariant.id())
            .collect::<Vec<_>>()
    );
}
