//! The loss models.
//!
//! Loss is not one thing. A radio link loses objects at random; a congested
//! queue loses objects in runs; a fading link alternates between a good and
//! a bad state and loses everything in the bad state. A model of loss that
//! is only "each object, independently, with probability p" describes none
//! of them well, and a transport's behaviour under loss depends on exactly
//! which one it is: a burst of losses is a *correlated* event, and the
//! transport's repair machinery answers a correlation very differently from
//! an independent event.
//!
//! The harness therefore offers four models, and a profile names one:
//!
//! | Model | What it models |
//! |---|---|
//! | [`LossModel::None`] | a link that loses nothing |
//! | [`LossModel::Uniform`] | independent loss: a radio fading at random |
//! | [`LossModel::Bursty`] | loss in runs of a geometric length |
//! | [`LossModel::GilbertElliott`] | a two-state chain: a good state and a bad state |
//! | [`LossModel::Correlated`] | loss that grows with the queue's occupancy |
//!
//! # When loss is decided
//!
//! Loss is decided **when the object is offered to the link**, because that
//! is when the link's occupancy is known, and the correlated model is a
//! function of the occupancy. A separate, deterministic drop — the queue's
//! overflow — is decided when the object is offered too, and is attributed
//! to the link's queue rather than to its radio.

use serde::{Deserialize, Serialize};

use super::super::rng::Generator;

/// How a direction loses objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LossModel {
    /// Nothing is lost.
    #[default]
    None,
    /// Every object is lost independently with the given probability.
    Uniform {
        /// The loss probability, in permille.
        permille: u32,
    },
    /// A loss starts with the given probability and runs for a geometric
    /// number of objects, with an expected run of
    /// [`Bursty::MEAN_RUN`] objects.
    Bursty {
        /// The probability that a loss run starts, in permille.
        permille: u32,
    },
    /// A two-state Markov chain: the link is in a good state or a bad one,
    /// and objects offered in the bad state are lost.
    GilbertElliott {
        /// The probability of leaving the good state, in permille.
        good_to_bad_permille: u32,
        /// The probability of leaving the bad state, in permille.
        bad_to_good_permille: u32,
    },
    /// The loss probability grows with the queue's occupancy: a congested
    /// queue drops a run of consecutive objects.
    Correlated {
        /// The loss probability at an empty queue, in permille.
        base_permille: u32,
        /// The extra loss probability at a full queue, in permille.
        queue_permille: u32,
    },
}

impl LossModel {
    /// The expected length of a burst's run, in objects.
    pub const MEAN_RUN: u32 = 4;

    /// The stable label spelling.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Uniform { .. } => "uniform",
            Self::Bursty { .. } => "bursty",
            Self::GilbertElliott { .. } => "gilbert-elliott",
            Self::Correlated { .. } => "correlated",
        }
    }

    /// Whether the model can lose anything at all.
    pub const fn loses(&self) -> bool {
        match self {
            Self::None => false,
            Self::Uniform { permille } => *permille > 0,
            Self::Bursty { permille } => *permille > 0,
            Self::GilbertElliott { .. } => true,
            Self::Correlated { .. } => true,
        }
    }

    /// The model's loss probability at a **full** queue, in permille.
    ///
    /// The correlated model is a function of the occupancy, so this is its
    /// ceiling; a bursty model loses more than its entry probability, once
    /// per object of the run.
    pub fn worst_case_permille(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Uniform { permille } => *permille,
            Self::Bursty { permille } => permille.saturating_mul(Self::MEAN_RUN),
            Self::GilbertElliott {
                good_to_bad_permille,
                bad_to_good_permille,
            } => {
                // The expected share of the objects the chain spends in the
                // bad state.
                let leaving = *good_to_bad_permille + *bad_to_good_permille;
                (*good_to_bad_permille * 1_000)
                    .checked_div(leaving)
                    .unwrap_or(0)
            }
            Self::Correlated {
                base_permille,
                queue_permille,
            } => base_permille.saturating_add(*queue_permille),
        }
    }

    /// The model's own loss probability at an empty queue, in permille.
    ///
    /// The correlated model is a function of the occupancy, so this is its
    /// floor.
    pub const fn base_permille(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Uniform { permille } | Self::Bursty { permille } => *permille,
            Self::GilbertElliott { .. } => 0,
            Self::Correlated { base_permille, .. } => *base_permille,
        }
    }
}

/// The state a two-state loss chain carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainState {
    Good,
    Bad,
}

/// A direction's loss state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LossState {
    chain: Option<ChainState>,
    in_burst: bool,
}

impl LossState {
    /// A fresh loss state for a model.
    pub fn new() -> Self {
        Self {
            chain: None,
            in_burst: false,
        }
    }

