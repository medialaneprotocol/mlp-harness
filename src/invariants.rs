//! The invariants: named, versioned, and stated in one place.
//!
//! An invariant here is a **definition** and a **prohibition**. The
//! definition says what must hold; the prohibition says what the harness
//! must never see. Both live in this table, so there is exactly one place
//! where the protocol's promises are written down, and the harness's
//! checkers and the harness's suite read them from here.
//!
//! The set is the protocol's (MLP spec §11.2):
//!
//! | id | version | invariant |
//! |---|---|---|
//! | INV-1 | 1 | an update is applied only whole |
//! | INV-2 | 1 | at a converged observation the receiver's surface equals the sender's emitted state |
//! | INV-3 | 1 | no input is applied twice, and none after its deadline |
//! | INV-4 | 1 | the pressed set is empty after a park or a session end |
//! | INV-5 | 1 | no queue exceeds its cap |
//! | INV-6 | 1 | no object exceeds its cap |
//! | INV-7 | 1 | the bytes sent in any one-second window stay inside the budget |
//! | INV-8 | 1 | the input echo stays inside the tier's budget |
//! | INV-9 | 1 | a session survives every outage and flap schedule |
//! | INV-10 | 1 | every drop is attributed to exactly one hop |
//!
//! # A version
//!
//! Each invariant carries a version. A change to what an invariant *means*
//! — not to how it is checked — is a version bump, and the harness records
//! the version in every trace, so a trace taken before the bump and a trace
//! taken after it are never silently compared.

use serde::{Deserialize, Serialize};

/// The protocol's invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Invariant {
    /// An update is applied only if every part of it arrived.
    #[serde(rename = "INV-1")]
    Inv1,
    /// At a converged observation the receiver's surface equals the sender's
    /// emitted state for every cell the receiver acknowledged.
    #[serde(rename = "INV-2")]
    Inv2,
    /// No input is applied twice, and none after its deadline.
    #[serde(rename = "INV-3")]
    Inv3,
    /// After a park beyond the grace period or a session end, the pressed set
    /// is empty.
    #[serde(rename = "INV-4")]
    Inv4,
    /// Every queue stays within its cap.
    #[serde(rename = "INV-5")]
    Inv5,
    /// No object exceeds `max_object_bytes`, and no update exceeds the
    /// carrier's part cap.
    #[serde(rename = "INV-6")]
    Inv6,
    /// The bytes sent in any one-second window stay inside the budget.
    #[serde(rename = "INV-7")]
    Inv7,
    /// The input echo stays inside `2 × RTT + 250 ms`.
    #[serde(rename = "INV-8")]
    Inv8,
    /// A session survives every outage and flap schedule.
    #[serde(rename = "INV-9")]
    Inv9,
    /// Every drop is attributed to exactly one hop.
    #[serde(rename = "INV-10")]
    Inv10,
}

impl Invariant {
    /// The invariant's id.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Inv1 => "INV-1",
            Self::Inv2 => "INV-2",
            Self::Inv3 => "INV-3",
            Self::Inv4 => "INV-4",
            Self::Inv5 => "INV-5",
            Self::Inv6 => "INV-6",
            Self::Inv7 => "INV-7",
            Self::Inv8 => "INV-8",
            Self::Inv9 => "INV-9",
            Self::Inv10 => "INV-10",
        }
    }

    /// The invariant's version.
    pub const fn version(self) -> u32 {
        1
    }

    /// What must hold.
    pub const fn statement(self) -> &'static str {
        match self {
            Self::Inv1 => {
                "An update is applied only if every part of it arrived; a partial update is never applied."
            }
            Self::Inv2 => {
                "At every converged observation, the receiver's surface equals the sender's emitted state for every cell the receiver acknowledged."
            }
            Self::Inv3 => {
                "No input object is applied twice, and none after its deadline — releases excepted, and asserted applied."
            }
            Self::Inv4 => {
                "After any session end or park beyond the grace period, the sender's pressed set is empty: no stuck keys."
            }
            Self::Inv5 => {
                "Every queue — the sender's, the intermediaries', the receiver's — stays within its cap under any profile."
            }
            Self::Inv6 => {
                "No object exceeds max_object_bytes, and no update exceeds max_parts times the carrier's fragment cap."
            }
            Self::Inv7 => {
                "The bytes sent in any one-second window are at most pace_bps/8 times 1.25; probes are accounted separately."
            }
            Self::Inv8 => {
                "The input echo is at most 2 times RTT plus 250 ms whenever the link delivers any object."
            }
            Self::Inv9 => {
                "A session survives every outage and flap schedule with no lost session and exactly one resumed lane per receiver."
            }
            Self::Inv10 => {
                "Every drop is attributed to exactly one hop: the sum of the hop reports and the link loss equals the receiver-reported loss, within one object."
            }
        }
    }

    /// What the harness must never see.
    pub const fn forbids(self) -> &'static str {
        match self {
            Self::Inv1 => "an update applied while one of its parts is missing.",
            Self::Inv2 => {
                "a converged observation in which the receiver holds a cell the sender never emitted."
            }
            Self::Inv3 => "an input applied twice, or applied after its deadline.",
            Self::Inv4 => "a key the receiver released that the sender still holds.",
            Self::Inv5 => "a queue deeper than its cap.",
            Self::Inv6 => {
                "an object larger than max_object_bytes, or an update with more parts than the carrier admits."
            }
            Self::Inv7 => "more bytes released in a one-second window than the budget allows.",
            Self::Inv8 => "an echo series whose p95 exceeds the tier's budget.",
            Self::Inv9 => "a session lost to an outage, or a second lane created for one receiver.",
            Self::Inv10 => "a drop that no hop accounts for, or a drop two hops account for.",
        }
    }

    /// Every invariant, in order.
    pub const fn all() -> [Invariant; 10] {
        [
            Self::Inv1,
            Self::Inv2,
            Self::Inv3,
            Self::Inv4,
            Self::Inv5,
            Self::Inv6,
            Self::Inv7,
            Self::Inv8,
            Self::Inv9,
            Self::Inv10,
        ]
    }

    /// The invariant with the given number, one-based.
    pub fn from_number(number: usize) -> Option<Self> {
        Self::all().get(number.checked_sub(1)?).copied()
    }

    /// The invariants the harness checks on **every tick**.
    ///
    /// These are the ones a state, not a moment, decides: a queue's depth,
    /// an object's size, a window's bytes, a cell's state. The others —
    /// the echo, the session's continuity, the loss attribution — depend on
    /// a moment in the run, and are checked where the scenario asks for
    /// them.
    pub const fn cheap() -> [Invariant; 6] {
        [
            Self::Inv1,
            Self::Inv2,
            Self::Inv3,
            Self::Inv5,
            Self::Inv6,
            Self::Inv7,
        ]
    }

    /// Whether the harness checks the invariant on every tick.
    pub const fn is_cheap(self) -> bool {
        matches!(
            self,
            Self::Inv1 | Self::Inv2 | Self::Inv3 | Self::Inv5 | Self::Inv6 | Self::Inv7
        )
    }
}

