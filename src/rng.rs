//! The seeded generator.
//!
//! The harness has no entropy. Every random quantity it needs — a jitter
//! sample, a loss decision, a reordering displacement, the order of a
//! randomised scenario — comes from a generator the scenario carries, seeded
//! by the scenario. The same seed and the same scenario produce the same
//! draws in the same order, on every machine.
//!
//! # The generator
//!
//! SplitMix64: a 64-bit state, an odd increment, and a Weyl step. It is not
//! a cipher and does not need to be — what it needs to be is *reproducible*,
//! *fast*, and *free of structure the model would react to*. Its output is
//! uniform over the whole 64-bit range, which is what the model's
//! probability tests need.
//!
//! Every draw the model makes is taken from one stream, in a fixed order, so
//! that adding a draw in one place shifts the whole sequence. The stream
//! therefore exposes its draw count: a trace that records the count is
//! sensitive to a change in *how many* draws happened, not only in what they
//! produced.

use serde::{Deserialize, Serialize};

/// A seeded, reproducible generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generator {
    state: u64,
    draws: u64,
}

impl Generator {
    /// A generator for a seed. Every seed is valid, including zero.
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed,
            draws: 0,
        }
    }

    /// How many draws have been taken. A determinism fingerprint.
    pub fn draws(&self) -> u64 {
        self.draws
    }

    /// The next 64-bit draw.
    pub fn next_u64(&mut self) -> u64 {
        self.draws += 1;
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The next 32-bit draw.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A uniform draw in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        // 53 bits is the most a `f64` represents exactly.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A uniform draw in `[0, bound)`, or zero when the bound is zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next_u64() % bound
    }

    /// A uniform draw in `[low, high]`.
    pub fn range_us(&mut self, low: u64, high: u64) -> u64 {
        if high <= low {
            return low;
        }
        low + self.below(high - low + 1)
    }

    /// Whether an event with the given probability in permille happens.
    pub fn chance_permille(&mut self, permille: u32) -> bool {
        if permille == 0 {
            return false;
        }
        if permille >= 1_000 {
            return true;
        }
        self.below(1_000) < u64::from(permille)
    }

    /// A standard-normal draw via Box–Muller.
    ///
    /// The cosine branch is taken, so one draw of the pair is spent per
    /// sample; the draw count is part of the fingerprint and the count is
    /// therefore stable.
    pub fn normal(&mut self) -> f64 {
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// A Pareto draw with the given shape, scaled by `scale`.
    ///
    /// The shape is 1.5, the tail that models a radio link. The draw is
    /// shifted so that it starts at zero rather than at the scale.
    pub fn pareto(&mut self, scale: f64) -> f64 {
        let u = self.next_f64().max(f64::MIN_POSITIVE);
        scale * (u.powf(-1.0 / 1.5) - 1.0)
    }

    /// An exponential draw with the given mean.
    pub fn exponential(&mut self, mean: f64) -> f64 {
        let u = self.next_f64().max(f64::MIN_POSITIVE);
        -mean * u.ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut first = Generator::new(42);
        let mut second = Generator::new(42);
        for _ in 0..100 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
        assert_eq!(first.draws(), second.draws());
    }

    #[test]
    fn different_seeds_give_different_sequences() {
        let mut first = Generator::new(1);
        let mut second = Generator::new(2);
        assert_ne!(first.next_u64(), second.next_u64());
    }

    #[test]
    fn the_draw_count_is_a_fingerprint() {
        let mut generator = Generator::new(7);
        assert_eq!(generator.draws(), 0);
        generator.next_u64();
        generator.chance_permille(1);
        assert_eq!(generator.draws(), 2);
    }

    #[test]
    fn a_chance_is_close_to_its_probability() {
        let mut generator = Generator::new(11);
        let mut hits = 0;
        for _ in 0..10_000 {
            if generator.chance_permille(250) {
                hits += 1;
            }
        }
        assert!((2_200..=2_800).contains(&hits), "250‰ hit {hits}/10000");
    }

    #[test]
    fn a_chance_at_the_bounds_is_exact() {
        let mut generator = Generator::new(3);
        assert!(!generator.chance_permille(0));
        assert!(generator.chance_permille(1_000));
    }

    #[test]
    fn a_uniform_draw_stays_in_its_range() {
        let mut generator = Generator::new(5);
        for _ in 0..1_000 {
            let value = generator.next_f64();
            assert!((0.0..1.0).contains(&value));
        }
    }

    #[test]
    fn a_range_draw_stays_in_its_range() {
        let mut generator = Generator::new(9);
        for _ in 0..1_000 {
            let value = generator.range_us(100, 200);
            assert!((100..=200).contains(&value));
        }
    }

    #[test]
    fn a_normal_draw_has_the_shape_it_claims() {
        let mut generator = Generator::new(13);
        let mut sum = 0.0;
        let count = 20_000;
        for _ in 0..count {
            sum += generator.normal();
        }
        let mean = sum / f64::from(count);
        assert!(mean.abs() < 0.05, "normal mean {mean}");
    }

    #[test]
    fn a_pareto_draw_is_never_negative() {
        let mut generator = Generator::new(17);
        for _ in 0..1_000 {
            assert!(generator.pareto(50.0) >= 0.0);
        }
    }
}
