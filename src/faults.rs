//! The faults: a deliberately broken peer.
//!
//! An invariant that nothing can violate is decoration. The harness
//! therefore carries a **fault injector**: a set of switches, each of which
//! makes the peer misbehave in exactly one stated way, and each of which
//! exists so that one invariant can be shown to bite.
//!
//! A fault is not a mode of the harness's *model*. It is a defect in the
//! *implementation under test* — the peer that misbehaves. The harness's
//! link, clock and machines stay honest; only the peer's behaviour changes.
//!
//! Each fault names the invariant it is meant to trip, so the suite can
//! state the pairing once and check it.

use serde::{Deserialize, Serialize};

use crate::invariants::Invariant;

/// A set of deliberate defects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Faults {
    /// The peer applies an update whose parts have not all arrived.
    pub apply_partial_update: bool,
    /// The peer acknowledges a cell the sender never emitted.
    pub acknowledge_unemitted_cell: bool,
    /// The peer applies the same input twice.
    pub apply_input_twice: bool,
    /// The peer applies an input after its deadline.
    pub apply_input_after_deadline: bool,
    /// The peer keeps a key held after the receiver released it.
    pub keep_key_held_after_release: bool,
    /// The peer's queue grows past its cap.
    pub unbounded_queue: bool,
    /// The peer puts an object on the link that is above the cap.
    pub oversize_object: bool,
    /// The peer releases more bytes in a window than the budget allows.
    pub burst_over_budget: bool,
    /// The peer delays its acknowledgement past the echo budget.
    pub delay_echo: bool,
    /// The peer recreates the session after an outage.
    pub recreate_session: bool,
    /// The peer loses an object without any hop accounting for it.
    pub unattributed_drop: bool,
    /// A garbled parity object passes the peer's wire: its XOR body no
    /// longer checks out against the group's CRC (MLP spec §6.9).
    pub parity_lies: bool,
}

impl Faults {
    /// No faults at all.
    pub fn none() -> Self {
        Self::default()
    }

    /// Every fault, on.
    pub fn all() -> Self {
        Self {
            apply_partial_update: true,
            acknowledge_unemitted_cell: true,
            apply_input_twice: true,
            apply_input_after_deadline: true,
            keep_key_held_after_release: true,
            unbounded_queue: true,
            oversize_object: true,
            burst_over_budget: true,
            delay_echo: true,
            recreate_session: true,
            unattributed_drop: true,
            parity_lies: true,
        }
    }

    /// The fault that trips an invariant.
    pub fn for_invariant(invariant: Invariant) -> Self {
        let mut faults = Self::none();
        match invariant {
            Invariant::Inv1 => faults.apply_partial_update = true,
            Invariant::Inv2 => faults.acknowledge_unemitted_cell = true,
            Invariant::Inv3 => faults.apply_input_twice = true,
            Invariant::Inv4 => faults.keep_key_held_after_release = true,
            Invariant::Inv5 => faults.unbounded_queue = true,
            Invariant::Inv6 => faults.oversize_object = true,
            Invariant::Inv7 => faults.burst_over_budget = true,
            Invariant::Inv8 => faults.delay_echo = true,
            Invariant::Inv9 => faults.recreate_session = true,
            Invariant::Inv10 => faults.unattributed_drop = true,
            Invariant::Inv11 => faults.parity_lies = true,
        }
        faults
    }

    /// The faults that are on, by name.
    pub fn active(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.apply_partial_update {
            names.push("apply-partial-update");
        }
        if self.acknowledge_unemitted_cell {
            names.push("acknowledge-unemitted-cell");
        }
        if self.apply_input_twice {
            names.push("apply-input-twice");
        }
        if self.apply_input_after_deadline {
            names.push("apply-input-after-deadline");
        }
        if self.keep_key_held_after_release {
            names.push("keep-key-held-after-release");
        }
        if self.unbounded_queue {
            names.push("unbounded-queue");
        }
        if self.oversize_object {
            names.push("oversize-object");
        }
        if self.burst_over_budget {
            names.push("burst-over-budget");
        }
        if self.delay_echo {
            names.push("delay-echo");
        }
        if self.recreate_session {
            names.push("recreate-session");
        }
        if self.unattributed_drop {
            names.push("unattributed-drop");
        }
        if self.parity_lies {
            names.push("parity-lies");
        }
        names
    }

    /// Whether any fault is on.
    pub fn any(&self) -> bool {
        !self.active().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_faults_is_no_faults() {
        assert!(!Faults::none().any());
        assert!(Faults::none().active().is_empty());
    }

    #[test]
    fn every_invariant_has_a_fault_that_trips_it() {
        for invariant in Invariant::all() {
            let faults = Faults::for_invariant(invariant);
            assert!(faults.any(), "{invariant} has no fault");
            assert_eq!(faults.active().len(), 1);
        }
    }

    #[test]
    fn every_fault_is_listed_when_it_is_on() {
        assert_eq!(Faults::all().active().len(), 12);
    }
}
