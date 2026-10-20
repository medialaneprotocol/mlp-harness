//! The sans-IO driver: a machine, driven deterministically.
//!
//! The harness's own driver runs the protocol's machines over the link
//! model. A **consumer's** machines — a transport, a relay, a gateway —
//! are driven the same way: bytes in, actions out, time as a parameter,
//! and every emitted action recorded.
//!
//! The machine is anything that implements [`mlp_core::StateMachine`], and
//! the driver never reads a clock of its own: the caller says when time
//! passes, and the driver says what the machine did.
//!
//! ```
//! use std::collections::VecDeque;
//! use std::time::Duration;
//!
//! use mlp_harness::sans_io::TestHarness;
//! use mlp_core::action::{Action, Receive, Transmit};
//! use mlp_core::state_machine::StateMachine;
//!
//! /// A machine that answers every byte with the same byte.
//! struct Echo {
//!     outbound: VecDeque<Vec<u8>>,
//! }
//!
//! impl StateMachine for Echo {
//!     type Action = Action<()>;
//!     type Transmit = Transmit;
//!     type Receive = Receive<()>;
//!
//!     fn handle_input(&mut self, input: Self::Receive, _now: std::time::Instant) -> Vec<Self::Action> {
//!         if let Receive::Bytes(bytes) = input {
//!             self.outbound.push_back(bytes);
//!         }
//!         Vec::new()
//!     }
//!
//!     fn poll_transmit(&mut self, _now: std::time::Instant) -> Option<Self::Transmit> {
//!         self.outbound.pop_front().map(|payload| Transmit { payload, flush: true })
//!     }
//!
//!     fn poll_timeout(&mut self, _now: std::time::Instant) -> Option<Duration> {
//!         None
//!     }
//!
//!     fn handle_timeout(&mut self, _now: std::time::Instant) -> Vec<Self::Action> {
//!         Vec::new()
//!     }
//! }
//!
//! let epoch = mlp_harness::epoch();
//! let mut driver = TestHarness::new(Echo { outbound: VecDeque::new() }, epoch);
//! driver.inject(Receive::Bytes(b"ping".to_vec()));
//! driver.run_until_quiescent(10);
//!
//! assert_eq!(driver.transmits().len(), 1);
//! assert_eq!(driver.transmits()[0].payload, b"ping");
//! ```

use std::collections::VecDeque;
use std::fmt::Debug;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use regex::Regex;

use mlp_core::state_machine::StateMachine;

/// In-memory driver for a sans-IO state machine.
///
/// The driver tracks the machine, the current time, the transmissions the
/// machine produced, and every action it emitted. It is the same driver
/// for every machine, whether the machine is the protocol's or a
/// consumer's.
#[derive(Debug)]
pub struct TestHarness<SM: StateMachine> {
    /// The state machine under test.
    pub machine: SM,
    /// The current time.
    pub now: Instant,
    /// The transmissions `poll_transmit` produced.
    pub transmit_queue: VecDeque<SM::Transmit>,
    /// Every action the machine emitted, in order.
    pub action_trace: Vec<SM::Action>,
    /// The absolute deadline from the last `poll_timeout` call.
    next_timeout: Option<Instant>,
}

impl<SM: StateMachine> TestHarness<SM> {
    /// A driver for a machine, starting at an origin.
    pub fn new(machine: SM, epoch: Instant) -> Self {
        Self {
            machine,
            now: epoch,
            transmit_queue: VecDeque::new(),
            action_trace: Vec::new(),
            next_timeout: None,
        }
    }

    /// Move the clock forward.
    pub fn advance_time(&mut self, duration: Duration) {
        self.now += duration;
    }

    /// Hand the machine an input, and record what it emitted.
    pub fn inject(&mut self, input: SM::Receive) {
        let actions = self.machine.handle_input(input, self.now);
        self.action_trace.extend(actions);
    }

