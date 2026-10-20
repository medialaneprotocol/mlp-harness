//! The carrier handover.
//!
//! A carrier handover is what a real handover costs: the objects in flight
//! at the switch are split between the old carrier and the new one, and some
//! of them are lost. A model that ignores this predicts a handover as a
//! change of delay, and the transport's behaviour under a handover is
//! nothing like that — it is a burst of loss followed by a re-measurement
//! of a path that is not the one it measured.
//!
//! The harness's rule, stated once:
//!
//! * the objects **already in the pipe** are split: `split_permille` of
//!   them continue on the old carrier, the rest continue on the new one;
//! * independently, `loss_permille` of them are lost at the switch;
//! * the objects **queued but not yet released** are released on the new
//!   carrier — they were not on the wire yet;
//! * the new carrier's profile applies to everything offered after the
//!   switch.

use serde::{Deserialize, Serialize};

use super::profile::LinkProfile;

/// A carrier change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handover {
    /// The new carrier's profile.
    pub profile: LinkProfile,
    /// The share of the objects in flight that stay on the old carrier, in
    /// permille.
    pub split_permille: u32,
    /// The share of the objects in flight the switch loses, in permille.
    pub loss_permille: u32,
}

impl Handover {
    /// A handover to a profile: half the objects in flight stay on the old
    /// carrier and none is lost.
    pub fn to(profile: LinkProfile) -> Self {
        Self {
            profile,
            split_permille: 500,
            loss_permille: 0,
        }
    }

    /// A handover that loses a share of the objects in flight.
    pub fn with_loss(mut self, loss_permille: u32) -> Self {
        self.loss_permille = loss_permille;
        self
    }

    /// A handover with a stated split.
    pub fn with_split(mut self, split_permille: u32) -> Self {
        self.split_permille = split_permille;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handover_carries_a_profile_and_two_shares() {
        let handover = Handover::to(LinkProfile::perfect())
            .with_split(250)
            .with_loss(100);
        assert_eq!(handover.split_permille, 250);
        assert_eq!(handover.loss_permille, 100);
    }
}
