//! The checkers: the invariants, applied to what the harness observed.
//!
//! The checkers are **pure**. They read an [`Observation`] — a plain record
//! of what happened — and they know nothing about how it happened. That
//! separation is what makes the harness's own suite meaningful: a
//! deliberately broken peer corrupts the observation's *source*, and the
//! checker, which never saw the corruption, still fires.
//!
//! Every checker returns the detail of the violation, or `None` when the
//! invariant holds.

use std::collections::BTreeMap;

use crate::invariants::{Invariant, Outcome};
use crate::trace::Violation;

/// One cell's state, as the shadow reference and the receiver see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellState {
    /// The cell.
    pub cell: u32,
    /// The update id the sender emitted for it.
    pub emitted: u64,
    /// The update id the receiver holds for it.
    pub held: u64,
    /// The update id the receiver acknowledged for it.
    pub acknowledged: u64,
}

/// One update the receiver applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedUpdate {
    /// The update's id.
    pub update_id: u64,
    /// How many parts the update had.
    pub part_count: u16,
    /// How many of its parts had arrived when it was applied.
    pub parts_present: u16,
}

/// One input the sender applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputApplication {
    /// The input's sequence number.
    pub seq: u64,
    /// When it was applied, in microseconds.
    pub applied_at_us: u64,
    /// Its deadline, in microseconds. Zero means it never expires.
    pub deadline_us: u64,
    /// Whether the input is always applied, deadline or not.
    pub always_applied: bool,
}

/// One queue's depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueDepth {
    /// The queue's name.
    pub name: String,
    /// How deep it is.
    pub depth: u64,
    /// How deep it may be.
    pub cap: u64,
}

/// One object's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectSize {
    /// The object's size, in bytes.
    pub bytes: u64,
    /// The cap it must not exceed.
    pub cap: u64,
}

/// One update's part count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateParts {
    /// How many parts the update had.
    pub parts: u16,
    /// How many parts the carrier admits.
    pub cap: u16,
}

/// One byte release, for the one-second window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    /// When the bytes were released, in microseconds.
    pub at_us: u64,
    /// How many bytes.
    pub bytes: u64,
    /// The window budget in force when they were released.
    pub budget_bytes: u64,
}

/// What the harness observed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    /// The time of the observation, in microseconds.
    pub now_us: u64,
    /// The updates the receiver applied.
    pub applied_updates: Vec<AppliedUpdate>,
    /// The cells' states.
    pub cells: Vec<CellState>,
    /// The inputs the sender applied.
    pub input_applications: Vec<InputApplication>,
    /// The tokens the sender believes are held.
    pub sender_pressed: Vec<u32>,
    /// The tokens the receiver believes are held.
    pub receiver_pressed: Vec<u32>,
    /// Whether the receiver is parked.
    pub parked: bool,
    /// Whether the session has ended.
    pub session_ended: bool,
    /// The queues' depths.
    pub queues: Vec<QueueDepth>,
    /// The objects' sizes.
    pub objects: Vec<ObjectSize>,
    /// The updates' part counts.
    pub update_parts: Vec<UpdateParts>,
    /// The byte releases, in order.
    pub releases: Vec<Release>,
    /// The budget for one second of releases.
    pub window_budget_bytes: u64,
    /// The measured echoes, in milliseconds.
    pub echoes: Vec<u64>,
    /// The echo budget.
    pub echo_budget_ms: u64,
    /// Whether the tier's echo promise applies here.
    pub echo_promised: bool,
    /// How many lanes the session created for the receiver.
    pub lanes_created: u32,
    /// Whether the session was lost.
    pub session_lost: bool,
    /// How many objects the link lost on the media path.
    pub link_dropped: u64,
    /// How many objects the sender dropped before the link saw them.
    pub sender_dropped: u64,
    /// How many objects the hops reported dropping.
    pub hop_dropped: u64,
    /// How many objects the receiver reported as lost.
    pub receiver_reported_lost: u64,
    /// How many objects the receiver saw out of order.
    pub receiver_reordered: u64,
}

