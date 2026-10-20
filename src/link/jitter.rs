//! The jitter distributions.
//!
//! A link's delay is not a constant: it is a propagation delay plus a
//! random component, and the random component's shape decides what the
//! transport experiences. A tail that is light produces occasional spikes; a
//! tail that is heavy produces frequent small delays and rare large ones.
//!
//! The harness offers three shapes, and a profile names one:
//!
//! * **none** — a constant delay. The baseline every other shape is
//!   measured against.
//! * **uniform** — a flat distribution over `[0, amplitude]`. The shape of
//!   a link whose delay is dominated by a queue of variable depth.
//! * **normal** — a bell around zero, truncated at zero. The shape of a
//!   link whose delay is dominated by independent noise.
//! * **Pareto** — a heavy tail with shape 1.5. The shape of a radio link,
//!   and the tier T4 profile's own.
//!
//! Every sample is drawn in microseconds and rounded to a whole one, so the
//! model never carries a fractional timestamp.

use serde::{Deserialize, Serialize};

/// The random component of a link's delay.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Jitter {
    /// A constant delay.
    None,
    /// A flat distribution over `[0, amplitude_us]`.
    Uniform {
        /// The distribution's width, in microseconds.
        amplitude_us: u64,
    },
    /// A bell around zero, truncated at zero.
    Normal {
        /// The distribution's standard deviation, in microseconds.
        amplitude_us: u64,
    },
    /// A heavy tail with shape 1.5.
    Pareto {
        /// The distribution's scale, in microseconds.
        amplitude_us: u64,
    },
}

impl Default for Jitter {
    fn default() -> Self {
        Self::None
    }
}

impl Jitter {
    /// No jitter at all.
    pub const fn none() -> Self {
        Self::None
    }

    /// A uniform jitter of the given width, in microseconds.
    pub const fn uniform(amplitude_us: u64) -> Self {
        Self::Uniform { amplitude_us }
    }

    /// A normal jitter of the given deviation, in microseconds.
    pub const fn normal(amplitude_us: u64) -> Self {
        Self::Normal { amplitude_us }
    }

    /// A Pareto jitter of the given scale, in microseconds.
    pub const fn pareto(amplitude_us: u64) -> Self {
        Self::Pareto { amplitude_us }
    }

    /// The distribution's amplitude, in microseconds.
    pub const fn amplitude_us(&self) -> u64 {
        match self {
            Self::None => 0,
            Self::Uniform { amplitude_us }
            | Self::Normal { amplitude_us }
            | Self::Pareto { amplitude_us } => *amplitude_us,
        }
    }

    /// Whether the distribution draws at all.
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The stable label spelling.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Uniform { .. } => "uniform",
            Self::Normal { .. } => "normal",
            Self::Pareto { .. } => "pareto",
        }
    }

    /// Draw one sample, in microseconds.
    pub fn sample_us(&self, generator: &mut super::super::rng::Generator) -> u64 {
        match self {
            Self::None => 0,
            Self::Uniform { amplitude_us } => generator.below(amplitude_us + 1),
            Self::Normal { amplitude_us } => {
                let sample = generator.normal().abs() * (*amplitude_us as f64);
                sample.round() as u64
            }
            Self::Pareto { amplitude_us } => {
                let sample = generator.pareto(*amplitude_us as f64);
                sample.round() as u64
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Generator;

    #[test]
    fn no_jitter_draws_nothing() {
        let mut generator = Generator::new(1);
        assert_eq!(Jitter::none().sample_us(&mut generator), 0);
        assert_eq!(generator.draws(), 0, "a distribution that draws nothing spends no draw");
    }

    #[test]
    fn a_uniform_sample_stays_within_its_width() {
        let mut generator = Generator::new(2);
        let jitter = Jitter::uniform(1_000);
        for _ in 0..1_000 {
            assert!(jitter.sample_us(&mut generator) <= 1_000);
        }
    }

    #[test]
    fn a_normal_sample_is_never_negative() {
        let mut generator = Generator::new(3);
        let jitter = Jitter::normal(1_000);
        for _ in 0..1_000 {
            assert!(jitter.sample_us(&mut generator) <= 4_000);
        }
    }

    #[test]
    fn a_pareto_sample_has_a_heavy_tail() {
        let mut generator = Generator::new(4);
        let jitter = Jitter::pareto(100_000);
        let mut samples: Vec<u64> = (0..2_000).map(|_| jitter.sample_us(&mut generator)).collect();
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        let p99 = samples[samples.len() * 99 / 100];
        assert!(p99 > median * 2, "median {median}, p99 {p99}");
    }

    #[test]
    fn the_same_seed_draws_the_same_samples() {
        let jitter = Jitter::normal(5_000);
        let first: Vec<u64> = {
            let mut generator = Generator::new(9);
            (0..50).map(|_| jitter.sample_us(&mut generator)).collect()
        };
        let second: Vec<u64> = {
            let mut generator = Generator::new(9);
            (0..50).map(|_| jitter.sample_us(&mut generator)).collect()
        };
        assert_eq!(first, second);
    }
}
