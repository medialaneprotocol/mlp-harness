# mlp-harness

The **Media Lane Protocol's verification layer**: a deterministic, sans-IO
network harness that anyone can drive, and that answers questions.

MLP is a transport for real-time interactive media over links that are
slow, lossy, or both. This repository is where the transport's behaviour
over such links is **modelled, checked and predicted** — before a single
byte crosses a real network.

- **[The design](docs/design.md)** — the link model with every parameter,
  the scenario model, the invariant model, the trace format, the
  prediction interface.

## What it is

- a **network model**, not a delay: a rate shaped by a token bucket and a
  queue, a propagation delay with a jitter distribution, four loss models,
  reordering, duplication, an MTU, scheduled outages, two separate
  directions, and a **carrier handover** that costs what a real one costs;
- a **scenario**: a program against a virtual clock — advance the clock,
  offer an object, change the link's profile, hand over the carrier,
  assert something;
- the protocol's **real machines**, driven over that link: the sender and
  the receiver of [`mlp-core`][core], byte for byte, with the protocol's
  own fragments, acknowledgements, feedback reports, probes and input
  envelopes;
- the protocol's **invariants**, named, versioned, stated in one place,
  and checked — with a deliberately broken peer for each, so that a
  harness that cannot fail is caught;
- a **trace** a human can read and a machine can diff;
- a **property suite** that shrinks a failing case to a counterexample;
- a **prediction**: given a tier's profile, what should the receiver's
  echo p95 be?
- a **sans-IO driver** for a consumer's own state machines, so that they
  are driven the way the protocol's are.

[core]: https://github.com/medialaneprotocol/mlp-core

## What it is not

It is not a second implementation of the protocol. The harness never
decides what an update means, never assembles an object, never paces a
byte: it drives the protocol's machines and watches what they decide.

## Determinism

No wall clock, no randomness, no sockets, no filesystem, no environment.
Time is injected; randomness comes from a seeded generator the scenario
carries. A scenario that runs twice runs identically, on any machine, in
any order, in any process — and its trace is byte-identical.

## Using it

```bash
# What should T2's echo p95 be?
mlp-harness predict --tier t2

# Run a scenario and write its trace.
mlp-harness run --scenario scenarios/baseline-t2.json --trace /tmp/t2.json

# Look for counterexamples.
mlp-harness property --cases 200 --seed 20240101

# The invariant table.
mlp-harness invariants
```

As a library:

```rust
use mlp_harness::link::Tier;
use mlp_harness::predict;

let prediction = predict::predict_tier(Tier::T2);
assert_eq!(prediction.echo_p95_budget_ms, 550);
```

## The invariants

The set is the protocol's (MLP spec §11.2): an update is applied only
whole; at a converged observation the receiver's surface equals the
sender's emitted state; no input is applied twice or after its deadline;
the pressed set is empty after a park or a session end; no queue exceeds
its cap; no object exceeds its cap; the one-second rate window holds; the
input echo is bounded; a session survives every outage and flap
schedule; every drop is attributed to exactly one hop.

Each has a **shipped scenario with a deliberately broken peer** that trips
it, and the gate runs those scenarios and requires them to fail.

## The gate

```bash
scripts/verify.sh              # check everything
scripts/verify.sh --self-test  # check that the gate can fail
```

The gate formats, builds, lints and tests the harness, runs every shipped
scenario, requires every broken peer to trip its invariant, runs the
property suite, and asks every tier for a prediction.

## The layout

```
src/            the harness: the link, the clock, the driver, the invariants,
                the trace, the predictions, the property suite, and the
                sans-IO driver a consumer's own machines are driven with
scenarios/      the shipped scenarios: one per tier, one per story, and one
                per deliberately broken peer
scripts/        the gate, and the authoring tool for the scenarios
docs/           the design
```

## Licence

Apache-2.0 — see [LICENSE](LICENSE).

The reference implementation this harness drives, [`mlp-core`][core], is
under PolyForm Noncommercial 1.0.0. A binary built from this repository
links it, and carries its terms; this repository's own source is
Apache-2.0.
