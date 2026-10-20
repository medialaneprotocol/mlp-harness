//! What should this tier's echo p95 be?
//!
//! ```text
//! cargo run --release --example predict -- t2
//! ```
//!
//! The example is the smallest thing a consumer of the harness can write:
//! ask the model a question about a link, and print the answer next to the
//! promise the tier makes.

use mlp_harness::link::Tier;
use mlp_harness::predict;

fn main() {
    let label = std::env::args().nth(1).unwrap_or_else(|| "t2".to_string());
    let Some(tier) = Tier::from_label(&label) else {
        eprintln!("predict: unknown tier {label:?}; the tiers are t0..t4");
        std::process::exit(2);
    };
    let prediction = predict::predict_tier(tier);
    println!("tier             {}", prediction.tier.as_str());
    println!("echo p50         {:?} ms", prediction.echo_p50_ms);
    println!("echo p95         {:?} ms", prediction.echo_p95_ms);
    println!("echo budget      {} ms", prediction.echo_p95_budget_ms);
    println!(
        "within promise  {}",
        if prediction.echo_within_promise {
            "yes"
        } else {
            "no"
        }
    );
    println!("convergence      {:?} ms", prediction.convergence_ms);
    println!("bytes down       {}", prediction.bytes_down);
    println!("bytes up         {}", prediction.bytes_up);
    println!("digest           {}", prediction.digest);
}