    /// Whether the next object is lost, drawing from the link's generator.
    ///
    /// `occupancy_permille` is the queue's occupancy in permille, which the
    /// correlated model needs and the others ignore.
    pub fn roll_with(
        &mut self,
        model: &LossModel,
        occupancy_permille: u32,
        generator: &mut Generator,
    ) -> bool {
        match model {
            LossModel::None => false,
            LossModel::Uniform { permille } => generator.chance_permille(*permille),
            LossModel::Bursty { permille } => {
                if self.in_burst {
                    // The run continues with probability one minus one over
                    // `MEAN_RUN`, so a run's expected length is `MEAN_RUN`.
                    let continue_permille = 1_000 - (1_000 / LossModel::MEAN_RUN);
                    self.in_burst = generator.chance_permille(continue_permille);
                    return true;
                }
                if generator.chance_permille(*permille) {
                    self.in_burst = true;
                    true
                } else {
                    false
                }
            }
            LossModel::GilbertElliott {
                good_to_bad_permille,
                bad_to_good_permille,
            } => {
                let state = self.chain.unwrap_or(ChainState::Good);
                let next = match state {
                    ChainState::Good => {
                        if generator.chance_permille(*good_to_bad_permille) {
                            ChainState::Bad
                        } else {
                            ChainState::Good
                        }
                    }
                    ChainState::Bad => {
                        if generator.chance_permille(*bad_to_good_permille) {
                            ChainState::Good
                        } else {
                            ChainState::Bad
                        }
                    }
                };
                self.chain = Some(next);
                next == ChainState::Bad
            }
            LossModel::Correlated {
                base_permille,
                queue_permille,
            } => {
                let effective = base_permille
                    .saturating_add(queue_permille.saturating_mul(occupancy_permille) / 1_000)
                    .min(1_000);
                generator.chance_permille(effective)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_model_loses_anything() {
        let mut state = LossState::new();
        let mut generator = Generator::new(1);
        for _ in 0..100 {
            assert!(!state.roll_with(&LossModel::None, 1_000, &mut generator));
        }
    }

    #[test]
    fn a_uniform_model_loses_close_to_its_probability() {
        let mut state = LossState::new();
        let mut generator = Generator::new(2);
        let mut lost = 0;
        for _ in 0..10_000 {
            if state.roll_with(&LossModel::Uniform { permille: 200 }, 0, &mut generator) {
                lost += 1;
            }
        }
        assert!((1_800..=2_200).contains(&lost), "200‰ lost {lost}/10000");
    }

    #[test]
    fn a_bursty_model_loses_in_runs() {
        let mut state = LossState::new();
        let mut generator = Generator::new(3);
        let mut runs = 0;
        let mut previous_lost = false;
        let mut lost = 0;
        for _ in 0..20_000 {
            let now_lost = state.roll_with(&LossModel::Bursty { permille: 100 }, 0, &mut generator);
            if now_lost {
                lost += 1;
                if !previous_lost {
                    runs += 1;
                }
            }
            previous_lost = now_lost;
        }
        assert!(
            lost > 1_500,
            "a bursty model loses more than the uniform one: {lost}"
        );
        assert!(
            runs < lost / 2,
            "the losses come in runs: {runs} runs of {lost} losses"
        );
    }

    #[test]
    fn a_gilbert_elliott_chain_alternates() {
        let mut state = LossState::new();
        let mut generator = Generator::new(4);
        let model = LossModel::GilbertElliott {
            good_to_bad_permille: 100,
            bad_to_good_permille: 500,
        };
        let mut transitions = 0;
        let mut previous = false;
        for _ in 0..10_000 {
            let now = state.roll_with(&model, 0, &mut generator);
            if now != previous {
                transitions += 1;
            }
            previous = now;
        }
        assert!(
            transitions > 100,
            "the chain moves: {transitions} transitions"
        );
    }

    #[test]
    fn a_correlated_model_loses_more_when_the_queue_is_full() {
        let mut empty = LossState::new();
        let mut full = LossState::new();
        let mut generator = Generator::new(5);
        let model = LossModel::Correlated {
            base_permille: 10,
            queue_permille: 500,
        };
        let mut empty_losses = 0;
        let mut full_losses = 0;
        for _ in 0..10_000 {
            if empty.roll_with(&model, 0, &mut generator) {
                empty_losses += 1;
            }
            if full.roll_with(&model, 1_000, &mut generator) {
                full_losses += 1;
            }
        }
        assert!(
            full_losses > empty_losses * 3,
            "empty {empty_losses}, full {full_losses}"
        );
    }
}
