# The switch-over plan

The harness replaces three things at once, and the end state has **one**
harness and no mention of the others anywhere:

| What goes | Where it lives now | Where it goes |
|---|---|---|
| the deterministic harness and the lossy-link simulator | `mlp-core`'s `src/testing/` | `mlp-harness` |
| the corpus's generator | `mlp-conformance`'s `tools/corpus-gen.rs` | `mlp-harness`'s library |
| the corpus's static validator | `mlp-conformance`'s `tools/corpus_static.py` | `mlp-harness`'s own checks |

The plan below is **the order**, and it is written so that every step is
mechanical: a file is moved, a manifest line is added, a command is
replaced.

The plan is written against the harness as it stands today
(`mlp-harness` v0.1.0, pinning `mlp-core` at `d1c257d`).

---

## 0. What must land before anything else

The harness's pin must move to a core revision that carries the two
fixes the harness found (see the design's §10):

- the receiver's gap detection must not lower its expectation when an
  object arrives out of order (MLP spec §6.2.2);
- the sender's estimator must treat the receiver's report as a
  cross-check and never as a sample (MLP spec §6.2.2).

Those fixes are in the core's own work. When they are published:

1. the harness's `Cargo.toml` moves its pin:
   ```toml
   mlp-core = { git = "https://github.com/medialaneprotocol/mlp-core", rev = "<new revision>" }
   ```
2. `Cargo.lock` is regenerated and committed;
3. the harness's `scripts/verify.sh` runs, and the shipped scenarios'
   digests are regenerated (the digests are over the *decisions*, so a
   core change moves them);
4. the harness's tag moves to the new commit.

**Nothing else in this plan depends on the pin moving**, so the rest can
be done in parallel — but the corpus's switch-over (§4) should wait for
it, because the corpus's committed traces are the golden record and they
should not be regenerated twice.

---

## 1. `mlp-core`: the core's own suites use the harness

**What changes.** The core's `src/testing/` module goes away, and the
core's suites that drove it now drive the harness.

**The manifest.** The core gains a dev-dependency on the harness and a
patch that unifies the cycle:

```toml
# mlp-core/Cargo.toml
[dev-dependencies]
# The core's own suites drive the harness. The harness drives the core:
# the cycle runs through a dev-dependency, which cargo allows. The patch
# makes the harness's dependency on the core resolve to *this* core, so
# there is exactly one copy of the machines in the graph and the types
# match.
mlp-harness = { path = "../mlp-harness" }

[patch."https://github.com/medialaneprotocol/mlp-core"]
mlp-core = { path = "." }
```

The patch is what makes the cycle type-consistent: without it the
harness would link the *published* core while the core under test is the
local one, and the two copies' types would not match. The patch applies
only when the core is the workspace root, so a consumer of the published
core is unaffected.

**The files.**

| File | What happens |
|---|---|
| `src/testing.rs` | **deleted** |
| `src/testing/clock.rs` | **deleted** |
| `src/testing/lossy_link.rs` | **deleted** |
| `src/testing/scenario.rs` | **deleted** |
| `src/lib.rs` | the `testing` module and its re-exports go; the `testing` feature goes from `Cargo.toml` |
| `Cargo.toml` | the `testing` feature and its `regex`/`serde_json` dependencies go; the dev-dependencies gain the harness |
| `tests/media_scenarios.rs` | **deleted** — the scenarios live in the harness now |
| `tests/media_budget.rs` | rewritten against the harness's scenarios |
| `tests/media_pacer.rs`, `tests/media_bwe.rs` | unchanged (they are unit-level) |

**The core's new suite.** The core's own suites keep what is the *core's*
— the fragment codec, the assembly, the estimator, the pacer, the
scheduler, the accounting, the input queue, the resume machine — and the
*scenario-level* suites move to the harness. The core's suite that
remains and uses the harness is one that says: "the harness's canonical
scenario at this tier, driven over this core, holds this invariant".

**The order.** The manifest change lands first, with the `src/testing/`
module still present (the core's own tests still use it), so the core is
green at every commit. The `src/testing/` deletion lands second, together
with the suites that used it.

---

## 2. `mlp-core`: the `testing` feature and its dependencies

The `testing` feature exists only for the simulator. When `src/testing/`
goes, the feature goes with it, and so do the feature-only dependencies:

```toml
[features]
default = []
# (the `testing` feature goes)

[dependencies]
serde = { version = "1", features = ["derive"] }
# (regex and the optional serde_json go)
```

`serde_json` stays a dev-dependency if the core's own suites need it.

---

## 3. `mlp-harness`: what the corpus's generator becomes

The corpus's generator is a translation unit: it reads the corpus's
scenarios, translates them into the simulator's builder API, runs them and
writes the traces. After the switch-over the harness **is** the runner, and
the corpus's generator becomes a thin program that links the harness.

**The corpus's generator, after:**

```rust
// mlp-conformance/tools/corpus-gen.rs
use mlp_harness::scenario::Scenario;

fn main() {
    // Read the corpus's scenarios, run each through the harness, write the
    // trace. The harness's own `Scenario` type is the corpus's scenario
    // type, so the translation disappears.
}
```

**The corpus's scenario format.** The corpus's scenarios are JSON today
with their own step names (`Link`, `Advance`, `CaptureChange`,
`ViewerInput`, `Disconnect`, `Reconnect`, `SenderReconnect`, `Path`,
`Assert`). The harness's scenario format is JSON too, with the
harness's step names. The two are the same document after the rename:

| The corpus's step | The harness's step |
|---|---|
| `Link { profile }` | `SetLink { profile }` |
| `Advance` | `Advance { ms }` |
| `CaptureChange` | `CaptureChange { region }` |
| `ViewerInput` | `ViewerInput { event }` |
| `Disconnect` / `Reconnect` | `ViewerDisconnect` / `ViewerReconnect` |
| `SenderReconnect` | `HostReconnect` |
| `Path` | `PathSwitch { path }` |
| `Assert` | `Assert { invariant }` |

The corpus's `profiles/*.json` become the harness's `LinkProfile`
documents, and the corpus's `decisions/*.json` become the harness's
traces.

---

## 4. `mlp-conformance`: the corpus runs on the harness

**The gate's shape stays.** The corpus is a corpus and a gate; the gate
regenerates the traces and compares them byte for byte. What changes is
what does the regenerating.

| File | What happens |
|---|---|
| `tools/corpus-gen.rs` | rewritten to link the harness (see §3) |
| `tools/corpus_static.py` | **deleted** — the harness's own checks cover the shape |
| `scripts/conformance-gate.sh` | the generator step changes; the shape checks move to the harness's gate |
| `profiles/*.json` | converted to the harness's profile documents |
| `scenarios/*.json` | converted to the harness's scenario documents |
| `decisions/*.json` | regenerated with the harness and committed with the change that caused them |

**The gate after the switch:**

```bash
# The corpus's gate: the harness regenerates the traces, and the gate
# compares them with the committed ones.
(cd "$HARNESS_DIR" && cargo build --release)
"$HARNESS_DIR/target/release/mlp-harness" corpus \
    --scenarios "$CORPUS_DIR/scenarios" \
    --out "$LOGDIR/decisions"
diff -r "$LOGDIR/decisions" "$CORPUS_DIR/decisions"
```

The `corpus` subcommand is the one addition the harness needs: it runs
every scenario in a directory and writes each trace to an output
directory, in the corpus's `{"schema":1,"scenario":…,"digest":…,"trace":…}`
wrapper. That wrapper is the corpus's document format, and the harness
writes it because the corpus's documents are the golden record.

**The order.** The corpus's profiles and scenarios are converted first
(the harness reads them), then the traces are regenerated, then the
generator and the static validator are deleted.

---

## 5. `mlp-conformance`: the static validator's checks move

The corpus's static validator checks four things. Each has a home in the
harness:

| The validator's check | Where it goes |
|---|---|
| every profile is a canonical tier point | the harness's `LinkProfile::tier` is the canonical point, and the harness's gate asserts that the shipped profiles equal it |
| every scenario's steps parse | the harness's `Scenario::from_json` — the gate's scenario step is exactly this |
| the acceptance matrix's coverage is complete | the harness's shipped scenario list, and the gate's step that runs them all |
| every trace is clean | the harness's `run` returns a trace with its violations, and the gate requires them empty |

The coverage matrix itself becomes a list in the harness's gate: one line
per case, naming the scenario that covers it. The corpus keeps its
`profiles/`, `scenarios/` and `decisions/`; it loses its tools.

---

## 6. The product

The product consumes the harness by revision, exactly as it consumes the
core:

```toml
# the product's Cargo.toml
mlp-harness = { git = "https://github.com/medialialaneprotocol/mlp-harness", rev = "<revision>" }
```

The product uses the harness for what it is for: **the prediction is what
the netem runs are checked against**. The product's live netem runs
measure an echo's p95, a convergence time and a byte count on a shaped
link; the harness's `predict` command answers the same three questions for
the same profile, and the product compares the two.

The dependency runs one way: the product depends on the harness, and the
harness depends on the core, and nothing depends on the product.

---

## 7. The vocabulary

After the switch, the words **simulator**, **corpus generator** and
**static validator** appear nowhere: not in the code, not in the comments,
not in the documents, not in the commit messages. What remains is
**the harness**, **the corpus**, and **the gate**.

The transport's vocabulary is unchanged: **update**, **part**, **region**,
**op**, **cell**, **surface**, **reference**; the roles **sender**,
**receiver**, **intermediary**.

---

## 8. The list of every file that changes

**`mlp-core`**

```
Cargo.toml                              the feature, the dependencies, the dev-dependency, the patch
src/lib.rs                              the testing module and its re-exports go
src/testing.rs                          deleted
src/testing/clock.rs                    deleted
src/testing/lossy_link.rs               deleted
src/testing/scenario.rs                 deleted
tests/media_scenarios.rs                deleted
tests/media_budget.rs                   rewritten against the harness
tests/media_harness.rs                  new: the core's suite, driven by the harness
```

**`mlp-conformance`**

```
tools/corpus-gen.rs                     rewritten to link the harness
tools/corpus_static.py                  deleted
scripts/conformance-gate.sh             the generator step changes
profiles/*.json                         converted
scenarios/*.json                        converted
decisions/*.json                        regenerated
README.md                               the tools go; the harness is named
```

**`mlp-harness`**

```
Cargo.toml                              the pin moves (step 0)
src/lib.rs                              the `corpus` subcommand is added
src/bin/mlp-harness.rs                  the `corpus` subcommand is added
src/corpus.rs                           new: the corpus's document format
scenarios/*.json                        the converted corpus's scenarios
docs/design.md                          the corpus's stories are named
```

**The specification repository** (`mlp`)

```
spec/11-verification.md                 the L2 and L3 rows name the harness
scripts/verify.sh                       a step runs the harness's own checks
```

---

## 9. The order, as a list of commits

1. **`mlp-core`**: the manifest gains the harness's dev-dependency and the
   patch; the core's new suite uses the harness. The core is green.
2. **`mlp-harness`**: the pin moves to the fixed core; the digests are
   regenerated; the tag moves.
3. **`mlp-conformance`**: the profiles and the scenarios are converted; the
   traces are regenerated with the harness; the generator is rewritten; the
   static validator is deleted; the gate changes.
4. **`mlp-core`**: `src/testing/` is deleted, the `testing` feature goes, and
   the suites that used it are gone.
5. **the product**: the harness is added as a dependency and the netem runs
   are checked against its predictions.
6. **the specification**: the verification section names the harness, and
   the specification's gate runs the harness's own checks.

Every commit is green on its own, and the corpus's traces are regenerated
exactly once — in step 3.