    /// Drive the machine until it has no more immediate work.
    ///
    /// The driver drains the transmissions and fires the expired timeouts,
    /// up to `max_steps` iterations. The bound is what keeps a machine that
    /// never settles from looping forever.
    pub fn run_until_quiescent(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            // The deadline is recomputed whenever the machine has none.
            if self.next_timeout.is_none()
                && let Some(relative) = self.machine.poll_timeout(self.now)
            {
                self.next_timeout = Some(self.now + relative);
            }

            // A timeout that has expired fires.
            if let Some(deadline) = self.next_timeout
                && self.now >= deadline
            {
                let actions = self.machine.handle_timeout(self.now);
                self.action_trace.extend(actions);
                self.next_timeout = None;
                continue;
            }

            // Otherwise a transmission leaves, if there is one.
            if let Some(transmission) = self.machine.poll_transmit(self.now) {
                self.transmit_queue.push_back(transmission);
                continue;
            }

            // Nothing is left to do.
            break;
        }
    }

    /// Every action the machine emitted.
    pub fn actions(&self) -> &[SM::Action] {
        &self.action_trace
    }

    /// Every transmission the machine produced.
    pub fn transmits(&self) -> &VecDeque<SM::Transmit> {
        &self.transmit_queue
    }

    /// A snapshot of the actions and the transmissions.
    pub fn golden_trace(&self) -> GoldenTrace<SM>
    where
        SM::Action: Clone + PartialEq + Debug,
        SM::Transmit: Clone + PartialEq + Debug,
    {
        GoldenTrace {
            actions: self.action_trace.clone(),
            transmits: self.transmit_queue.clone(),
        }
    }

    /// Hand the machine a sequence of inputs, in order.
    pub fn inject_sequence<I>(&mut self, inputs: I)
    where
        I: IntoIterator<Item = SM::Receive>,
    {
        for input in inputs {
            self.inject(input);
        }
    }
}

/// A snapshot of every action and every transmission a machine produced.
///
/// Two snapshots are equal when their actions and their transmissions are
/// equal in the same order, which makes a test's repeatability a single
/// assertion.
pub struct GoldenTrace<SM: StateMachine>
where
    SM::Action: Clone + PartialEq + Debug,
    SM::Transmit: Clone + PartialEq + Debug,
{
    /// Every action the machine emitted.
    pub actions: Vec<SM::Action>,
    /// Every transmission the machine produced.
    pub transmits: VecDeque<SM::Transmit>,
}

impl<SM: StateMachine> GoldenTrace<SM>
where
    SM::Action: Clone + PartialEq + Debug,
    SM::Transmit: Clone + PartialEq + Debug,
{
    /// A snapshot from an action trace and a transmission queue.
    pub fn new(actions: Vec<SM::Action>, transmits: VecDeque<SM::Transmit>) -> Self {
        Self { actions, transmits }
    }

    /// Write the snapshot to a file, in the debug form.
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let content = format!("{:#?}", self);
        std::fs::write(path, content)
    }

    /// Write only the actions to a file.
    ///
    /// The transmissions are left out because they carry the bytes a build
    /// produced, and the actions are the machine's observable decisions.
    pub fn save_actions(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let content = format!("{:#?}", self.actions);
        std::fs::write(path, content)
    }

    /// Whether the snapshot equals the one in a file.
    pub fn matches_file(&self, path: impl AsRef<Path>) -> std::io::Result<bool> {
        let expected = std::fs::read_to_string(path)?;
        let actual = format!("{:#?}", self);
        Ok(expected == actual)
    }

    /// Assert that the snapshot equals the one in a file.
    ///
    /// Only the actions are compared, and the durations in them are
    /// normalised, so that a golden file is stable across runs. A missing
    /// file is an error: a golden file is a committed artefact, and one
    /// that is recreated silently hides an accidental deletion. Setting
    /// `UPDATE_GOLDEN_TRACES=1` writes the file instead of comparing it.
    pub fn assert_matches_file(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        if std::env::var_os("UPDATE_GOLDEN_TRACES").is_some() {
            self.save_actions(path).unwrap_or_else(|error| {
                panic!("failed to write golden file {}: {error}", path.display())
            });
            return;
        }
        let expected = std::fs::read_to_string(path).unwrap_or_else(|error| {
            panic!(
                "golden trace {} is missing ({error}); commit it or set UPDATE_GOLDEN_TRACES=1",
                path.display()
            )
        });
        let actual = normalize_golden_trace(&format!("{:#?}", self.actions));
        let expected_normalized = normalize_golden_trace(&expected);
        if expected_normalized != actual {
            panic!(
                "golden trace mismatch for {}\n\n--- expected ---\n{}\n\n--- actual ---\n{}",
                path.display(),
                expected_normalized,
                actual
            );
        }
    }
}