/// The checker: it counts the checks and the violations.
#[derive(Debug, Clone, Default)]
pub struct Checker {
    outcomes: BTreeMap<Invariant, Outcome>,
    violations: Vec<Violation>,
}

impl Checker {
    /// A checker with no checks yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check one invariant against an observation.
    ///
    /// Returns whether the invariant held.
    pub fn check(&mut self, invariant: Invariant, observation: &Observation) -> bool {
        let detail = match invariant {
            Invariant::Inv1 => check_inv1(observation),
            Invariant::Inv2 => check_inv2(observation),
            Invariant::Inv3 => check_inv3(observation),
            Invariant::Inv4 => check_inv4(observation),
            Invariant::Inv5 => check_inv5(observation),
            Invariant::Inv6 => check_inv6(observation),
            Invariant::Inv7 => check_inv7(observation),
            Invariant::Inv8 => check_inv8(observation),
            Invariant::Inv9 => check_inv9(observation),
            Invariant::Inv10 => check_inv10(observation),
        };
        let entry = self.outcomes.entry(invariant).or_insert(Outcome {
            invariant,
            version: invariant.version(),
            checks: 0,
            violations: 0,
            holds: true,
        });
        entry.checks += 1;
        match detail {
            None => true,
            Some(detail) => {
                entry.violations += 1;
                entry.holds = false;
                // One violation per invariant is enough: a broken invariant
                // floods the trace otherwise, and the flood is noise.
                let first = !self
                    .violations
                    .iter()
                    .any(|violation| violation.invariant == invariant);
                if first {
                    self.violations.push(Violation {
                        at_us: observation.now_us,
                        invariant,
                        detail,
                    });
                }
                false
            }
        }
    }

    /// The outcome of every invariant that was checked.
    pub fn outcomes(&self) -> Vec<Outcome> {
        self.outcomes.values().cloned().collect()
    }

