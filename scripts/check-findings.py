#!/usr/bin/env python3
"""Check the property suite's counterexamples against the known findings.

The suite runs a fixed range of seeds and reports every case that breaks an
invariant. A counterexample is a *finding*: it says something about the
implementation the harness drives. The known findings are the ones that are
already understood, and they are committed with the reason they exist.

The check fails when a counterexample appears that the known findings do
not name: that is a new finding, and it is somebody's turn to look at it.

Usage: python3 scripts/check-findings.py <report.json> <known-findings.json>
"""

import json
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("check-findings: a report and the known findings", file=sys.stderr)
        return 2
    report = json.load(open(sys.argv[1]))
    known = json.load(open(sys.argv[2]))

    expected = {(finding["seed"], finding["invariant"]) for finding in known["findings"]}
    found = set()
    for case in report["counterexamples"]:
        for invariant in case["violated"]:
            found.add((case["seed"], invariant))

    new = sorted(found - expected)
    missing = sorted(expected - found)
    for seed, invariant in new:
        print(f"check-findings: seed {seed} breaks {invariant}, and no finding names it")
    for seed, invariant in missing:
        print(f"check-findings: the finding for seed {seed} ({invariant}) did not reproduce")
    if new or missing:
        return 1
    print(
        f"check-findings: {len(found)} known finding(s), no new one"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