/// Normalise a golden trace by replacing the durations with a placeholder.
///
/// The debug form renders a duration as `110ms`, `1.5s`, `0ns` or `42µs`,
/// and a duration is the one value that changes from run to run.
fn normalize_golden_trace(text: &str) -> String {
    static DURATION: OnceLock<Regex> = OnceLock::new();
    let pattern = DURATION.get_or_init(|| {
        Regex::new(r"\b\d+(?:\.\d+)?(?:ns|µs|us|ms|s)\b").expect("the duration pattern")
    });
    pattern.replace_all(text, "<DELAY>").to_string()
}

impl<SM: StateMachine> Clone for GoldenTrace<SM>
where
    SM::Action: Clone + PartialEq + Debug,
    SM::Transmit: Clone + PartialEq + Debug,
{
    fn clone(&self) -> Self {
        Self {
            actions: self.actions.clone(),
            transmits: self.transmits.clone(),
        }
    }
}

impl<SM: StateMachine> Debug for GoldenTrace<SM>
where
    SM::Action: Clone + PartialEq + Debug,
    SM::Transmit: Clone + PartialEq + Debug,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GoldenTrace")
            .field("actions", &self.actions)
            .field("transmits", &self.transmits)
            .finish()
    }
}

impl<SM: StateMachine> PartialEq for GoldenTrace<SM>
where
    SM::Action: Clone + PartialEq + Debug,
    SM::Transmit: Clone + PartialEq + Debug,
{
    fn eq(&self, other: &Self) -> bool {
        self.actions == other.actions && self.transmits == other.transmits
    }
}

