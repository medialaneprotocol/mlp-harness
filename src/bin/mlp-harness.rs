//! The harness's command line.
//!
//! Four commands:
//!
//! * `run` — run a scenario and write its trace;
//! * `predict` — answer what a tier's link will do;
//! * `property` — run the randomised suite over a range of seeds;
//! * `invariants` — print the invariant table.
//!
//! Every command prints JSON when `--json` is given, and a short human
//! summary otherwise. A command that finds a violation exits non-zero, so
//! the harness can be a gate without a wrapper.

use std::path::PathBuf;
use std::process::ExitCode;

use mlp_harness::invariants::DEFINITIONS;
use mlp_harness::link::Tier;
use mlp_harness::predict;
use mlp_harness::property;
use mlp_harness::scenario::Scenario;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = arguments.first().map(String::as_str) else {
        usage();
        return ExitCode::from(2);
    };
    let rest = &arguments[1..];
    match command {
        "run" => run(rest),
        "predict" => predict_command(rest),
        "property" => property_command(rest),
        "invariants" => invariants(rest),
        "--help" | "-h" | "help" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("mlp-harness: unknown command {other:?}");
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "mlp-harness: the Media Lane Protocol's verification layer\n\
         \n\
         usage:\n\
         \x20 mlp-harness run --scenario <file> [--trace <file>] [--json]\n\
         \x20 mlp-harness predict --tier <t0..t4> [--scenario <file>] [--json]\n\
         \x20 mlp-harness property --cases <n> [--seed <n>] [--json]\n\
         \x20 mlp-harness invariants [--json]"
    );
}

/// Read a scenario from a file.
fn read_scenario(path: &PathBuf) -> Result<Scenario, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Scenario::from_json(&text)
}

/// Run a scenario.
fn run(arguments: &[String]) -> ExitCode {
    let mut scenario_path: Option<PathBuf> = None;
    let mut trace_path: Option<PathBuf> = None;
    let mut json = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--scenario" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("mlp-harness: --scenario needs a path");
                    return ExitCode::from(2);
                };
                scenario_path = Some(PathBuf::from(value));
            }
            "--trace" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("mlp-harness: --trace needs a path");
                    return ExitCode::from(2);
                };
                trace_path = Some(PathBuf::from(value));
            }
            "--json" => json = true,
            other => {
                eprintln!("mlp-harness: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }
    let Some(path) = scenario_path else {
        eprintln!("mlp-harness: run needs --scenario <file>");
        return ExitCode::from(2);
    };
    let scenario = match read_scenario(&path) {
        Ok(scenario) => scenario,
        Err(error) => {
            eprintln!("mlp-harness: {error}");
            return ExitCode::from(2);
        }
    };
    let trace = mlp_harness::run(&scenario);
    if let Some(path) = trace_path {
        if let Err(error) = std::fs::write(&path, trace.to_json()) {
            eprintln!("mlp-harness: cannot write {}: {error}", path.display());
            return ExitCode::from(2);
        }
    }
    if json {
        println!("{}", trace.to_json());
    } else {
        println!(
            "mlp-harness: {} — {} ms, {} update(s), {} object(s) delivered, {} dropped, digest {}",
            trace.scenario.name,
            trace.measurements.duration_ms,
            trace.measurements.updates_sent,
            trace.measurements.objects_delivered,
            trace.measurements.objects_dropped,
            trace.digest
        );
        for violation in &trace.violations {
            println!(
                "  {} at {} µs: {}",
                violation.invariant.id(),
                violation.at_us,
                violation.detail
            );
        }
    }
    if trace.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Answer what a link will do.
fn predict_command(arguments: &[String]) -> ExitCode {
    let mut tier: Option<Tier> = None;
    let mut scenario_path: Option<PathBuf> = None;
    let mut json = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--tier" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("mlp-harness: --tier needs a label");
                    return ExitCode::from(2);
                };
                match Tier::from_label(value) {
                    Some(parsed) => tier = Some(parsed),
                    None => {
                        eprintln!("mlp-harness: unknown tier {value:?}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--scenario" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("mlp-harness: --scenario needs a path");
                    return ExitCode::from(2);
                };
                scenario_path = Some(PathBuf::from(value));
            }
            "--json" => json = true,
            other => {
                eprintln!("mlp-harness: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }
    let prediction = match (tier, scenario_path) {
        (Some(tier), None) => predict::predict_tier(tier),
        (None, Some(path)) => match read_scenario(&path) {
            Ok(scenario) => predict::predict(&scenario),
            Err(error) => {
                eprintln!("mlp-harness: {error}");
                return ExitCode::from(2);
            }
        },
        (Some(_), Some(_)) => {
            eprintln!("mlp-harness: give a tier or a scenario, not both");
            return ExitCode::from(2);
        }
        (None, None) => {
            eprintln!("mlp-harness: predict needs --tier <t0..t4> or --scenario <file>");
            return ExitCode::from(2);
        }
    };
    if json {
        println!("{}", prediction.to_json());
    } else {
        println!(
            "mlp-harness: {} — echo p50 {:?} ms, p95 {:?} ms (budget {} ms), convergence {:?} ms, {} bytes down, {} bytes up, digest {}",
            prediction.scenario,
            prediction.echo_p50_ms,
            prediction.echo_p95_ms,
            prediction.echo_p95_budget_ms,
            prediction.convergence_ms,
            prediction.bytes_down,
            prediction.bytes_up,
            prediction.digest
        );
    }
    if prediction.clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Run the randomised suite.
fn property_command(arguments: &[String]) -> ExitCode {
    let mut cases: u64 = 32;
    let mut seed: u64 = 1;
    let mut json = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--cases" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("mlp-harness: --cases needs a number");
                    return ExitCode::from(2);
                };
                cases = value.parse().unwrap_or(0);
            }
            "--seed" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("mlp-harness: --seed needs a number");
                    return ExitCode::from(2);
                };
                seed = value.parse().unwrap_or(0);
            }
            "--json" => json = true,
            other => {
                eprintln!("mlp-harness: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }
    let report = property::run(cases, seed);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("a report is serializable")
        );
    } else {
        println!(
            "mlp-harness: {} case(s), {} clean, {} counterexample(s)",
            report.cases,
            report.clean,
            report.counterexamples.len()
        );
        for case in &report.counterexamples {
            println!(
                "  seed {} violated {}",
                case.seed,
                case.violated.join(", ")
            );
        }
    }
    if report.counterexamples.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Print the invariant table.
fn invariants(arguments: &[String]) -> ExitCode {
    let json = arguments.iter().any(|argument| argument == "--json");
    if json {
        let table: Vec<_> = DEFINITIONS
            .iter()
            .map(|definition| {
                serde_json::json!({
                    "id": definition.invariant.id(),
                    "version": definition.version,
                    "statement": definition.statement,
                    "forbids": definition.forbids,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&table).expect("the table is serializable")
        );
        return ExitCode::SUCCESS;
    }
    for definition in DEFINITIONS {
        println!(
            "{} v{}: {}",
            definition.invariant.id(),
            definition.version,
            definition.statement
        );
        println!("    forbids: {}", definition.forbids);
    }
    ExitCode::SUCCESS
}