    /// Everything the checker saw.
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// Whether the run was clean.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// INV-1: an update is applied only if every part of it arrived.
fn check_inv1(observation: &Observation) -> Option<String> {
    for applied in &observation.applied_updates {
        if applied.parts_present < applied.part_count {
            return Some(format!(
                "update {} was applied with {}/{} parts",
                applied.update_id, applied.parts_present, applied.part_count
            ));
        }
    }
    None
}

/// INV-2: at a converged observation the receiver's surface equals the
/// sender's emitted state for every cell the receiver acknowledged.
fn check_inv2(observation: &Observation) -> Option<String> {
    for cell in &observation.cells {
        if cell.acknowledged == 0 {
            continue;
        }
        if cell.emitted < cell.acknowledged {
            return Some(format!(
                "cell {} acknowledged update {} the sender never emitted",
                cell.cell, cell.acknowledged
            ));
        }
        if cell.held < cell.acknowledged {
            return Some(format!(
                "cell {} acknowledged update {} but holds {}",
                cell.cell, cell.acknowledged, cell.held
            ));
        }
    }
    None
}

/// INV-3: no input is applied twice, and none after its deadline.
fn check_inv3(observation: &Observation) -> Option<String> {
    let mut seen: Vec<u64> = Vec::new();
    for application in &observation.input_applications {
        if seen.contains(&application.seq) {
            return Some(format!("input {} was applied twice", application.seq));
        }
        seen.push(application.seq);
        if !application.always_applied
            && application.deadline_us != 0
            && application.applied_at_us > application.deadline_us
        {
            return Some(format!(
                "input {} was applied {} µs after its deadline",
                application.seq,
                application.applied_at_us - application.deadline_us
            ));
        }
    }
    None
}

/// INV-4: after a park or a session end, the pressed set is empty.
///
/// The invariant is about the *end* of a session, not about a release that
/// the link lost: a lost key release leaves the key held until the
/// receiver's next pressed set repairs it, which is the protocol's answer
/// (MLP spec §8.3.4), and the harness reports that window as a
/// measurement rather than as a violation.
fn check_inv4(observation: &Observation) -> Option<String> {
    if observation.parked || observation.session_ended {
        if let Some(token) = observation.sender_pressed.first() {
            return Some(format!(
                "the sender still holds {token} after the park or the session end"
            ));
        }
    }
    None
}

/// INV-5: no queue exceeds its cap.
fn check_inv5(observation: &Observation) -> Option<String> {
    for queue in &observation.queues {
        if queue.depth > queue.cap {
            return Some(format!(
                "the {} queue is {} deep, above its cap of {}",
                queue.name, queue.depth, queue.cap
            ));
        }
    }
    None
}

/// INV-6: no object exceeds its cap, and no update exceeds its part cap.
fn check_inv6(observation: &Observation) -> Option<String> {
    for object in &observation.objects {
        if object.bytes > object.cap {
            return Some(format!(
                "an object of {} bytes exceeds the cap of {}",
                object.bytes, object.cap
            ));
        }
    }
    for update in &observation.update_parts {
        if update.parts > update.cap {
            return Some(format!(
                "an update of {} parts exceeds the carrier's cap of {}",
                update.parts, update.cap
            ));
        }
    }
    None
}

/// INV-7: the bytes sent in any one-second window stay inside the budget.
fn check_inv7(observation: &Observation) -> Option<String> {
    let window_us = 1_000_000u64;
    let mut total = 0u64;
    let mut budget = observation.window_budget_bytes;
    for release in &observation.releases {
        if release.at_us.saturating_add(window_us) > observation.now_us {
            total = total.saturating_add(release.bytes);
            // The budget in force when the last bytes of the window left.
            budget = release.budget_bytes;
        }
    }
    if total > budget {
        return Some(format!(
            "{total} bytes in a one-second window above the {budget} byte budget"
        ));
    }
    None
}

/// INV-8: the echo's p95 stays inside the budget.
fn check_inv8(observation: &Observation) -> Option<String> {
    if !observation.echo_promised || observation.echoes.is_empty() {
        return None;
    }
    let mut samples = observation.echoes.clone();
    samples.sort_unstable();
    let index = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    let p95 = samples[index.min(samples.len() - 1)];
    if p95 > observation.echo_budget_ms {
        return Some(format!(
            "the echo's p95 is {p95} ms, above the {} ms budget",
            observation.echo_budget_ms
        ));
    }
    None
}

/// INV-9: a session survives every outage and flap schedule.
fn check_inv9(observation: &Observation) -> Option<String> {
    if observation.session_lost {
        return Some("the session was lost".to_string());
    }
    if observation.lanes_created > 1 {
        return Some(format!(
            "{} lanes were created for one receiver",
            observation.lanes_created
        ));
    }
    None
}

/// INV-10: every drop is attributed to exactly one hop.
///
/// The check is one-directional, and deliberately so. The receiver's loss
/// report comes from gap detection on the object id (MLP spec §6.2.2), and
/// the receiver cannot know about an object that is still in flight, so it
/// may report *less* than the hops lost: the difference is the objects on
/// the wire. It may not report *more*, though: a loss nobody accounts for
/// is a loss the transport cannot repair and cannot explain.
fn check_inv10(observation: &Observation) -> Option<String> {
    let attributed =
        observation.link_dropped + observation.sender_dropped + observation.hop_dropped;
    let reported = observation.receiver_reported_lost;
    if reported > attributed + 2 {
        return Some(format!(
            "{reported} objects lost, {attributed} attributed to a hop"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> Observation {
        Observation {
            now_us: 1_000,
            applied_updates: vec![AppliedUpdate {
                update_id: 1,
                part_count: 2,
                parts_present: 2,
            }],
            cells: vec![CellState {
                cell: 3,
                emitted: 5,
                held: 5,
                acknowledged: 5,
            }],
            input_applications: vec![InputApplication {
                seq: 1,
                applied_at_us: 100,
                deadline_us: 1_000_000,
                always_applied: false,
            }],
            sender_pressed: vec![65],
            receiver_pressed: vec![65],
            parked: false,
            session_ended: false,
            queues: vec![QueueDepth {
                name: "sender".to_string(),
                depth: 2,
                cap: 64,
            }],
            objects: vec![ObjectSize {
                bytes: 100,
                cap: 1_150,
            }],
            update_parts: vec![UpdateParts { parts: 1, cap: 64 }],
            releases: vec![Release {
                at_us: 0,
                bytes: 100,
                budget_bytes: 1_000,
            }],
            window_budget_bytes: 1_000,
            echoes: vec![100],
            echo_budget_ms: 550,
            echo_promised: true,
            lanes_created: 1,
            session_lost: false,
            link_dropped: 2,
            sender_dropped: 0,
            hop_dropped: 1,
            receiver_reordered: 0,
            receiver_reported_lost: 3,
        }
    }

    #[test]
    fn a_clean_observation_holds_every_invariant() {
        let observation = clean();
        let mut checker = Checker::new();
        for invariant in Invariant::all() {
            assert!(checker.check(invariant, &observation), "{invariant} fired");
        }
        assert!(checker.is_clean());
        assert_eq!(checker.outcomes().len(), 10);
    }

    #[test]
    fn a_partial_update_trips_inv1() {
        let mut observation = clean();
        observation.applied_updates[0].parts_present = 1;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv1, &observation));
        assert_eq!(checker.violations().len(), 1);
    }

    #[test]
    fn an_unemitted_cell_trips_inv2() {
        let mut observation = clean();
        observation.cells[0].emitted = 0;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv2, &observation));
    }