impl std::fmt::Display for Invariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// One invariant's definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Definition {
    /// The invariant.
    pub invariant: Invariant,
    /// Its version.
    pub version: u32,
    /// What must hold.
    pub statement: &'static str,
    /// What the harness must never see.
    pub forbids: &'static str,
}

/// Every definition, in one table.
pub const DEFINITIONS: [Definition; 10] = [
    Definition {
        invariant: Invariant::Inv1,
        version: Invariant::Inv1.version(),
        statement: Invariant::Inv1.statement(),
        forbids: Invariant::Inv1.forbids(),
    },
    Definition {
        invariant: Invariant::Inv2,
        version: Invariant::Inv2.version(),
        statement: Invariant::Inv2.statement(),
        forbids: Invariant::Inv2.forbids(),
    },
    Definition {
        invariant: Invariant::Inv3,
        version: Invariant::Inv3.version(),
        statement: Invariant::Inv3.statement(),
        forbids: Invariant::Inv3.forbids(),
    },
    Definition {
        invariant: Invariant::Inv4,
        version: Invariant::Inv4.version(),
        statement: Invariant::Inv4.statement(),
        forbids: Invariant::Inv4.forbids(),
    },
    Definition {
        invariant: Invariant::Inv5,
        version: Invariant::Inv5.version(),
        statement: Invariant::Inv5.statement(),
        forbids: Invariant::Inv5.forbids(),
    },
    Definition {
        invariant: Invariant::Inv6,
        version: Invariant::Inv6.version(),
        statement: Invariant::Inv6.statement(),
        forbids: Invariant::Inv6.forbids(),
    },
    Definition {
        invariant: Invariant::Inv7,
        version: Invariant::Inv7.version(),
        statement: Invariant::Inv7.statement(),
        forbids: Invariant::Inv7.forbids(),
    },
    Definition {
        invariant: Invariant::Inv8,
        version: Invariant::Inv8.version(),
        statement: Invariant::Inv8.statement(),
        forbids: Invariant::Inv8.forbids(),
    },
    Definition {
        invariant: Invariant::Inv9,
        version: Invariant::Inv9.version(),
        statement: Invariant::Inv9.statement(),
        forbids: Invariant::Inv9.forbids(),
    },
    Definition {
        invariant: Invariant::Inv10,
        version: Invariant::Inv10.version(),
        statement: Invariant::Inv10.statement(),
        forbids: Invariant::Inv10.forbids(),
    },
];

/// One invariant's outcome in a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// The invariant.
    pub invariant: Invariant,
    /// Its version at the time of the run.
    pub version: u32,
    /// How many times it was checked.
    pub checks: u64,
    /// How many times it was violated.
    pub violations: u64,
    /// Whether it held.
    pub holds: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_ten_invariants_and_they_are_numbered_in_order() {
        assert_eq!(Invariant::all().len(), 10);
        for (index, invariant) in Invariant::all().into_iter().enumerate() {
            assert_eq!(invariant.id(), format!("INV-{}", index + 1));
            assert_eq!(Invariant::from_number(index + 1), Some(invariant));
        }
        assert_eq!(Invariant::from_number(0), None);
        assert_eq!(Invariant::from_number(11), None);
    }

    #[test]
    fn every_invariant_states_itself_and_its_prohibition() {
        for definition in DEFINITIONS {
            assert!(!definition.statement.is_empty());
            assert!(!definition.forbids.is_empty());
            assert_eq!(definition.version, definition.invariant.version());
        }
    }

    #[test]
    fn the_definitions_cover_every_invariant_exactly_once() {
        for invariant in Invariant::all() {
            let count = DEFINITIONS
                .iter()
                .filter(|definition| definition.invariant == invariant)
                .count();
            assert_eq!(count, 1, "{invariant} appears {count} times");
        }
    }
}
