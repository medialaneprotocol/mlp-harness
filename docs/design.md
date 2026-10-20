# The harness's design

The **Media Lane Protocol's verification layer**: a deterministic, sans-IO
network harness that anyone can drive, and that answers questions.

This document is the design: the link model with every parameter and its
default, the scenario model, the invariant model, the trace format, the
prediction interface, and what the harness deliberately does not do.

---

## 1. What the harness is

The harness is three things at once, and the three are one thing:

1. a **network model** — a link with a rate, a queue, a delay with jitter,
   four kinds of loss, reordering, duplication, an MTU, outages, two
   directions, and a carrier handover;
2. a **driver** — a program against a virtual clock that runs the
   protocol's *real* sender and receiver over that link, byte for byte;
3. a **verdict** — the protocol's invariants, named, versioned, stated in
   one place, and checked, together with a trace a human can read and a
   machine can diff, and a prediction the model answers with.

It is **sans-IO, strictly**: no wall clock, no randomness, no sockets, no
filesystem, no environment. Time is injected; randomness comes from a
seeded generator the scenario carries. A scenario that runs twice runs
identically, on any machine, in any order, in any process, and its trace is
byte-identical.

### 1.1 The dependency runs one way

The harness depends on `mlp-core` — the machines it drives — and on nothing
else of ours. `mlp-core` does not depend on the harness, except as a
*dev-dependency* for its own suites: a cycle cargo allows, and one the
harness's repository sets up in the wave that switches the consumers over
(see [the switch-over plan](switch-over.md)).

The consumer depends on the harness, never the other way round.

### 1.2 The vocabulary

The transport's: **update**, **part**, **region**, **op**, **cell**,
**surface**, **reference**; the roles **sender**, **receiver**,
**intermediary**. No "frame", no "chunk".

---

## 2. The link model

A link is a **network**, not a delay. Every parameter has a default, and
the default is the tier's canonical point.

### 2.1 The shape of a profile

```
LinkProfile
├── down: DirectionProfile      the sender → receiver direction
├── up:   DirectionProfile      the receiver → sender direction
├── mtu_bytes: u64              the largest object the carrier admits
├── outages: OutageSchedule     the outages this link suffers
└── seed: u64                   the seed of the link's generator
```

The two directions are **separate links with separate profiles**. A link
whose downlink is 1 Mbps and whose uplink is 256 kbps is the normal case,
not the exotic one.

### 2.2 The parameters of a direction

| Parameter | Meaning | Default |
|---|---|---|
| `rate_bps` | the direction's rate, in bits per second | the tier's floor |
| `propagation_us` | the one-way propagation delay | half the tier's RTT |
| `jitter` | the delay's random component | the tier's jitter bound |
| `loss` | how objects are lost | the tier's loss bound |
| `reorder_permille` | the share of objects displaced out of order | 0 |
| `duplicate_permille` | the share of objects delivered twice | 0 |
| `queue_cap_bytes` | the queue's cap | two seconds of the rate |
| `burst_bytes` | the token bucket's size | one second of the rate |
| `reorder_window_us` | the largest displacement a reordering causes | 10 ms |
| `duplicate_delay_us` | the delay between an object and its duplicate | 1 ms |
| `extra_delay_us` | an extra delay this direction adds to every object | 0 |

### 2.3 The rate: a token bucket and a queue

The rate is shaped by a **token bucket** and a **FIFO queue**:

- the bucket holds up to `burst_bytes` and refills at `rate_bps / 8` bytes
  per millisecond;
- an object offered to the link waits in the queue until the bucket holds
  enough tokens for it;
- the object leaves the queue at the **exact** moment the bucket holds
  enough — computed in integer micro-bytes, so the release time does not
  depend on how often the driver polls the link;
- an object that does not fit in the queue is **dropped** (the queue's
  overflow), and the drop is attributed to the link's queue.

A burst is therefore *shaped*: the objects that do not fit the bucket wait
their turn instead of travelling all at once. The bucket's size is what a
burst of that size passes at once.

