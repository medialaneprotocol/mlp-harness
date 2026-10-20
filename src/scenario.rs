//! The scenario: a program against a virtual clock.
//!
//! A scenario is a **sequence of steps**, not a script. Each step is one of
//! a small set of statements, and the harness executes them in order against
//! a clock it owns:
//!
//! * `Advance` — move the clock;
//! * `SetLink` — replace the link's profile;
//! * `Handover` — change the carrier under the link's feet;
//! * `CaptureChange` — the capture produced a change in a region;
//! * `ViewerInput` — the user did something;
//! * `ViewerDisconnect` / `ViewerReconnect` — the receiver's lane died, and
//!   came back;
//! * `HostReconnect` — the sender's transport to the intermediary was
//!   re-established;
//! * `PathSwitch` — the receiver's carrier changed;
//! * `Assert` — check an invariant at this point.
//!
//! A scenario is a value: it serializes, it diffs, and two scenarios that
//! are equal produce byte-identical traces.

use serde::{Deserialize, Serialize};

use crate::faults::Faults;
use crate::invariants::Invariant;
use crate::link::{Handover, LinkProfile, OutageWindow, Tier};

/// The surface a session carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridSpec {
    /// The surface's width, in pixels.
    pub width_px: u32,
    /// The surface's height, in pixels.
    pub height_px: u32,
    /// A cell's edge, in pixels.
    pub cell_size_px: u32,
}

impl Default for GridSpec {
    fn default() -> Self {
        Self {
            width_px: 1_920,
            height_px: 1_080,
            cell_size_px: 64,
        }
    }
}

impl GridSpec {
    /// The surface's cells.
    pub fn cells(&self) -> u32 {
        let cols = self.width_px.div_ceil(self.cell_size_px).max(1);
        let rows = self.height_px.div_ceil(self.cell_size_px).max(1);
        cols * rows
    }
}

/// A region of the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    /// The region's left edge, in pixels.
    pub x_px: u32,
    /// The region's top edge, in pixels.
    pub y_px: u32,
    /// The region's width, in pixels.
    pub width_px: u32,
    /// The region's height, in pixels.
    pub height_px: u32,
}

impl Region {
    /// A region.
    pub fn new(x_px: u32, y_px: u32, width_px: u32, height_px: u32) -> Self {
        Self {
            x_px,
            y_px,
            width_px,
            height_px,
        }
    }
}

/// Something the user did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    /// A key went down or came up.
    Key {
        /// The key's code, as the browser reports it.
        code: String,
        /// Whether the key went down.
        pressed: bool,
    },
    /// The pointer moved.
    PointerMove {
        /// The pointer's x, in pixels.
        x_px: u32,
        /// The pointer's y, in pixels.
        y_px: u32,
    },
    /// A pointer button went down or came up.
    PointerButton {
        /// The button's code.
        button: u32,
        /// Whether it went down.
        pressed: bool,
    },
    /// The wheel turned.
    Wheel {
        /// The notches, positive away from the user.
        delta: i32,
    },
    /// The clipboard changed.
    Clipboard {
        /// The text.
        text: String,
    },
    /// Everything was released at once.
    ReleaseAll,
    /// The receiver states the keys and buttons it holds.
    ///
    /// The pressed-set protocol: the sender releases every key the set does
    /// not name (MLP spec §8.3.4).
    PressedSet,
}

impl InputEvent {
    /// The key code a browser reports, as the number the protocol carries.
    ///
    /// The protocol carries a key's code as a number, and the browser
    /// implementation maps the browser's string onto it. The harness uses
    /// the same mapping the reference implementation's suites use: the
    /// characters' code points, offset from the first letter.
    pub fn key_code(code: &str) -> u32 {
        match code {
            "Enter" => 13,
            "Escape" => 27,
            "Space" => 32,
            "Backspace" => 8,
            "Tab" => 9,
            "ShiftLeft" => 16,
            "ShiftRight" => 16,
            "ControlLeft" => 17,
            "ControlRight" => 17,
            "AltLeft" => 18,
            "AltRight" => 18,
            other => {
                let mut chars = other.chars();
                let first = chars.next().unwrap_or('?');
                if other.len() == 1 {
                    // A single character: its code point.
                    return u32::from(first as u8);
                }
                // A named key: a stable number derived from its name, so the
                // same name always means the same key.
                let mut hash: u32 = 2_166_136_261;
                for byte in other.as_bytes() {
                    hash ^= u32::from(*byte);
                    hash = hash.wrapping_mul(16_777_619);
                }
                // The code travels in four bytes.
                hash & 0x00ff_ffff
            }
        }
    }
}