    #[test]
    fn a_cell_the_receiver_does_not_hold_trips_inv2() {
        let mut observation = clean();
        observation.cells[0].held = 4;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv2, &observation));
    }

    #[test]
    fn a_twice_applied_input_trips_inv3() {
        let mut observation = clean();
        let application = observation.input_applications[0].clone();
        observation.input_applications.push(application);
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv3, &observation));
    }

    #[test]
    fn a_late_input_trips_inv3() {
        let mut observation = clean();
        observation.input_applications[0].applied_at_us = 2_000_000;
        observation.input_applications[0].deadline_us = 1_000_000;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv3, &observation));
    }

    #[test]
    fn a_stuck_key_trips_inv4() {
        let mut observation = clean();
        observation.parked = true;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv4, &observation));
    }

    #[test]
    fn a_stuck_key_after_a_park_trips_inv4() {
        let mut observation = clean();
        observation.parked = true;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv4, &observation));
    }

    #[test]
    fn a_deep_queue_trips_inv5() {
        let mut observation = clean();
        observation.queues[0].depth = 65;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv5, &observation));
    }

    #[test]
    fn an_oversize_object_trips_inv6() {
        let mut observation = clean();
        observation.objects[0].bytes = 2_000;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv6, &observation));
    }

    #[test]
    fn too_many_parts_trips_inv6() {
        let mut observation = clean();
        observation.update_parts[0].parts = 100;
        observation.update_parts[0].cap = 64;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv6, &observation));
    }

    #[test]
    fn a_burst_over_the_budget_trips_inv7() {
        let mut observation = clean();
        observation.releases = vec![
            Release {
                at_us: 0,
                bytes: 900,
                budget_bytes: 1_000,
            },
            Release {
                at_us: 100,
                bytes: 900,
                budget_bytes: 1_000,
            },
        ];
        observation.window_budget_bytes = 1_000;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv7, &observation));
    }

    #[test]
    fn a_slow_echo_trips_inv8() {
        let mut observation = clean();
        observation.echoes = vec![900];
        observation.echo_budget_ms = 550;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv8, &observation));
    }

    #[test]
    fn a_lost_session_trips_inv9() {
        let mut observation = clean();
        observation.session_lost = true;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv9, &observation));
    }

    #[test]
    fn a_second_lane_trips_inv9() {
        let mut observation = clean();
        observation.lanes_created = 2;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv9, &observation));
    }

    #[test]
    fn an_unattributed_drop_trips_inv10() {
        let mut observation = clean();
        observation.link_dropped = 0;
        observation.hop_dropped = 0;
        observation.receiver_reported_lost = 5;
        let mut checker = Checker::new();
        assert!(!checker.check(Invariant::Inv10, &observation));
    }
}