impl<SM: StateMachine> Eq for GoldenTrace<SM>
where
    SM::Action: Clone + PartialEq + Debug,
    SM::Transmit: Clone + PartialEq + Debug,
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlp_core::action::{Action, ConnectionEvent, Receive, Transmit};

    /// A tiny echo machine, used to exercise the driver.
    struct EchoMachine {
        outbound: VecDeque<Vec<u8>>,
        last_activity: Instant,
        heartbeat_interval: Duration,
    }

    impl EchoMachine {
        fn new(epoch: Instant, heartbeat_interval: Duration) -> Self {
            Self {
                outbound: VecDeque::new(),
                last_activity: epoch,
                heartbeat_interval,
            }
        }
    }

    impl StateMachine for EchoMachine {
        type Action = Action<()>;
        type Transmit = Transmit;
        type Receive = Receive<()>;

        fn handle_input(&mut self, input: Self::Receive, now: Instant) -> Vec<Self::Action> {
            match input {
                Receive::Bytes(bytes) => {
                    self.last_activity = now;
                    self.outbound.push_back(bytes);
                    Vec::new()
                }
                Receive::ConnectionEvent(ConnectionEvent::Closed) => {
                    vec![Action::Log("connection closed".to_string())]
                }
                _ => Vec::new(),
            }
        }

        fn poll_transmit(&mut self, _now: Instant) -> Option<Self::Transmit> {
            self.outbound.pop_front().map(|payload| Transmit {
                payload,
                flush: true,
            })
        }

        fn poll_timeout(&mut self, now: Instant) -> Option<Duration> {
            let elapsed = now.duration_since(self.last_activity);
            if elapsed >= self.heartbeat_interval {
                Some(Duration::ZERO)
            } else {
                Some(self.heartbeat_interval - elapsed)
            }
        }

        fn handle_timeout(&mut self, now: Instant) -> Vec<Self::Action> {
            self.last_activity = now;
            vec![Action::Log("heartbeat".to_string())]
        }
    }

    #[test]
    fn the_machine_transmits_the_bytes_it_is_given() {
        let epoch = crate::epoch();
        let machine = EchoMachine::new(epoch, Duration::from_secs(1));
        let mut driver = TestHarness::new(machine, epoch);

        driver.inject(Receive::Bytes(b"ping".to_vec()));
        driver.run_until_quiescent(10);

        assert_eq!(driver.transmits().len(), 1);
        assert_eq!(driver.transmits()[0].payload, b"ping");
        assert!(driver.actions().is_empty());
    }

    #[test]
    fn the_machine_answers_a_timeout_with_an_action() {
        let epoch = crate::epoch();
        let machine = EchoMachine::new(epoch, Duration::from_millis(100));
        let mut driver = TestHarness::new(machine, epoch);

        driver.advance_time(Duration::from_millis(150));
        driver.run_until_quiescent(10);

        assert_eq!(driver.actions().len(), 1);
        assert_eq!(driver.actions()[0], Action::Log("heartbeat".to_string()));
    }

    #[test]
    fn the_transmissions_keep_their_order() {
        let epoch = crate::epoch();
        let machine = EchoMachine::new(epoch, Duration::from_secs(1));
        let mut driver = TestHarness::new(machine, epoch);

        driver.inject(Receive::Bytes(b"first".to_vec()));
        driver.inject(Receive::Bytes(b"second".to_vec()));
        driver.run_until_quiescent(10);

        let payloads: Vec<_> = driver
            .transmits()
            .iter()
            .map(|transmission| transmission.payload.clone())
            .collect();
        assert_eq!(payloads, vec![b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn the_same_inputs_give_the_same_actions() {
        let epoch = crate::epoch();

        let run = || {
            let machine = EchoMachine::new(epoch, Duration::from_millis(50));
            let mut driver = TestHarness::new(machine, epoch);
            driver.inject(Receive::Bytes(b"a".to_vec()));
            driver.advance_time(Duration::from_millis(75));
            driver.run_until_quiescent(10);
            driver.action_trace.clone()
        };

        assert_eq!(run(), run());
    }

    #[test]
    fn a_closed_connection_is_reported() {
        let epoch = crate::epoch();
        let machine = EchoMachine::new(epoch, Duration::from_secs(1));
        let mut driver = TestHarness::new(machine, epoch);

        driver.inject(Receive::ConnectionEvent(ConnectionEvent::Closed));
        driver.run_until_quiescent(10);

        assert_eq!(
            driver.actions()[0],
            Action::Log("connection closed".to_string())
        );
    }

    #[test]
    fn a_snapshot_is_reproducible() {
        let epoch = crate::epoch();

        let run = || {
            let machine = EchoMachine::new(epoch, Duration::from_millis(50));
            let mut driver = TestHarness::new(machine, epoch);
            driver.inject_sequence([
                Receive::Bytes(b"first".to_vec()),
                Receive::Bytes(b"second".to_vec()),
            ]);
            driver.advance_time(Duration::from_millis(75));
            driver.run_until_quiescent(10);
            driver.golden_trace()
        };

        assert_eq!(run(), run());
    }

    #[test]
    fn a_snapshot_diverges_on_changed_inputs() {
        let epoch = crate::epoch();

        let run_with = |payload: &[u8]| {
            let machine = EchoMachine::new(epoch, Duration::from_millis(50));
            let mut driver = TestHarness::new(machine, epoch);
            driver.inject(Receive::Bytes(payload.to_vec()));
            driver.advance_time(Duration::from_millis(75));
            driver.run_until_quiescent(10);
            driver.golden_trace()
        };

        assert_ne!(run_with(b"ping"), run_with(b"pong"));
    }

    #[test]
    fn the_durations_are_normalised() {
        let normalized = normalize_golden_trace("waited 110ms and 1.5s and 42µs and 0ns");
        assert_eq!(
            normalized,
            "waited <DELAY> and <DELAY> and <DELAY> and <DELAY>"
        );
    }
}