/// One step of a scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Step {
    /// Move the clock forward.
    Advance {
        /// How far, in milliseconds.
        ms: u64,
    },
    /// Replace the link's profile.
    SetLink {
        /// The new profile.
        profile: Box<LinkProfile>,
    },
    /// Change the carrier.
    Handover {
        /// The handover.
        handover: Box<Handover>,
    },
    /// The capture produced a change in a region.
    CaptureChange {
        /// The region.
        region: Region,
    },
    /// The user did something.
    ViewerInput {
        /// What they did.
        event: Box<InputEvent>,
    },
    /// The receiver's lane died.
    ViewerDisconnect,
    /// The receiver's lane came back.
    ViewerReconnect,
    /// The sender's transport was re-established.
    HostReconnect,
    /// The receiver's carrier changed.
    PathSwitch {
        /// The new carrier's label.
        path: String,
    },
    /// An outage is scheduled from now.
    ScheduleOutage {
        /// The window.
        window: OutageWindow,
    },
    /// Check an invariant at this point.
    Assert {
        /// The invariant.
        invariant: Invariant,
    },
}

/// A scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// The schema version of the scenario document.
    pub schema: u32,
    /// The scenario's name.
    pub name: String,
    /// The tier the scenario targets.
    pub tier: Tier,
    /// The seed of the scenario's own generator.
    pub seed: u64,
    /// The link the scenario starts with.
    pub link: LinkProfile,
    /// The pace the sender remembers for this link.
    ///
    /// The protocol's estimator can start from a remembered rate (MLP spec
    /// §8), which is what a session on a link the sender has used before
    /// looks like. `None` means a cold start: the sender measures the link
    /// from nothing.
    #[serde(default)]
    pub prior_bps: Option<u64>,
    /// The surface the session carries.
    pub grid: GridSpec,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// The faults the implementation under test carries.
    ///
    /// A fault is a **deliberate defect**: it makes the peer misbehave in
    /// one stated way, so that the invariant that forbids it is shown to
    /// bite rather than merely to exist.
    #[serde(default)]
    pub faults: Faults,
}

impl Scenario {
    /// A scenario for a tier, with the tier's canonical link profile.
    pub fn new(name: impl Into<String>, tier: Tier) -> Self {
        Self {
            schema: 1,
            name: name.into(),
            tier,
            seed: 0x5EED_1234_ABCD_0001,
            link: LinkProfile::tier(tier),
            prior_bps: None,
            grid: GridSpec::default(),
            steps: Vec::new(),
            faults: Faults::default(),
        }
    }

    /// Append a step.
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Append a wait.
    pub fn advance(self, ms: u64) -> Self {
        self.step(Step::Advance { ms })
    }

    /// Append a capture change.
    pub fn change(self, region: Region) -> Self {
        self.step(Step::CaptureChange { region })
    }

    /// Append an input event.
    pub fn input(self, event: InputEvent) -> Self {
        self.step(Step::ViewerInput {
            event: Box::new(event),
        })
    }

    /// Append an assertion.
    pub fn assert(self, invariant: Invariant) -> Self {
        self.step(Step::Assert { invariant })
    }

    /// The scenario's total scheduled time, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.steps
            .iter()
            .map(|step| match step {
                Step::Advance { ms } => *ms,
                _ => 0,
            })
            .sum()
    }

    /// The scenario as a JSON document.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a scenario is serializable")
    }

    /// Read a scenario from a JSON document.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| format!("the scenario does not parse: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scenario_round_trips_through_json() {
        let scenario = Scenario::new("t2-typing", Tier::T2)
            .advance(1_000)
            .change(Region::new(320, 240, 640, 200))
            .input(InputEvent::Key {
                code: "KeyH".to_string(),
                pressed: true,
            })
            .assert(Invariant::Inv8);
        let json = scenario.to_json();
        let parsed = Scenario::from_json(&json).expect("the scenario parses");
        assert_eq!(parsed, scenario);
    }

    #[test]
    fn a_scenario_reports_its_duration() {
        let scenario = Scenario::new("d", Tier::T1).advance(100).advance(250);
        assert_eq!(scenario.duration_ms(), 350);
    }

    #[test]
    fn the_grid_reports_its_cells() {
        let grid = GridSpec::default();
        assert_eq!(grid.cells(), 30 * 17);
    }

    #[test]
    fn a_key_code_is_stable() {
        assert_eq!(InputEvent::key_code("KeyH"), InputEvent::key_code("KeyH"));
        assert_ne!(InputEvent::key_code("KeyH"), InputEvent::key_code("KeyE"));
        assert_eq!(InputEvent::key_code("Space"), 32);
    }
}