### 2.4 The delay: a propagation delay and a jitter distribution

The delay is the propagation delay plus a jitter sample. Four shapes:

| Shape | What it models |
|---|---|
| `None` | a constant delay |
| `Uniform` | a flat distribution over `[0, amplitude]` — a queue of variable depth |
| `Normal` | a bell around zero, truncated at zero — independent noise |
| `Pareto` | a heavy tail with shape 1.5 — a radio link; T4's own |

Every sample is drawn in microseconds and rounded to a whole one, so the
model never carries a fractional timestamp.

### 2.5 The loss: four models

Loss is not one thing, and a transport's behaviour under loss depends on
which it is:

| Model | What it models |
|---|---|
| `None` | a link that loses nothing |
| `Uniform { permille }` | independent loss: a radio fading at random |
| `Bursty { permille }` | loss in runs of a geometric length, mean 4 objects |
| `GilbertElliott { good_to_bad, bad_to_good }` | a two-state chain: a good state and a bad state, and everything offered in the bad state is lost |
| `Correlated { base_permille, queue_permille }` | loss that grows with the queue's occupancy: a congested queue drops a run of consecutive objects |

Loss is decided **when the object is offered**, because that is when the
queue's occupancy is known, and the correlated model is a function of the
occupancy. The queue's overflow is a separate, deterministic drop.

### 2.6 Reordering and duplication

- **Reordering**: with probability `reorder_permille` an object's delivery
  is displaced by a uniform draw in `[1, reorder_window_us]`. The window is
  the bound on how far an object may be displaced.
- **Duplication**: with probability `duplicate_permille` a copy of the
  object is delivered `duplicate_delay_us` later. The copy is a separate
  delivery, and the receiver must be idempotent about it.

### 2.7 The MTU

An object larger than the MTU is **dropped, not fragmented**. The
protocol fragments an update into parts that fit the carrier's cap, so an
oversize object is a breach of the protocol's own rules, and the link does
not paper over it. The drop is attributed to the link.

### 2.8 The outages

Three kinds, because a network fails in three ways:

| Kind | What it does |
|---|---|
| `Blackhole` | nothing gets through, in either direction |
| `OneWayBlackhole { direction }` | one direction is dead and the other works |
| `PartialLoss { permille }` | the link works, but drops objects at a stated rate |

An outage is checked at two moments: when an object is **offered** (a
blackholed link takes nothing) and when an object would be **delivered** (a
link that goes black while the object is in flight loses it). Both are
attributed to the outage, so a drop is never unexplained.

### 2.9 The carrier handover

A handover is what a real handover costs. The rule, stated once:

- the objects **already in the pipe** are split: `split_permille` of them
  continue on the old carrier and the rest continue on the new one;
- independently, `loss_permille` of them are lost at the switch;
- the objects **queued but not yet released** are released on the new
  carrier — they were not on the wire yet;
- the new carrier's profile applies to everything offered after the
  switch.

A model that ignores the handover predicts a handover as a change of
delay. It is not one: it is a burst of loss followed by a re-measurement
of a path that is not the one the sender measured.

### 2.10 The units

Everything is counted in microseconds and bytes. The rate is applied
through a token bucket held in **micro-bytes** (a millionth of a byte), so
a rate of any precision is represented exactly and no rounding ever
decides a delivery time.

---

## 3. The scenario model

A scenario is a **sequence of steps** against a virtual clock. It is a
value: it serializes, it diffs, and two equal scenarios produce
byte-identical traces.

| Step | What it does |
|---|---|
| `Advance { ms }` | move the clock |
| `SetLink { profile }` | replace the link's profile |
| `Handover { handover }` | change the carrier under the link's feet |
| `CaptureChange { region }` | the capture produced a change in a region |
| `ViewerInput { event }` | the user did something |
| `ViewerDisconnect` / `ViewerReconnect` | the receiver's lane died, and came back |
| `HostReconnect` | the sender's transport was re-established |
| `PathSwitch { path }` | the receiver's carrier changed |
| `ScheduleOutage { window }` | an outage is scheduled from now |
| `Assert { invariant }` | check an invariant at this point |

