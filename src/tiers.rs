//! The tier table: the protocol's target envelope, published as data.
//!
//! The tiers are the protocol's public contract: the rate, the round trip,
//! the loss, the jitter and the outage pattern a link may have and still be
//! one the protocol speaks about (MLP spec §2.1). The harness owns those
//! numbers — [`LinkProfile::tier`] holds each tier's canonical point, and
//! [`Tier::band`] holds the band it sits in — and this module publishes them
//! as a **table**: one document, machine-readable, that a consumer reads
//! instead of copying.
//!
//! # Why a table
//!
//! A consumer that needs a tier's numbers has three bad options and one
//! good one. It can read the source, which is not machine-readable; it can
//! ask the harness, which needs a toolchain; it can copy the numbers, which
//! drifts. The table is the good one: a single document, produced by the
//! harness that owns the numbers, and checked byte for byte by the corpus
//! that publishes it.
//!
//! The table is **generated**, never written by hand: the corpus's gate
//! regenerates it and compares it with the committed copy, exactly as it
//! does for the decision traces.
//!
//! # The row
//!
//! A row carries three things:
//!
//! * the tier's own numbers — the round trip, the echo budget the tier
//!   promises, and the loss bound;
//! * the tier's **band** — the range of links the tier speaks about;
//! * the tier's **canonical point** — the whole [`LinkProfile`], the link
//!   the harness asserts at.
//!
//! The canonical point is inside its own band, and the table says so: a
//! consumer can check the link it is on against the tier it names without
//! knowing anything else about the protocol.

use serde::{Deserialize, Serialize};

use crate::link::{LinkProfile, Tier, TierBand};

/// The document's schema.
pub const SCHEMA: u32 = 1;

/// One tier's row of the envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierRow {
    /// The tier's label, in the spelling a metric carries.
    pub tier: String,
    /// The tier's round trip, in milliseconds.
    pub rtt_ms: u64,
    /// The echo budget the tier promises, in milliseconds.
    pub echo_budget_ms: u64,
    /// The tier's loss bound, in permille.
    pub loss_permille: u32,
    /// The band of links the tier speaks about.
    pub band: TierBand,
    /// The tier's canonical point: the whole link profile.
    pub profile: LinkProfile,
}

/// The whole table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierTable {
    /// The document's schema.
    pub schema: u32,
    /// The rows, in tier order.
    pub tiers: Vec<TierRow>,
}

impl TierTable {
    /// The table as a JSON document.
    ///
    /// The document is byte-stable: the same table always serializes to the
    /// same bytes, so the corpus can commit it and compare it.
    pub fn to_json(&self) -> String {
        let mut document = serde_json::to_string_pretty(self).expect("the table is serializable");
        document.push('\n');
        document
    }

    /// The row of a tier.
    pub fn row(&self, tier: Tier) -> Option<&TierRow> {
        self.tiers.iter().find(|row| row.tier == tier.as_str())
    }
}

/// The table: every tier, in order.
pub fn table() -> TierTable {
    TierTable {
        schema: SCHEMA,
        tiers: Tier::all().iter().map(|tier| row(*tier)).collect(),
    }
}

/// One tier's row.
fn row(tier: Tier) -> TierRow {
    TierRow {
        tier: tier.as_str().to_string(),
        rtt_ms: tier.rtt_ms(),
        echo_budget_ms: tier.echo_budget_ms(),
        loss_permille: tier.loss_permille(),
        band: tier.band(),
        profile: LinkProfile::tier(tier),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Jitter;

    #[test]
    fn the_table_holds_every_tier_in_order() {
        let table = table();
        assert_eq!(table.tiers.len(), Tier::all().len());
        let labels: Vec<&str> = table.tiers.iter().map(|row| row.tier.as_str()).collect();
        assert_eq!(labels, vec!["t0", "t1", "t2", "t3", "t4"]);
    }

    #[test]
    fn a_row_carries_the_tier_own_numbers() {
        let table = table();
        let row = table.row(Tier::T2).expect("the table holds T2");
        assert_eq!(row.rtt_ms, 150);
        assert_eq!(row.echo_budget_ms, 550);
        assert_eq!(row.loss_permille, 20);
    }

    #[test]
    fn a_row_carries_the_canonical_point() {
        let table = table();
        for tier in Tier::all() {
            let row = table.row(tier).expect("the table holds the tier");
            assert_eq!(row.profile, LinkProfile::tier(tier));
        }
    }

    #[test]
    fn the_canonical_point_sits_inside_its_own_band() {
        let table = table();
        for tier in Tier::all() {
            let row = table.row(tier).expect("the table holds the tier");
            let band = row.band;
            assert!(
                row.profile.down.rate_bps >= band.rate_bps_down_min,
                "{}: the downlink is below the band",
                tier.as_str()
            );
            if let Some(max) = band.rate_bps_down_max {
                assert!(
                    row.profile.down.rate_bps <= max,
                    "{}: the downlink is above the band",
                    tier.as_str()
                );
            }
            assert!(
                row.profile.up.rate_bps >= band.rate_bps_up_min,
                "{}: the uplink is below the band",
                tier.as_str()
            );
            let rtt_ms = (row.profile.down.propagation_us + row.profile.up.propagation_us) / 1_000;
            assert!(
                rtt_ms >= band.rtt_ms_min && rtt_ms <= band.rtt_ms_max,
                "{}: the round trip {rtt_ms} ms is outside the band",
                tier.as_str()
            );
            assert!(
                row.loss_permille >= band.loss_permille_min
                    && row.loss_permille <= band.loss_permille_max,
                "{}: the loss bound is outside the band",
                tier.as_str()
            );
            let jitter_ms = row.profile.down.jitter.amplitude_us() / 1_000;
            assert!(
                jitter_ms >= band.jitter_ms_min && jitter_ms <= band.jitter_ms_max,
                "{}: the jitter {jitter_ms} ms is outside the band",
                tier.as_str()
            );
        }
    }

    #[test]
    fn the_round_trip_is_the_two_propagations_added() {
        let table = table();
        for tier in Tier::all() {
            let row = table.row(tier).expect("the table holds the tier");
            let rtt_ms = (row.profile.down.propagation_us + row.profile.up.propagation_us) / 1_000;
            assert_eq!(rtt_ms, row.rtt_ms, "{}", tier.as_str());
        }
    }

    #[test]
    fn the_table_serializes_the_same_bytes_every_time() {
        let first = table().to_json();
        let second = table().to_json();
        assert_eq!(first, second);
        assert!(first.ends_with('\n'));
    }

    #[test]
    fn the_tiers_carry_the_shapes_the_specification_names() {
        let table = table();
        let t4 = table.row(Tier::T4).expect("the table holds T4");
        assert_eq!(t4.profile.down.jitter, Jitter::pareto(500_000));
        assert_eq!(t4.profile.mtu_bytes, 1_150);
        assert_eq!(t4.band.flap_cycles, 5);
        assert_eq!(t4.band.flap_down_ms, 10_000);
        assert_eq!(t4.band.flap_up_ms, 10_000);
    }
}
