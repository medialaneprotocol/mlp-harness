#!/usr/bin/env bash
# The harness's gate.
#
# The harness has no continuous integration: its checks are local, and this
# script is how they are run together. It is what a contributor runs before
# opening a proposal, and what a release is cut from.
#
# Usage:
#   scripts/verify.sh              # check what is present
#   scripts/verify.sh --self-test  # check that the gate can fail
#
# The gate needs a Rust toolchain and nothing else, so it runs the same on a
# developer machine, in a container, and on a build machine.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

LOGDIR="$(mktemp -d "${TMPDIR:-/tmp}/mlp-harness-XXXXXX")"
trap 'rm -rf "$LOGDIR"' EXIT

PASSES=0
FAILURES=0
SELFTEST=0

for argument in "$@"; do
    case "$argument" in
        --self-test) SELFTEST=1 ;;
        *) printf 'mlp-harness-gate: unknown argument %s\n' "$argument" >&2; exit 2 ;;
    esac
done

ok() {
    PASSES=$((PASSES + 1))
    printf '  ok    %s\n' "$1"
}

fail() {
    FAILURES=$((FAILURES + 1))
    printf '  FAIL  %s\n' "$1" >&2
    if [ $# -gt 1 ]; then
        printf '        %s\n' "$2" >&2
    fi
}

# Run a step, and report it by name.
step() { # name, command...
    local name="$1"
    shift
    if "$@" >"$LOGDIR/step.log" 2>&1; then
        ok "$name"
    else
        fail "$name" "see the output below"
        tail -20 "$LOGDIR/step.log" >&2
    fi
}

# ── the gate's own check ───────────────────────────────────────────────────
#
# A gate that cannot fail is not a gate. The self-test breaks a copy of the
# repository three ways and requires the gate to notice each of them.

self_test() {
    local scratch checks=0 failures=0
    scratch="$(mktemp -d "${TMPDIR:-/tmp}/mlp-harness-selftest-XXXXXX")"
    trap 'rm -rf "$scratch"' RETURN
    cp -r "$REPO_ROOT/." "$scratch/"

    # 1. A scenario that breaks an invariant must be noticed.
    python3 - "$scratch/scenarios/baseline-t2.json" <<'PY'
import json
import sys

path = sys.argv[1]
document = json.load(open(path))
document["faults"] = {"apply_partial_update": True}
json.dump(document, open(path, "w"), indent=2)
PY
    if ! (cd "$scratch" && cargo run --release --quiet --bin mlp-harness -- \
            run --scenario scenarios/baseline-t2.json >/dev/null 2>&1); then
        checks=$((checks + 1))
    else
        echo "self-test: a broken peer was not noticed" >&2
        failures=$((failures + 1))
    fi

    # 2. A scenario that does not parse must be noticed.
    printf 'not a scenario' >"$scratch/scenarios/baseline-t2.json"
    if (cd "$scratch" && cargo run --release --quiet --bin mlp-harness -- \
            run --scenario scenarios/baseline-t2.json >/dev/null 2>&1); then
        echo "self-test: a scenario that does not parse was not noticed" >&2
        failures=$((failures + 1))
    else
        checks=$((checks + 1))
    fi

    # 3. A missing scenario must be noticed.
    rm -f "$scratch/scenarios/baseline-t2.json"
    if (cd "$scratch" && cargo run --release --quiet --bin mlp-harness -- \
            run --scenario scenarios/baseline-t2.json >/dev/null 2>&1); then
        echo "self-test: a missing scenario was not noticed" >&2
        failures=$((failures + 1))
    else
        checks=$((checks + 1))
    fi

    if [ "$failures" -eq 0 ]; then
        echo "self-test: $checks mutation(s) rejected"
        return 0
    fi
    echo "self-test: $failures mutation(s) were not rejected" >&2
    return 1
}

echo "mlp-harness-gate: checking the harness at $REPO_ROOT"

if [ "$SELFTEST" = "1" ]; then
    if self_test; then
        exit 0
    fi
    exit 1
fi

# ── the code ──────────────────────────────────────────────────────────────

step "the harness is formatted" cargo fmt --check
step "the harness builds" cargo build --release
step "the harness lints clean" cargo clippy --release --all-targets -- -D warnings
step "the harness's own tests pass" cargo test --release

# ── the shipped scenarios ──────────────────────────────────────────────────

scenario_failures=0
scenario_count=0
for scenario in scenarios/*.json; do
    case "$(basename "$scenario")" in
        broken-*) continue ;;
    esac
    scenario_count=$((scenario_count + 1))
    if ! ./target/release/mlp-harness run --scenario "$scenario" >"$LOGDIR/scenario.log" 2>&1; then
        scenario_failures=$((scenario_failures + 1))
        printf '  FAIL  %s\n' "$scenario" >&2
        head -5 "$LOGDIR/scenario.log" >&2
    fi
done
if [ "$scenario_failures" -eq 0 ]; then
    ok "$scenario_count shipped scenarios run clean"
else
    fail "$scenario_failures of $scenario_count shipped scenarios violated an invariant"
fi

# The broken peers must trip the invariant they are built for. A harness
# that cannot fail is decoration, so this is a gate step and not a comment.
broken_failures=0
broken_count=0
for scenario in scenarios/broken-*.json; do
    broken_count=$((broken_count + 1))
    if ./target/release/mlp-harness run --scenario "$scenario" >"$LOGDIR/broken.log" 2>&1; then
        broken_failures=$((broken_failures + 1))
        printf '  FAIL  %s did not trip its invariant\n' "$scenario" >&2
    fi
done
if [ "$broken_failures" -eq 0 ]; then
    ok "$broken_count broken peers trip their invariant"
else
    fail "$broken_failures of $broken_count broken peers did not trip their invariant"
fi

# ── the property suite ────────────────────────────────────────────────────

# The property suite runs a fixed range of seeds. A counterexample is a
# finding about the implementation the harness drives; the known findings
# are committed with the reason they exist, and a *new* counterexample is
# a gate failure.
if ./target/release/mlp-harness property \
        --cases "$(python3 -c 'import json;print(json.load(open("known-findings.json"))["cases"])')" \
        --seed "$(python3 -c 'import json;print(json.load(open("known-findings.json"))["first_seed"])')" \
        --json >"$LOGDIR/property.json" 2>"$LOGDIR/property.log" || true; then
    if python3 scripts/check-findings.py "$LOGDIR/property.json" known-findings.json >"$LOGDIR/findings.log" 2>&1; then
        ok "the property suite found no new counterexample"
    else
        fail "the property suite found a new counterexample" "see the output below"
        cat "$LOGDIR/findings.log" >&2
    fi
else
    fail "the property suite did not run" "see the output below"
    head -20 "$LOGDIR/property.log" >&2
fi

# ── the predictions ───────────────────────────────────────────────────────

prediction_failures=0
for tier in t0 t1 t2 t3 t4; do
    if ! ./target/release/mlp-harness predict --tier "$tier" >"$LOGDIR/predict.log" 2>&1; then
        prediction_failures=$((prediction_failures + 1))
        printf '  FAIL  the %s prediction\n' "$tier" >&2
        head -5 "$LOGDIR/predict.log" >&2
    fi
done
if [ "$prediction_failures" -eq 0 ]; then
    ok "every tier's prediction is clean"
else
    fail "$prediction_failures tier prediction(s) are not clean"
fi

if [ "$FAILURES" -eq 0 ]; then
    echo "mlp-harness-gate: all checks passed ($PASSES)"
    exit 0
fi
echo "mlp-harness-gate: $FAILURES check(s) failed" >&2
exit 1