A scenario also carries a **seed**, a **link profile**, a **grid**, and a
set of **faults** (see §5.3).

### 3.1 The virtual clock

The clock counts microseconds. The protocol's machines are handed an
`Instant`, and they measure everything as a difference from the instant
they were started at, so the harness takes its origin from the process
once and never exposes it: it never appears in a trace, it never reaches a
decision, and every number the harness reports is a difference from it.

The harness's tick is **one millisecond**. It is fine enough for the
protocol's timers and the pacer, and coarse enough that a minute-long
scenario is sixty thousand ticks.

---

## 4. The driver

The driver runs a scenario. It owns the clock, the link, the protocol's
sender and receiver machines, the shadow reference, the input path, and
the checker.

### 4.1 The real machines, byte for byte

Every fragment on the link is built by the protocol's own sender machine
and parsed by the protocol's own receiver machine. Every acknowledgement,
every feedback report, every probe, every probe echo and every input
envelope is the protocol's own type. The harness never decides what an
update *means*.

The one thing the harness owns is the **content** of a media update: the
transport does not interpret an update's content (MLP spec §5.1), so the
harness's media source is a **cell bitmap** — the smallest thing that says
which cells an update covers — padded to the source's cost per cell.

That cost is **256 bytes per cell**. The protocol's scheduler budgets 512
bytes per cell as the *upper bound* of any codec's cost, and the object cap
the sender enforces is at least the scheduler's per-update budget, so a
source that costs half the upper bound always fits inside one object.

### 4.2 The shadow reference

The harness keeps, per cell:

- **emitted** — the update id the sender emitted for it;
- **held** — the update id the receiver holds for it;
- **acknowledged** — the update id the receiver acknowledged for it.

INV-2 compares the three. The shadow reference is the harness's own
record; the receiver's own record is the protocol's.

### 4.3 The input path

The user's keystrokes go into the receiver's input queue, travel up the
link as control-class objects at the input priority, and are applied by
the sender's input state. The **host's pressed set** is the deployment's:
the harness maintains it from the input objects the sender applied, and the
pressed-set protocol (the heartbeat and the resume) is what repairs it
after a release the link lost.

When the sender applies an input, the harness asks for an **input pulse**,
so the echo is measured on the very next update rather than on the next
idle one.

### 4.4 What the driver reports to the estimator

A driver that never says what it wants is a driver the estimator cannot
measure, so the harness reports its **demand** to the estimator on every
tick: the pending cells, the dirty cells, and the queued objects. That is
what arms the demand-driven probe and lets the pace climb on a link whose
loss keeps the estimator's own ramp from growing it.

---

## 5. The invariant model

### 5.1 The definitions

Every invariant is **named, versioned, and stated in one place**: a
definition and a prohibition, in the table in `src/invariants.rs`. The
harness's checkers and the harness's suite read them from there.

| id | version | invariant | forbids |
|---|---|---|---|
| INV-1 | 1 | an update is applied only whole | an update applied while one of its parts is missing |
| INV-2 | 1 | at a converged observation the receiver's surface equals the sender's emitted state | a converged observation in which the receiver holds a cell the sender never emitted |
| INV-3 | 1 | no input is applied twice, and none after its deadline | an input applied twice, or applied after its deadline |
| INV-4 | 1 | the pressed set is empty after a park or a session end | a key the receiver released that the sender still holds |
| INV-5 | 1 | no queue exceeds its cap | a queue deeper than its cap |
| INV-6 | 1 | no object exceeds its cap | an object larger than `max_object_bytes` |
| INV-7 | 1 | the bytes sent in any one-second window stay inside the budget | more bytes released in a one-second window than the budget allows |
| INV-8 | 1 | the input echo stays inside `2 × RTT + 250 ms` | an echo series whose p95 exceeds the tier's budget |
| INV-9 | 1 | a session survives every outage and flap schedule | a session lost to an outage, or a second lane for one receiver |
| INV-10 | 1 | every drop is attributed to exactly one hop | a drop that no hop accounts for |

