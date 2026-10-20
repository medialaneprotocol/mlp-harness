use mlp_harness::link::Tier;
use mlp_harness::predict;

fn main() {
    let tier = Tier::T4;
    let scenario = predict::canonical_scenario(tier);
    let trace = mlp_harness::run(&scenario);
    for u in trace.decisions.updates.iter().take(40) {
        println!("  upd t={} id={} kind={} cells={} bytes={} anchor={} reflects={}", u.at_us, u.update_id, u.kind, u.cells, u.bytes, u.anchor, u.reflects_input_seq);
    }
}