A change to what an invariant *means* — not to how it is checked — is a
version bump, and every trace records the version, so a trace taken before
the bump and one taken after it are never silently compared.

### 5.2 When each invariant is checked

The **cheap** invariants are checked on every tick: INV-1, INV-2, INV-3,
INV-5, INV-6 and INV-7. They are the ones a *state* decides: a queue's
depth, an object's size, a window's bytes, a cell's state.

The **contextual** ones depend on a moment in the run — the echo, the
session's continuity, the loss attribution — and are checked where the
scenario asks for them, with two exceptions: INV-4 is checked at the
moment a park's grace elapses, and INV-4 and INV-8 are checked at the end
of the run.

### 5.3 The faults: a deliberately broken peer

An invariant that nothing can violate is decoration. The harness carries a
**fault injector**: a set of switches, each of which makes the *peer*
misbehave in exactly one stated way, and each of which exists so that one
invariant can be shown to bite.

A fault is not a mode of the harness's *model*. It is a defect in the
implementation under test. The link, the clock and the protocol's machines
stay honest; only the peer's behaviour changes.

| Fault | The broken peer | The invariant it trips |
|---|---|---|
| `apply_partial_update` | applies an update whose parts have not all arrived | INV-1 |
| `acknowledge_unemitted_cell` | acknowledges a cell the sender never emitted | INV-2 |
| `apply_input_twice` | applies the same input twice | INV-3 |
| `apply_input_after_deadline` | applies an input after its deadline | INV-3 |
| `keep_key_held_after_release` | never learns that a key was released | INV-4 |
| `unbounded_queue` | queues the input objects and never sends them | INV-5 |
| `oversize_object` | puts an object on the link above the cap | INV-6 |
| `burst_over_budget` | releases a whole window at once | INV-7 |
| `delay_echo` | answers the input far too late | INV-8 |
| `recreate_session` | creates a second lane for one receiver | INV-9 |
| `unattributed_drop` | claims a loss no hop accounts for | INV-10 |

Every fault has a **shipped scenario** that trips it, and the gate runs
those scenarios and requires them to fail.

### 5.4 The checkers are pure

The checkers read an `Observation` — a plain record of what happened — and
know nothing about how it happened. The separation is what makes the
broken peers meaningful: a fault corrupts the observation's *source*, and
the checker, which never saw the corruption, still fires.

---

## 6. The trace format

One canonical format, JSON, with the keys in a fixed order. It records
five things:

1. the **scenario** that produced it;
2. the **link's events** — offered, released, delivered, dropped (with the
   reason), the profile changes and the handovers;
3. the **decisions** — the updates, the acknowledgements, the plans, the
   lane states, the inputs and the echoes;
4. the **invariant outcomes** — how many times each was checked, how many
   times it was violated, and its version;
5. a **digest** over the decisions, and the **measurements**.

### 6.1 The digest

The digest is FNV-1a over 64 bits, written as sixteen lowercase hex
digits — the same shape the protocol's corpus uses, so a corpus document's
digest field keeps its meaning across the switch.

It covers the **decisions** and deliberately not the timings: it answers
"did the protocol decide something different?", and the timings depend on
the link's profile while the decisions do not. Two runs of the same
scenario over the same profile produce the same digest and byte-identical
traces.

### 6.2 The diff is the review

Two runs of the same scenario produce byte-identical traces, so a diff of
two traces is the review of a change: what changed, where, and by how
much.

---

## 7. The prediction interface

The harness is not only a test rig. It is a **prediction engine**: given a
link's profile and a scenario, it says what the session will do.

```
mlp-harness predict --tier t2
mlp-harness predict --tier t2 --json
mlp-harness predict --scenario scenarios/baseline-t2.json
```

Every prediction carries the **model's** answer and the **promise** it is
measured against:

- `echo_p50_ms`, `echo_p95_ms`, `echo_max_ms` — what the model says;
- `echo_p95_budget_ms` — what the tier promises (`2 × RTT + 250 ms`);
- `echo_within_promise` — whether the model's answer is inside it;
- `convergence_ms`, `convergences`;
- `bytes_down`, `bytes_up`, `objects_delivered`, `objects_dropped`;
- `rtt_ms`, `pace_bps`, the trace's `digest`, and whether the run was
  clean.

A consumer that deploys the transport has live measurements; the
prediction is what those measurements are *checked against*.

### 7.1 The canonical scenario

A prediction needs a scenario: the same link with different work gives
different answers. The harness ships one **canonical scenario per tier** —
the story the tiers are written about: the link settles, the capture
changes, the user types six characters, the session settles — scaled by
the tier's round trip, because a scenario too short for the tier measures
the clock rather than the protocol.

---

## 8. The property suite

Randomised profiles, random object sequences, random outages, random
handovers. The invariants must hold. A failing case is **shrunk** to a
counterexample: the shrinker removes steps, shortens durations and
simplifies profiles, keeping only the changes that preserve the failure,
and prints the result as a scenario document.

The suite is seeded: a case is reproducible from its seed alone, and the
seed is the first thing a report names. The randomised profiles stay
**inside the tier's envelope** — the rate, the delay, the jitter and the
loss are no worse than the tier's canonical point — because a promise is a
promise about a link inside the envelope, and a link outside it is a
different question.

### 8.1 The known findings

A counterexample is a **finding**: it says something about the
implementation the harness drives. The gate runs a fixed range of seeds
and compares the counterexamples with the **known findings**, which are
committed in `known-findings.json` with the reason they exist. A *new*
counterexample is a gate failure: somebody has to look at it.

At the pinned revision the known findings are two seeds that break INV-8 —
the echo's budget against a link whose loss drives the estimator's pace
down, so the pulse update waits for the pacer longer than the budget
allows. The fixes in the core's own work change both, and the list is
emptied when the pin moves (see the switch-over plan).

---

## 9. What the harness deliberately does not do

- It does **not** re-implement the protocol. It drives the protocol's
  machines and watches what they decide.
- It does **not** interpret an update's content. The media source is a
  bitmap.
- It does **not** model an intermediary. The harness's link *is* the path
  between the two ends, so the link's drops are the only attribution; the
  invariant's arithmetic carries a hop term for a deployment that has one.
- It does **not** read the environment, the clock or the filesystem while
  it runs. The gate reads files; the harness does not.
- It does **not** check the invariants it cannot judge. The echo promise
  applies where the tier's promise applies: not at T4, not on a link with
  an outage, and not on a link whose loss is beyond the tier's bound.

---

## 10. What the harness found

The harness is a verification layer, and it has already found two things
about the core it drives, at the revision it pins.

### 10.1 The receiver's gap detection counts a reordering as a loss

The receiver's loss report comes from gap detection on the object id. The
core's writer drains by priority, so an object may overtake an older one
on the wire; the receiver must not lower its expectation when that
happens, or every later object looks like a lost one (MLP spec §6.2.2).

At the pinned revision the expectation *does* move back, so a reordered
object costs the receiver two phantom gaps, and the estimator reacts to the
phantom loss by decaying the pace. The fix is in the core's own work; the
harness's pin moves with it in the switch-over wave.

### 10.2 The sender's round trip starts at one millisecond

The receiver's first feedback report carries a round trip of zero — it has
not measured one yet — and the sender's estimator takes the report as a
*sample*, so the sender's round trip starts at one millisecond and grows
back over the next reports. The estimator's own rules say a report is a
cross-check and never a sample (MLP spec §6.2.2); the fix is in the
core's own work as well.

Both findings are the harness doing its job: a verification layer that
finds nothing is a decoration.
