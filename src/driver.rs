//! The driver: both ends, as the real machines, over the link.
//!
//! The driver runs a scenario. It owns:
//!
//! * the **virtual clock**;
//! * the **link**;
//! * the protocol's **sender machine** and **receiver machine**, driven
//!   exactly as a deployment drives them: bytes in, bytes out, time as a
//!   parameter;
//! * the **shadow reference** — what the sender emitted, per cell, and what
//!   the receiver holds, per cell — which is what INV-2 compares;
//! * the **input path**, from the user's keystroke to the sender's pressed
//!   set;
//! * the **checker**, which reads an observation and decides whether each
//!   invariant held.
//!
//! # What the driver does *not* do
//!
//! It does not re-implement the protocol. Every fragment on the link is
//! built by the protocol's own sender machine and parsed by the protocol's
//! own receiver machine. Every acknowledgement, every feedback report, every
//! probe and every input envelope is the protocol's own type. The only
//! thing the driver owns is the *content* of a media update — the transport
//! does not interpret an update's content (MLP spec §5.1), so the harness's
//! media source is a cell bitmap, which is the smallest thing that says
//! which cells an update covers.
//!
//! # The faults
//!
//! The fault injector makes the *peer* misbehave. It never touches the
//! link, the clock, or the protocol's machines: a broken implementation is
//! broken in what it decides to do, and the checkers read the decisions.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mlp_core::media::MediaPath;
use mlp_core::media::cells::{CellGrid, CellRect};
use mlp_core::media::input::{
    HostInputConfig, HostInputState, InputKind, InputQueueConfig, MediaInputEnvelope, PushOutcome,
    ViewerInputQueue,
};
use mlp_core::media::pdu::{Carrier, Fragment, FragmentClass, FragmentFlags, HeaderExt, Priority};
use mlp_core::media::receiver::{MediaReceiveEvent, MediaReceiverConfig, MediaReceiverMachine};
use mlp_core::media::scheduler::UpdateKind;
use mlp_core::media::sender::{
    EnqueueOutcome, MediaSenderConfig, MediaSenderEvent, MediaSenderMachine, UpdateRequest,
};

use crate::checks::{
    AppliedUpdate, CellState, Checker, InputApplication, ObjectSize, Observation, QueueDepth,
    Release, UpdateParts,
};
use crate::clock::VirtualClock;
use crate::faults::Faults;
use crate::invariants::Invariant;
use crate::link::{Direction, Link, LinkEvent, Tier};
use crate::scenario::{InputEvent, Scenario, Step};
use crate::trace::{
    AckDecision, Decisions, EchoSample, InputDecision, Measurements, PlanDecision, ScenarioSummary,
    StateDecision, Trace, UpdateDecision,
};
use crate::wire;

/// The driver's tick, in microseconds.
///
/// One millisecond is fine enough for the protocol's timers and the pacer,
/// and coarse enough that a minute-long scenario is sixty thousand ticks.
pub const TICK_US: u64 = 1_000;

/// The first object id the receiver's own objects use.
///
/// The sender allocates its ids from one; the receiver's own objects live
/// above that space, so the two never collide and a dedupe window cannot
/// mistake one for the other.
const UP_OBJECT_ID_BASE: u32 = 0x4000_0000;

/// The driver.
#[derive(Debug)]
pub struct Driver {
    clock: VirtualClock,
    link: Link,
    sender: MediaSenderMachine,
    receiver: MediaReceiverMachine,
    host_rx: MediaReceiverMachine,
    grid: CellGrid,
    carrier: Carrier,
    tier: Tier,
    faults: Faults,
    prior_bps: Option<u64>,

    capture_dirty: Vec<u32>,
    last_change_us: u64,

    emitted: Vec<u64>,
    held: Vec<u64>,
    /// The cells the receiver acknowledged, and the update it acknowledged.
    acknowledged: BTreeMap<u32, u64>,
    /// The releases of the last second, for the one-second window.
    recent_releases: VecDeque<Release>,

    input_queue: ViewerInputQueue,
    host_input: HostInputState,
    /// The host's pressed set: the keys and buttons the *deployment* holds
    /// down, driven by the input objects the sender applied.
    host_pressed_keys: BTreeSet<u32>,
    host_pressed_buttons: BTreeSet<u32>,
    input_sent_us: BTreeMap<u64, u64>,
    input_applications: Vec<InputApplication>,
    /// The input sequence the next pulse carries (MLP spec §6.3.5).
    pulse_pending: Option<u64>,
    /// The highest input sequence the receiver's applied content reflects.
    receiver_echo: u64,
    retained_inputs: usize,
    /// The input objects the deployment has taken but not sent.
    unsent_inputs: usize,
    received_updates: BTreeSet<u64>,
    echoed_inputs: BTreeSet<u64>,
    sender_echoed_inputs: BTreeSet<u64>,
    up_object_id: u32,

    connected: bool,
    parked: bool,
    parked_since_ms: Option<u64>,
    lanes_created: u32,
    parks: u32,
    convergences: u64,
    convergence_us: Option<u64>,

    decisions: Decisions,
    releases: Vec<Release>,
    objects: Vec<ObjectSize>,
    update_parts: Vec<UpdateParts>,
    applied_updates: Vec<AppliedUpdate>,
    link_events: Vec<LinkEvent>,
    hop_dropped: u64,
    unattributed_claims: u64,
    burst_injected: bool,
    feedbacks_received: u64,
    acks_received: u64,
    inputs_received: u64,
    gaps: u64,
    /// Parts the wire parity recovered at the receiver (MLP spec §6.9):
    /// each one is a lost part that cost no round trip.
    parity_recovered: u64,
    stale_applies_skipped: u64,

    checker: Checker,
}

impl Driver {
    /// A driver for a scenario.
    pub fn new(scenario: &Scenario) -> Self {
        let grid = CellGrid::with_cell_size(
            scenario.grid.width_px,
            scenario.grid.height_px,
            scenario.grid.cell_size_px,
        );
        let carrier = carrier_for(scenario.link.mtu_bytes);
        let mut sender_config = MediaSenderConfig::new(carrier, grid);
        sender_config.accounting.datagram = carrier.is_datagram();
        sender_config.bwe.pace_floor_bps = 8_000;
        let prior_bps = scenario.prior_bps.or(Some(scenario.link.down.rate_bps));
        sender_config.bwe.prior_bps = prior_bps;
        let clock = VirtualClock::new();
        let sender = MediaSenderMachine::new(sender_config, grid, clock.instant());
        let receiver =
            MediaReceiverMachine::new(MediaReceiverConfig::for_carrier(carrier), clock.instant());
        // The sender's side parses the feedback that comes back, with its
        // own machine: the feedback objects carry no update id, so they are
        // never accounted as media the peer lost.
        let host_rx =
            MediaReceiverMachine::new(MediaReceiverConfig::for_carrier(carrier), clock.instant());
        let cells = grid.cells() as usize;
        Self {
            clock,
            link: Link::new(scenario.link.clone()),
            sender,
            receiver,
            host_rx,
            grid,
            carrier,
            tier: scenario.tier,
            faults: scenario.faults,
            prior_bps,
            capture_dirty: Vec::new(),
            last_change_us: 0,
            emitted: vec![0; cells],
            held: vec![0; cells],
            acknowledged: BTreeMap::new(),
            recent_releases: VecDeque::new(),
            input_queue: ViewerInputQueue::new(InputQueueConfig::default(), 1),
            host_input: HostInputState::new(HostInputConfig::default()),
            host_pressed_keys: BTreeSet::new(),
            host_pressed_buttons: BTreeSet::new(),
            input_sent_us: BTreeMap::new(),
            input_applications: Vec::new(),
            pulse_pending: None,
            receiver_echo: 0,
            retained_inputs: 0,
            unsent_inputs: 0,
            received_updates: BTreeSet::new(),
            echoed_inputs: BTreeSet::new(),
            sender_echoed_inputs: BTreeSet::new(),
            up_object_id: UP_OBJECT_ID_BASE,
            connected: true,
            parked: false,
            parked_since_ms: None,
            lanes_created: 1,
            parks: 0,
            convergences: 0,
            convergence_us: None,
            decisions: Decisions::default(),
            releases: Vec::new(),
            objects: Vec::new(),
            update_parts: Vec::new(),
            applied_updates: Vec::new(),
            link_events: Vec::new(),
            hop_dropped: 0,
            unattributed_claims: if scenario.faults.unattributed_drop {
                5
            } else {
                0
            },
            burst_injected: false,
            feedbacks_received: 0,
            acks_received: 0,
            inputs_received: 0,
            gaps: 0,
            parity_recovered: 0,
            stale_applies_skipped: 0,
            checker: Checker::new(),
        }
    }

    /// Run a scenario and return its trace.
    pub fn run(scenario: &Scenario) -> Trace {
        let mut driver = Driver::new(scenario);
        driver.execute(scenario);
        driver.finish(scenario)
    }

    /// Execute the scenario's steps.
    fn execute(&mut self, scenario: &Scenario) {
        for step in &scenario.steps {
            self.apply_step(step);
        }
    }

    /// Apply one step.
    fn apply_step(&mut self, step: &Step) {
        match step {
            Step::Advance { ms } => self.advance(*ms),
            Step::SetLink { profile } => self.link.set_profile((**profile).clone()),
            Step::Handover { handover } => self.link.handover(handover),
            Step::CaptureChange { region } => self.capture_change(*region),
            Step::ViewerInput { event } => self.viewer_input(event),
            Step::ViewerDisconnect => self.disconnect(),
            Step::ViewerReconnect => self.reconnect(),
            Step::HostReconnect => {}
            Step::PathSwitch { path } => {
                if let Some(path) = path_for(path) {
                    self.sender.set_path(path, self.clock.instant());
                }
            }
            Step::ScheduleOutage { window } => {
                let mut profile = self.link.profile().clone();
                profile.outages.windows.push(*window);
                self.link.set_profile(profile);
            }
            Step::Assert { invariant } => {
                let observation = self.observation();
                self.checker.check(*invariant, &observation);
            }
        }
    }

    /// Move the clock forward, in ticks.
    fn advance(&mut self, ms: u64) {
        let total_us = ms.saturating_mul(1_000);
        let mut moved = 0;
        while moved < total_us {
            let step = TICK_US.min(total_us - moved);
            self.tick(step);
            moved += step;
        }
    }

    /// One tick of the whole session.
    fn tick(&mut self, dt_us: u64) {
        self.clock.advance_us(dt_us);
        self.sender_side();
        let deliveries = self.link.advance_to(self.clock.elapsed_us());
        for delivery in deliveries {
            match delivery.direction {
                Direction::Down => self.receiver_receive(&delivery.bytes),
                Direction::Up => self.sender_receive(&delivery.bytes),
            }
        }
        self.receiver_side();
        self.check_park_grace();
        self.observe();
    }

    // ── the sender side ────────────────────────────────────────────────────

    /// The sender's obligations: its timers, its updates, its fragments.
    fn sender_side(&mut self) {
        let now = self.clock.instant();
        for event in self.sender.handle_timeout_events(now) {
            if let MediaSenderEvent::ProbeDue {
                probe_id,
                payload_bytes,
                ..
            } = event
            {
                let probe = mlp_core::media::MediaProbe {
                    probe_id,
                    payload_bytes,
                    send_epoch_ms: 0,
                };
                let payload = vec![0u8; payload_bytes as usize];
                self.sender.enqueue_probe(&probe, payload, now);
            }
        }
        let lane_events = self.sender.poll_lane(now);
        self.note_sender_events(&lane_events);
        self.emit_update(now);
        self.release_fragments(now);
        self.report_demand(now);
    }

    /// Tell the estimator that there is more work than the pace allows.
    ///
    /// This is what arms the demand-driven probe (MLP spec §8) and lets the
    /// pace climb on a link whose loss keeps the estimator's own ramp from
    /// growing it. A driver that never says what it wants is a driver the
    /// estimator cannot measure.
    fn report_demand(&mut self, now: std::time::Instant) {
        let pending = self.sender.accounting().pending_count()
            + self.capture_dirty.len() as u32
            + u32::try_from(self.sender.queued_objects()).unwrap_or(0);
        if pending == 0 {
            return;
        }
        let estimate = self.sender.estimate();
        let demand_bps = estimate.pace_bps.saturating_mul(u64::from(pending).min(8));
        self.sender.estimator_mut().on_demand(demand_bps, now);
    }

    /// Emit an update when one is due.
    fn emit_update(&mut self, now: std::time::Instant) {
        let session_ms = self.sender.session_ms(now);
        let pulse = self.sender.scheduler().timer().pulse_active(session_ms);
        let cadence = self
            .sender
            .scheduler()
            .timer()
            .due(session_ms, self.sender.target_fps(now));
        if !cadence && !pulse {
            return;
        }
        let pending = self.sender.accounting().pending_count();
        // An open pulse is work: the idle cadence must not swallow it, or
        // the echo would wait for the next idle update.
        if self.capture_dirty.is_empty() && pending == 0 && !pulse {
            if let Some(idle) = self.sender.poll_idle(now) {
                let update_id = self.sender.next_update_id();
                if let EnqueueOutcome::Accepted { .. } = self.sender.enqueue_update(idle, now) {
                    self.record_update(update_id, "idle", 0, 0, false, None);
                }
            }
            return;
        }
        let fresh = self.capture_dirty.clone();
        let refresh: Vec<u32> = self
            .emitted
            .iter()
            .enumerate()
            .filter(|(_, update)| **update == 0)
            .map(|(index, _)| index as u32)
            .take(64)
            .collect();
        let decision = self.sender.plan_next_update(&fresh, &refresh, now);
        if decision.cells.is_empty() {
            return;
        }
        let update_id = self.sender.next_update_id();
        let total_cells = self.grid.cells();
        let cap = u64::from(self.sender.estimate().max_object_bytes);
        // The harness's media source: a cell bitmap, padded to the source's
        // cost per cell.
        let desired = (decision.cells.len() as u64) * wire::MEDIA_CELL_BYTES;
        let bitmap = wire::bitmap_bytes(total_cells) as u64;
        let source_bytes = desired.max(bitmap);
        // The broken peer emits an object above the cap.
        let payload_bytes = if self.faults.oversize_object {
            source_bytes.max(cap + 1) as usize
        } else {
            source_bytes as usize
        };
        let payload =
            wire::encode_media_payload(&decision.cells, total_cells, payload_bytes, update_id);
        // The pulse is the echo's carrier (MLP spec §6.3.5): the update that
        // goes out while an input is applied carries the input sequence the
        // receiver answers, and it is echo-critical, so the sender's write
        // queue never refuses it and it is released before the bulk.
        let carrier = pulse && self.pulse_pending.is_some();
        let request = UpdateRequest {
            parts: vec![payload],
            cells: decision.cells.clone(),
            anchor: decision.anchor,
            kind: if carrier {
                UpdateKind::Pulse
            } else {
                decision.kind
            },
            reflects_input_seq: if carrier {
                // The stamp is spent only when the carrier is **accepted**: an
                // update the sender refuses (its write queue is full) never
                // left the host, so it never carried the stamp (MLP spec
                // §6.3.5).
                self.pulse_pending.unwrap_or_default()
            } else {
                0
            },
            update_bytes: payload_bytes as u32,
            stream_id: 0,
            path_id: 0,
            echo_critical: carrier,
            deadline_epoch_ms: None,
        };
        let payload_len = request.parts[0].len() as u64;
        let parts_cap = self.sender.scheduler().max_chunks_for(
            self.carrier.is_datagram(),
            self.sender.estimate().loss_ratio,
        );
        match self.sender.enqueue_update(request, now) {
            EnqueueOutcome::Accepted { .. } => {
                if carrier {
                    self.pulse_pending = None;
                }
                for cell in &decision.cells {
                    self.emitted[*cell as usize] = update_id;
                    self.capture_dirty.retain(|dirty| dirty != cell);
                }
                self.record_update(
                    update_id,
                    decision.kind.as_str(),
                    decision.cells.len() as u32,
                    payload_len as u32,
                    decision.anchor,
                    decision.plan_id,
                );
                self.objects.push(ObjectSize {
                    bytes: payload_len,
                    cap,
                });
                self.update_parts.push(UpdateParts {
                    parts: 1,
                    cap: parts_cap,
                });
                if self.capture_dirty.is_empty() {
                    self.last_change_us = self.clock.elapsed_us();
                }
                if let Some(plan_id) = decision.plan_id {
                    self.decisions.plans.push(PlanDecision {
                        at_us: self.clock.elapsed_us(),
                        plan_id,
                        event: "band".to_string(),
                        cells: decision.cells.len() as u32,
                    });
                    if decision.deferred == 0 {
                        self.decisions.plans.push(PlanDecision {
                            at_us: self.clock.elapsed_us(),
                            plan_id,
                            event: "complete".to_string(),
                            cells: 0,
                        });
                    }
                }
            }
            EnqueueOutcome::Oversize { .. } => {
                // The object was refused; the harness still saw it, and
                // INV-6 is about what the peer put on the wire.
                self.objects.push(ObjectSize {
                    bytes: payload_len,
                    cap,
                });
            }
            EnqueueOutcome::Deferred { .. } | EnqueueOutcome::Coalesced { .. } => {}
        }
    }

    /// Take the fragments the sender is ready to release and offer them to
    /// the link.
    fn release_fragments(&mut self, now: std::time::Instant) {
        let mut released = 0;
        while let Some(fragment) = self.sender.poll_fragment(now) {
            let bytes = fragment.bytes.len() as u64;
            let release = Release {
                at_us: self.clock.elapsed_us(),
                bytes,
                budget_bytes: self.sender.pacer_window_budget_bytes(),
            };
            self.releases.push(release);
            self.recent_releases.push_back(release);
            // The hop's writer drains by the object's own priority, and the
            // echo's carrier goes before the bulk (MLP spec §3.2 rule 2,
            // §6.3.5).
            let (priority, echo_critical) = match Fragment::parse(&fragment.bytes, Carrier::Grpc) {
                Ok((parsed, _)) => (
                    parsed.header.priority.get(),
                    parsed.header.flags.contains(FragmentFlags::ECHO_REQUESTED),
                ),
                Err(_) => (Priority::MEDIA.get(), false),
            };
            self.link
                .offer_with(Direction::Down, fragment.bytes, priority, echo_critical);
            released += 1;
            if released > 256 {
                break;
            }
        }
        // The broken peer lets the whole window through at once.
        if self.faults.burst_over_budget && !self.burst_injected {
            self.burst_injected = true;
            let budget = self.sender.pacer_window_budget_bytes();
            let release = Release {
                at_us: self.clock.elapsed_us(),
                bytes: budget,
                budget_bytes: budget,
            };
            self.releases.push(release);
            self.recent_releases.push_back(release);
        }
    }

    // ── the receiver side ──────────────────────────────────────────────────

    /// The receiver's obligations: its timers, its reports, its input.
    fn receiver_side(&mut self) {
        let now = self.clock.instant();
        let events = self.receiver.tick(now);
        for event in events {
            match event {
                MediaReceiveEvent::FeedbackDue => self.report(),
                MediaReceiveEvent::ProbeDue(probe) => {
                    let payload = wire::encode_probe(&probe);
                    self.send_up(FragmentClass::Feedback, Priority::REPORT, payload);
                }
                MediaReceiveEvent::ProbeEchoDue(echo) => {
                    let payload = wire::encode_echo(&echo);
                    self.send_up(FragmentClass::Feedback, Priority::REPORT, payload);
                }
                MediaReceiveEvent::Object(_)
                | MediaReceiveEvent::ParityRecovered(_)
                | MediaReceiveEvent::FragmentDropped { .. }
                | MediaReceiveEvent::ObjectDropped { .. } => {}
            }
        }
        self.send_input();
        self.release_stuck_keys();
    }

    /// The receiver's periodic report.
    fn report(&mut self) {
        let now = self.clock.instant();
        if !self.receiver.feedback_due(now) {
            return;
        }
        self.receiver
            .set_viewer_state(self.pending_cells(), self.grid.cells(), self.converged());
        let feedback = self.receiver.build_feedback(now);
        let highest = feedback
            .ack
            .map(|ack| ack.highest_applied_update_id)
            .unwrap_or(0);
        self.decisions.acks.push(AckDecision {
            at_us: self.clock.elapsed_us(),
            highest_applied_update_id: highest,
            pending_cells: self.pending_cells(),
            converged: self.converged(),
        });
        let payload = wire::encode_feedback(&feedback);
        self.send_up(FragmentClass::Feedback, Priority::REPORT, payload);
    }

    /// The receiver's input objects, on their way to the sender.
    fn send_input(&mut self) {
        let now_ms = self.clock.now_ms();
        let envelopes = self.input_queue.take_ready(now_ms);
        for envelope in envelopes {
            self.input_sent_us
                .insert(envelope.input_seq, self.clock.elapsed_us());
            if self.faults.unbounded_queue {
                // The broken deployment queues the input objects and never
                // sends them: its backlog grows without a cap.
                self.unsent_inputs += 1;
                continue;
            }
            let payload = wire::encode_input(&envelope);
            // The input path stays on the control lane, at the input
            // priority (MLP spec §3.1, §8.3).
            self.send_up(FragmentClass::Control, Priority::INPUT, payload);
        }
        // The pressed-set heartbeat: the safety net for a key release the
        // link lost (MLP spec §8.3.4).
        if let Some(heartbeat) = self.input_queue.poll_pressed_heartbeat(now_ms) {
            self.input_sent_us
                .insert(heartbeat.input_seq, self.clock.elapsed_us());
            let payload = wire::encode_input(&heartbeat);
            self.send_up(FragmentClass::Control, Priority::INPUT, payload);
        }
    }

    /// Send an object from the receiver to the sender.
    fn send_up(&mut self, class: FragmentClass, priority: Priority, payload: Vec<u8>) {
        let object_id = self.next_up_object_id();
        let fragment = match Fragment::new(
            class,
            FragmentFlags::FIN,
            priority,
            object_id,
            0,
            1,
            HeaderExt::new(),
            payload,
        ) {
            Ok(fragment) => fragment,
            Err(_) => return,
        };
        self.link
            .offer_with(Direction::Up, fragment.encode(), priority.get(), false);
    }

    /// The next object id the receiver's side uses.
    fn next_up_object_id(&mut self) -> u32 {
        let id = self.up_object_id;
        self.up_object_id = self.up_object_id.wrapping_add(1).max(UP_OBJECT_ID_BASE);
        id
    }

    // ── the wire ───────────────────────────────────────────────────────────

    /// What the receiver receives.
    fn receiver_receive(&mut self, bytes: &[u8]) {
        let now = self.clock.instant();
        let events = self.receiver.on_message(bytes, now);
        for event in events {
            match event {
                MediaReceiveEvent::ParityRecovered(object) => {
                    // A lost part the parity answered on arrival (MLP spec
                    // §6.9): no round trip was spent on it.
                    self.parity_recovered += 1;
                    self.apply_received_object(object, now);
                }
                MediaReceiveEvent::Object(object) => {
                    self.apply_received_object(object, now);
                }
                MediaReceiveEvent::FragmentDropped { .. }
                | MediaReceiveEvent::ObjectDropped { .. } => {}
                MediaReceiveEvent::FeedbackDue => self.report(),
                MediaReceiveEvent::ProbeEchoDue(echo) => {
                    let payload = wire::encode_echo(&echo);
                    self.send_up(FragmentClass::Feedback, Priority::REPORT, payload);
                }
                MediaReceiveEvent::ProbeDue(probe) => {
                    let payload = wire::encode_probe(&probe);
                    self.send_up(FragmentClass::Feedback, Priority::REPORT, payload);
                }
            }
        }
    }

    /// The receiver's application rules for one arrived media object, wire
    /// or parity-recovered (MLP spec §5.1, §6.9).
    fn apply_received_object(
        &mut self,
        object: mlp_core::media::MediaObject,
        now: std::time::Instant,
    ) {
        if std::env::var_os("MLP_HARNESS_DEBUG").is_some() {
            eprintln!(
                "RECV t={} id={} class={:?} lost={} reord={}",
                self.clock.elapsed_us(),
                object.object_id,
                object.class,
                self.receiver.stats().lost_objects,
                self.receiver.stats().reordered_objects
            );
        }
        if let Some(echo) = wire::decode_echo(&object.payload) {
            self.receiver.on_probe_echo(echo, now);
            return;
        }
        if object.class != FragmentClass::Media {
            return;
        }
        let Some(meta) = object.update_meta() else {
            return;
        };
        // A chained update applies only onto the state it chains from; an
        // anchor applies onto any held state.
        let anchor = object.is_anchor();
        if !anchor && meta.update_base != 0 && !self.received_updates.contains(&meta.update_base) {
            // A gap: the update is refused whole, its cells stay pending,
            // and the sender repairs them.
            self.gaps += 1;
            return;
        }
        self.apply_update(
            meta.update_id,
            meta.part_index,
            meta.part_count,
            meta.reflects_input_seq,
            anchor,
            &object.payload,
        );
    }

    /// Apply an update at the receiver.
    #[allow(clippy::too_many_arguments)]
    fn apply_update(
        &mut self,
        update_id: u64,
        part_index: u16,
        part_count: u16,
        reflects_input_seq: u64,
        anchor: bool,
        payload: &[u8],
    ) {
        if self.receiver.is_chunk_applied(update_id, part_index) {
            return;
        }
        let total_cells = self.grid.cells();
        let cells = wire::decode_media_payload(payload, total_cells);
        // INV-1: the machine only completes an object whose every part
        // arrived. The broken peer does not wait for that.
        let parts_present = if self.faults.apply_partial_update {
            part_count.saturating_sub(1)
        } else {
            part_count
        };
        self.receiver.note_applied_chunk(update_id, part_index);
        self.applied_updates.push(AppliedUpdate {
            update_id,
            part_count,
            parts_present,
        });
        let mut applied_cells = Vec::with_capacity(cells.len());
        for cell in &cells {
            // A cell whose held update is newer is not overwritten: the
            // receiver keeps the newer state and says so.
            if !anchor && self.held[*cell as usize] > update_id {
                self.stale_applies_skipped += 1;
                continue;
            }
            self.held[*cell as usize] = update_id;
            applied_cells.push(*cell);
        }
        self.received_updates.insert(update_id);
        // The receiver's own report of what its applied content reflects
        // (MLP spec §5.2, annex N-1): an anchor replaces the surface, so the
        // content that reflected the older input is gone and the report goes
        // back to what the anchor reflects; a delta only ever raises it.
        let echo = if anchor {
            reflects_input_seq
        } else {
            self.receiver_echo.max(reflects_input_seq)
        };
        self.receiver_echo = echo;
        self.receiver.set_echo_of_input_seq(echo);
        self.receiver.note_applied(update_id);
        self.measure_echo(reflects_input_seq);
        // The prompt acknowledgement: the update is applied, so the sender
        // learns of it at once.
        let now = self.clock.instant();
        let ack = self.receiver.build_ack(now);
        for cell in &applied_cells {
            // The broken peer acknowledges a cell the sender never emitted.
            let acknowledged = if self.faults.acknowledge_unemitted_cell {
                update_id + 1_000_000
            } else {
                update_id
            };
            self.acknowledged.insert(*cell, acknowledged);
        }
        let payload = wire::encode_ack(&ack);
        self.send_up(FragmentClass::Feedback, Priority::CONTROL, payload);
    }

    /// Measure the echo the way a **sender** measures it: from the moment the
    /// sender applied the input to the moment the acknowledgement that names
    /// it arrives back.
    ///
    /// The receiver-side measurement ([`Self::measure_echo`]) is the round trip
    /// the receiver sees: the input's send to the update's apply. The
    /// sender-side measurement is the other half of the same path: the update's
    /// apply to the acknowledgement's arrival. A deployment measures the second
    /// one (`rm_host_media_echo_seconds`), so the harness measures it too, and
    /// the echo's promise (annex INV-8) is checked on it.
    fn measure_sender_echo(&mut self, echo_seq: u64) {
        if echo_seq == 0 {
            return;
        }
        if !self.sender_echoed_inputs.insert(echo_seq) {
            return;
        }
        let Some(applied_at_us) = self
            .input_applications
            .iter()
            .find(|application| application.seq == echo_seq)
            .map(|application| application.applied_at_us)
        else {
            return;
        };
        let echo_us = self.clock.elapsed_us().saturating_sub(applied_at_us);
        self.decisions.sender_echoes.push(EchoSample {
            input_seq: echo_seq,
            echo_ms: echo_us / 1_000,
            at_us: self.clock.elapsed_us(),
        });
    }

    /// Measure the echo of an input, when an update reflects one.
    ///
    /// The echo is measured **once per input**: the first update whose
    /// content reflects the input is the measurement, and the later updates
    /// that carry the same `reflects_input_seq` are not new measurements.
    fn measure_echo(&mut self, reflects_input_seq: u64) {
        if reflects_input_seq == 0 {
            return;
        }
        if !self.echoed_inputs.insert(reflects_input_seq) {
            return;
        }
        let Some(sent_us) = self.input_sent_us.get(&reflects_input_seq).copied() else {
            if std::env::var_os("MLP_HARNESS_DEBUG").is_some() {
                eprintln!(
                    "ECHO MISS seq={} sent={:?}",
                    reflects_input_seq,
                    self.input_sent_us.keys().collect::<Vec<_>>()
                );
            }
            return;
        };
        let echo_us = self.clock.elapsed_us().saturating_sub(sent_us);
        let mut echo_ms = echo_us / 1_000;
        if self.faults.delay_echo {
            // The broken peer answers a whole budget late: the echo's p95
            // then sits outside the budget it is measured against.
            echo_ms += 4 * self.tier.echo_budget_ms();
        }
        self.decisions.echoes.push(EchoSample {
            input_seq: reflects_input_seq,
            echo_ms,
            at_us: self.clock.elapsed_us(),
        });
    }

    /// What the sender receives.
    fn sender_receive(&mut self, bytes: &[u8]) {
        let now = self.clock.instant();
        let events = self.host_rx.on_message(bytes, now);
        for event in events {
            let MediaReceiveEvent::Object(object) = event else {
                continue;
            };
            if let Some(ack) = wire::decode_ack(&object.payload) {
                self.acks_received += 1;
                self.measure_sender_echo(ack.echo_of_input_seq);
                self.sender.on_ack(ack, now);
                continue;
            }
            if let Some(feedback) = wire::decode_feedback(&object.payload) {
                self.feedbacks_received += 1;
                self.sender.on_feedback(feedback, now);
                continue;
            }
            if let Some(envelope) = wire::decode_input(&object.payload) {
                self.inputs_received += 1;
                self.apply_input(envelope);
                continue;
            }
            if let Some(echo) = wire::decode_echo(&object.payload) {
                self.sender.on_probe_echo(echo, now);
            }
        }
    }

    /// Apply an input at the sender.
    fn apply_input(&mut self, envelope: MediaInputEnvelope) {
        let now_ms = self.clock.now_ms();
        let always_applied = envelope.is_always_applied();
        let expired = envelope.is_expired(now_ms);
        let is_release = envelope
            .flags
            .contains(mlp_core::media::input::InputFlags::RELEASE)
            || matches!(envelope.kind, InputKind::ReleaseAll);
        // The broken peer never learns that a key was released.
        if self.faults.keep_key_held_after_release && is_release {
            self.decisions.inputs.push(InputDecision {
                at_us: self.clock.elapsed_us(),
                seq: envelope.input_seq,
                kind: envelope.kind.as_str().to_string(),
                applied: false,
                expired,
            });
            return;
        }
        let outcome = self.host_input.submit(&envelope, now_ms);
        let applied = outcome.accepted;
        if applied {
            // On applying an input the sender emits an immediate pulse
            // update covering the focus region, so the echo promise is met
            // on the very next update rather than on the next idle one.
            self.sender
                .request_pulse(CellRect::new(96, 96, 64, 64), self.clock.instant());
            self.input_applications.push(InputApplication {
                seq: envelope.input_seq,
                applied_at_us: self.clock.elapsed_us(),
                deadline_us: envelope.deadline_epoch_ms * 1_000,
                always_applied,
            });
            // The input is applied: the sender schedules the pulse that carries
            // its sequence (MLP spec §6.3.5).
            self.pulse_pending = Some(envelope.input_seq);
        }
        // The broken peer applies the same input twice, or applies one that
        // has already expired.
        if self.faults.apply_input_twice && applied {
            self.input_applications.push(InputApplication {
                seq: envelope.input_seq,
                applied_at_us: self.clock.elapsed_us(),
                deadline_us: envelope.deadline_epoch_ms * 1_000,
                always_applied,
            });
        }
        if self.faults.apply_input_after_deadline && applied {
            self.input_applications.push(InputApplication {
                seq: envelope.input_seq + 1_000_000,
                applied_at_us: self.clock.elapsed_us(),
                deadline_us: envelope.deadline_epoch_ms * 1_000,
                always_applied: false,
            });
        }
        self.decisions.inputs.push(InputDecision {
            at_us: self.clock.elapsed_us(),
            seq: envelope.input_seq,
            kind: envelope.kind.as_str().to_string(),
            applied,
            expired,
        });
        // The injection worker is the deployment's, and the deployment
        // drains it. A peer that never does grows a queue without a cap.
        if !self.faults.unbounded_queue {
            let _ = self.host_input.drain_worker(8);
        }
        // The host's pressed set is the deployment's: the input object
        // carries the key, and the deployment holds it down. The transport
        // guarantees the object arrived; what the host does with it is the
        // host's business.
        let is_release = envelope
            .flags
            .contains(mlp_core::media::input::InputFlags::RELEASE);
        match envelope.kind {
            InputKind::Key => {
                let code = u32::from_le_bytes(
                    envelope.payload[..4.min(envelope.payload.len())]
                        .to_vec()
                        .try_into()
                        .unwrap_or([0, 0, 0, 0]),
                );
                if is_release {
                    self.host_pressed_keys.remove(&code);
                } else {
                    self.host_pressed_keys.insert(code);
                }
            }
            InputKind::PointerButton => {
                let code = u32::from_le_bytes(
                    envelope.payload[..4.min(envelope.payload.len())]
                        .to_vec()
                        .try_into()
                        .unwrap_or([0, 0, 0, 0]),
                );
                if is_release {
                    self.host_pressed_buttons.remove(&code);
                } else {
                    self.host_pressed_buttons.insert(code);
                }
            }
            InputKind::ReleaseAll => {
                self.host_pressed_keys.clear();
                self.host_pressed_buttons.clear();
            }
            InputKind::PressedSet => {
                // The pressed set states what the receiver holds: the host
                // releases everything the set does not name.
                if let Some((keys, buttons)) =
                    mlp_core::media::input::decode_pressed_set(&envelope.payload)
                {
                    self.host_pressed_keys.retain(|code| keys.contains(code));
                    self.host_pressed_buttons
                        .retain(|code| buttons.contains(code));
                }
            }
            _ => {}
        }
    }

    /// Release the keys the grace period has released.
    ///
    /// On a park, the sender releases everything for that receiver once the
    /// grace elapses (MLP spec §8.3.4). A peer that ignores the grace
    /// keeps its keys down.
    fn release_stuck_keys(&mut self) {
        if self.faults.keep_key_held_after_release {
            return;
        }
        let now_ms = self.clock.now_ms();
        if self.host_input.poll_grace_release(now_ms).is_some() {
            self.host_pressed_keys.clear();
            self.host_pressed_buttons.clear();
        }
    }

    // ── the session ────────────────────────────────────────────────────────

    /// The capture produced a change in a region.
    fn capture_change(&mut self, region: crate::scenario::Region) {
        self.convergence_us = None;
        let rect = CellRect::new(region.x_px, region.y_px, region.width_px, region.height_px);
        for cell in self.grid.cells_in_rect(&rect) {
            if !self.capture_dirty.contains(&cell) {
                self.capture_dirty.push(cell);
            }
        }
        self.capture_dirty.sort_unstable();
        self.last_change_us = self.clock.elapsed_us();
    }

    /// The user did something.
    fn viewer_input(&mut self, event: &InputEvent) {
        let now_ms = self.clock.now_ms();
        let outcome = match event {
            InputEvent::Key { code, pressed } => {
                let payload = InputEvent::key_code(code).to_le_bytes().to_vec();
                self.input_queue.push_key(
                    InputEvent::key_code(code),
                    *pressed,
                    false,
                    payload,
                    now_ms,
                )
            }
            InputEvent::PointerMove { x_px, y_px } => {
                let payload = [x_px.to_le_bytes(), y_px.to_le_bytes()].concat();
                self.input_queue.push_pointer_move(payload, now_ms)
            }
            InputEvent::PointerButton { button, pressed } => {
                let payload = button.to_le_bytes().to_vec();
                self.input_queue
                    .push_pointer_button(*button, *pressed, payload, now_ms)
            }
            InputEvent::Wheel { delta } => {
                let payload = delta.to_le_bytes().to_vec();
                self.input_queue.push_wheel(*delta, payload, now_ms)
            }
            InputEvent::Clipboard { text } => {
                let payload = text.as_bytes().to_vec();
                self.input_queue.push_clipboard(payload, now_ms)
            }
            InputEvent::ReleaseAll => {
                let payload = Vec::new();
                self.input_queue.push_release_all(payload, now_ms)
            }
            InputEvent::PressedSet => {
                // The receiver states what it holds; the queue builds the
                // envelope the pressed-set protocol carries.
                let envelope = self.input_queue.pressed_set_envelope(now_ms);
                self.input_sent_us
                    .insert(envelope.input_seq, self.clock.elapsed_us());
                let payload = wire::encode_input(&envelope);
                self.send_up(FragmentClass::Control, Priority::INPUT, payload);
                PushOutcome::Queued
            }
        };
        // The broken peer keeps the inputs it should have dropped, so its
        // queue grows past its cap.
        if self.faults.unbounded_queue && matches!(outcome, PushOutcome::Dropped(_)) {
            self.retained_inputs += 1;
        }
        let _ = &outcome;
    }

    /// The receiver's lane died.
    fn disconnect(&mut self) {
        self.connected = false;
        self.parked = true;
        self.parked_since_ms = Some(self.clock.now_ms());
        self.parks += 1;
        self.input_queue.set_connected(false, self.clock.now_ms());
        self.host_input.on_park(self.clock.now_ms());
        self.decisions.states.push(StateDecision {
            at_us: self.clock.elapsed_us(),
            state: "parked".to_string(),
        });
    }

    /// The receiver's lane came back.
    fn reconnect(&mut self) {
        self.connected = true;
        self.parked = false;
        self.parked_since_ms = None;
        self.input_queue.set_connected(true, self.clock.now_ms());
        self.host_input.on_resume();
        if self.faults.recreate_session {
            // The broken peer creates a second lane for the same receiver.
            self.lanes_created += 1;
        }
        self.decisions.states.push(StateDecision {
            at_us: self.clock.elapsed_us(),
            state: "resumed".to_string(),
        });
    }

    // ── the observation ────────────────────────────────────────────────────

    /// How many cells the sender still owes the receiver.
    fn pending_cells(&self) -> u32 {
        self.sender.accounting().pending_count()
    }

    /// Whether the tier's echo promise applies to this run.
    ///
    /// The promise is a promise about a link inside the tier's envelope: a
    /// link with an outage, or with loss beyond the tier's bound, is
    /// outside it, and the promise does not apply there.
    fn echo_promise_applies(&self) -> bool {
        let profile = self.link.profile();
        self.tier != Tier::T4
            && profile.outages.windows.is_empty()
            && profile.down.loss.worst_case_permille() <= 100
            && profile.up.loss.worst_case_permille() <= 100
            && profile.within_envelope(self.tier)
    }

    /// The echo's budget: `2 × RTT + 250 ms`, on the **measured** round
    /// trip, plus one repair delay (MLP spec §2.1, §9.1).
    ///
    /// The tier's table states the budget for the tier's nominal round trip;
    /// the protocol's own rule is on the round trip the estimator measured,
    /// because an implementation never assumes a tier in advance.
    fn echo_budget_ms(&self) -> u64 {
        let rtt_ms = self.sender.estimator().rtt_ms().max(1.0);
        let base = (2.0 * rtt_ms + 250.0).round() as u64;
        base + self.sender.accounting().repair_delay_ms()
    }

    /// Whether the sender's accounting reports convergence.
    fn converged(&self) -> bool {
        self.sender.accounting().converged()
    }

    /// What the harness observed, right now.
    fn observation(&self) -> Observation {
        // Only the cells the receiver acknowledged matter to INV-2, and
        // there are few of them: the observation is built on every tick, so
        // it must not carry the whole surface.
        let cells: Vec<CellState> = self
            .acknowledged
            .iter()
            .map(|(cell, acknowledged)| CellState {
                cell: *cell,
                emitted: self.emitted[*cell as usize],
                held: self.held[*cell as usize],
                acknowledged: *acknowledged,
            })
            .collect();
        let sender_keys: Vec<u32> = self.host_pressed_keys.iter().copied().collect();
        let sender_buttons: Vec<u32> = self.host_pressed_buttons.iter().copied().collect();
        let (receiver_keys, receiver_buttons) = self.input_queue.pressed_set();
        if std::env::var_os("MLP_HARNESS_DEBUG").is_some() {
            eprintln!(
                "PRESSED t={} sender={:?} receiver={:?} parked={} connected={}",
                self.clock.elapsed_us(),
                sender_keys,
                receiver_keys,
                self.parked,
                self.connected
            );
        }
        let mut sender_pressed = sender_keys;
        sender_pressed.extend(sender_buttons);
        let mut receiver_pressed = receiver_keys;
        receiver_pressed.extend(receiver_buttons);
        let link_stats = self.link.stats();
        let sender_stats = *self.sender.stats();
        let link_cap =
            self.link.profile().down.queue_cap_bytes + self.link.profile().up.queue_cap_bytes;
        Observation {
            now_us: self.clock.elapsed_us(),
            applied_updates: self.applied_updates.clone(),
            cells,
            input_applications: self.input_applications.clone(),
            sender_pressed,
            receiver_pressed,
            parked: self.parked,
            session_ended: !self.connected,
            queues: vec![
                QueueDepth {
                    name: "sender objects".to_string(),
                    depth: self.sender.queued_objects() as u64,
                    cap: 256,
                },
                QueueDepth {
                    name: "sender input worker".to_string(),
                    depth: self.host_input.worker_depth() as u64,
                    cap: 64,
                },
                QueueDepth {
                    name: "receiver input queue".to_string(),
                    depth: self.input_queue.len() as u64
                        + self.retained_inputs as u64
                        + self.unsent_inputs as u64,
                    cap: self.input_queue.object_capacity() as u64,
                },
                QueueDepth {
                    name: "receiver open objects".to_string(),
                    depth: self.receiver.assembler().open_objects() as u64,
                    cap: 64,
                },
                QueueDepth {
                    name: "link".to_string(),
                    depth: self.link.queued_bytes(),
                    cap: link_cap,
                },
            ],
            objects: self.objects.clone(),
            update_parts: self.update_parts.clone(),
            releases: self.recent_releases.iter().copied().collect(),
            window_budget_bytes: self.sender.pacer_window_budget_bytes(),
            echoes: self
                .decisions
                .echoes
                .iter()
                .map(|sample| sample.echo_ms)
                .collect(),
            sender_echoes: self
                .decisions
                .sender_echoes
                .iter()
                .map(|sample| sample.echo_ms)
                .collect(),
            echo_budget_ms: self.echo_budget_ms(),
            echo_promised: self.echo_promise_applies(),
            lanes_created: self.lanes_created,
            session_lost: self.lanes_created > 1,
            link_dropped: link_stats.down.dropped(),
            sender_dropped: sender_stats.queue_overflow_drops_total
                + sender_stats.in_flight_evictions_total
                + sender_stats.superseded_total
                + sender_stats.object_oversize_total,
            hop_dropped: self.hop_dropped,
            receiver_reported_lost: self.receiver.stats().lost_objects + self.unattributed_claims,
            receiver_reordered: self.receiver.stats().reordered_objects,
        }
    }

    /// Check the pressed set at the moment the park's grace elapses.
    ///
    /// INV-4 is about that moment: after a park beyond the grace period,
    /// the sender's pressed set is empty. Checking it at the end of the run
    /// would check a different moment — one in which the receiver may have
    /// come back.
    fn check_park_grace(&mut self) {
        if !self.parked {
            return;
        }
        let grace = mlp_core::media::input::KEY_RELEASE_GRACE_MS;
        let Some(parked_since_ms) = self.parked_since_ms else {
            return;
        };
        if self.clock.now_ms().saturating_sub(parked_since_ms) < grace {
            return;
        }
        let observation = self.observation();
        self.checker.check(Invariant::Inv4, &observation);
    }

    /// Drop the releases that have left the one-second window.
    fn trim_releases(&mut self) {
        let now = self.clock.elapsed_us();
        while let Some(front) = self.recent_releases.front() {
            if front.at_us.saturating_add(1_000_000) <= now {
                self.recent_releases.pop_front();
            } else {
                break;
            }
        }
    }

    /// Check the cheap invariants against the current observation.
    ///
    /// The cheap ones are the ones a *state* decides: a queue's depth, an
    /// object's size, a window's bytes, a cell's state. The others depend
    /// on a moment in the run — the echo, the session's continuity, the
    /// loss attribution — and the scenario asks for them where it wants
    /// them checked.
    fn observe(&mut self) {
        if self.converged() && self.capture_dirty.is_empty() {
            self.convergences += 1;
            if self.convergence_us.is_none() {
                // The time to converge, measured from the last change.
                self.convergence_us =
                    Some(self.clock.elapsed_us().saturating_sub(self.last_change_us));
            }
        }
        self.trim_releases();
        let observation = self.observation();
        for invariant in Invariant::cheap() {
            self.checker.check(invariant, &observation);
        }
    }

    /// Record what the sender decided.
    fn note_sender_events(&mut self, events: &[MediaSenderEvent]) {
        for event in events {
            match event {
                MediaSenderEvent::PlanStarted {
                    plan_id,
                    reason,
                    cells,
                    ..
                } => {
                    self.decisions.plans.push(PlanDecision {
                        at_us: self.clock.elapsed_us(),
                        plan_id: *plan_id,
                        event: format!("start:{}", reason.as_str()),
                        cells: *cells,
                    });
                }
                MediaSenderEvent::PlanOverdue { plan_id } => {
                    self.decisions.plans.push(PlanDecision {
                        at_us: self.clock.elapsed_us(),
                        plan_id: *plan_id,
                        event: "overdue".to_string(),
                        cells: 0,
                    });
                }
                MediaSenderEvent::LaneStateChanged(state) => {
                    self.decisions.states.push(StateDecision {
                        at_us: self.clock.elapsed_us(),
                        state: state.as_str().to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    /// Record an update in the decisions.
    fn record_update(
        &mut self,
        update_id: u64,
        kind: &str,
        cells: u32,
        bytes: u32,
        anchor: bool,
        plan_id: Option<u32>,
    ) {
        self.decisions.updates.push(UpdateDecision {
            at_us: self.clock.elapsed_us(),
            update_id,
            kind: kind.to_string(),
            cells,
            bytes,
            anchor,
            plan_id,
            reflects_input_seq: self.host_input.last_applied_seq(),
        });
    }

    /// Finish the run and build the trace.
    fn finish(mut self, scenario: &Scenario) -> Trace {
        self.observe();
        // The end of the run is a moment too: the pressed set must be empty
        // and the echo must be inside its budget.
        {
            let observation = self.observation();
            self.checker.check(Invariant::Inv4, &observation);
            self.checker.check(Invariant::Inv8, &observation);
        }
        self.link_events = self.link.events().to_vec();
        let digest = Trace::compute_digest(&self.decisions);
        let measurements = self.measurements();
        let summary = ScenarioSummary {
            name: scenario.name.clone(),
            tier: scenario.tier,
            seed: scenario.seed,
            steps: scenario.steps.len(),
            duration_ms: self.clock.elapsed_ms(),
            faults: scenario
                .faults
                .active()
                .into_iter()
                .map(str::to_string)
                .collect(),
        };
        Trace {
            schema: crate::trace::TRACE_SCHEMA,
            scenario: summary,
            link: crate::trace::LinkSummary {
                profile: self.link.profile().clone(),
                down: self.link.stats().down,
                up: self.link.stats().up,
                events: self.link_events.clone(),
            },
            decisions: self.decisions.clone(),
            invariants: self.checker.outcomes(),
            violations: self.checker.violations().to_vec(),
            measurements,
            digest,
        }
    }

    /// The run's measurements.
    fn measurements(&self) -> Measurements {
        let sender_stats = *self.sender.stats();
        let receiver_stats = *self.receiver.stats();
        let mut echoes: Vec<u64> = self
            .decisions
            .echoes
            .iter()
            .map(|sample| sample.echo_ms)
            .collect();
        echoes.sort_unstable();
        let percentile = |fraction: f64| -> Option<u64> {
            if echoes.is_empty() {
                return None;
            }
            let index = ((echoes.len() as f64 * fraction).ceil() as usize).saturating_sub(1);
            echoes.get(index.min(echoes.len() - 1)).copied()
        };
        // The sender-side echo: the round trip the sender measures, and the
        // one a deployment measures (`rm_host_media_echo_seconds`).
        let mut sender_echoes: Vec<u64> = self
            .decisions
            .sender_echoes
            .iter()
            .map(|sample| sample.echo_ms)
            .collect();
        sender_echoes.sort_unstable();
        let sender_percentile = |fraction: f64| -> Option<u64> {
            if sender_echoes.is_empty() {
                return None;
            }
            let index = ((sender_echoes.len() as f64 * fraction).ceil() as usize).saturating_sub(1);
            sender_echoes
                .get(index.min(sender_echoes.len() - 1))
                .copied()
        };
        let link_stats = self.link.stats();
        Measurements {
            echo_p50_ms: percentile(0.50),
            echo_p95_ms: percentile(0.95),
            echo_max_ms: echoes.last().copied(),
            echo_samples: echoes.len() as u64,
            sender_echo_p50_ms: sender_percentile(0.50),
            sender_echo_p95_ms: sender_percentile(0.95),
            sender_echo_max_ms: sender_echoes.last().copied(),
            sender_echo_samples: sender_echoes.len() as u64,
            convergence_ms: self.convergence_ms(),
            convergences: self.convergences,
            bytes_down: link_stats.down.bytes_offered,
            bytes_up: link_stats.up.bytes_offered,
            updates_applied: self.applied_updates.len() as u64,
            updates_sent: self.decisions.updates.len() as u64,
            objects_delivered: link_stats.delivered(),
            objects_dropped: link_stats.dropped(),
            objects_lost: self.receiver.stats().lost_objects,
            rtt_ms: self.sender.estimator().rtt_ms(),
            pace_bps: self.sender.estimate().pace_bps,
            target_fps: self.sender.estimate().target_fps,
            feedbacks_received: self.feedbacks_received,
            acks_received: self.acks_received,
            inputs_received: self.inputs_received,
            objects_enqueued: sender_stats.objects_enqueued,
            fragments_sent: sender_stats.fragments_sent,
            queue_overflow_drops: sender_stats.queue_overflow_drops_total,
            in_flight_deferred_total: sender_stats.in_flight_deferred_total,
            echo_critical_replacements_total: sender_stats.echo_critical_replacements_total,
            in_flight_evictions_total: sender_stats.in_flight_evictions_total,
            superseded: sender_stats.superseded_total,
            oversize_total: sender_stats.object_oversize_total,
            probes_sent: sender_stats.probes_sent,
            receiver_reordered: receiver_stats.reordered_objects,
            receiver_duplicated: receiver_stats.duplicate_objects,
            receiver_dropped_objects: receiver_stats.dropped_objects,
            pending_cells: self.pending_cells(),
            cells: self.grid.cells(),
            duration_ms: self.clock.elapsed_ms(),
        }
    }

    /// The time from the last capture change to the convergence.
    fn convergence_ms(&self) -> Option<u64> {
        self.convergence_us.map(|micros| micros / 1_000)
    }

    /// The pace the sender remembered for this link, when it has one.
    pub fn prior_bps(&self) -> Option<u64> {
        self.prior_bps
    }
}

/// The carrier a link's MTU implies.
///
/// A link whose MTU is the data channel's cap carries the data channel;
/// every other link carries the stream. The rule is the protocol's own
/// (MLP spec §4.1), and a consumer that needs the same answer reads it
/// here rather than repeating it.
pub fn carrier_for(mtu_bytes: u64) -> Carrier {
    if mtu_bytes <= 1_150 {
        Carrier::DataChannel
    } else {
        Carrier::Ws
    }
}

/// The path a label names.
fn path_for(label: &str) -> Option<MediaPath> {
    match label {
        "ws" => Some(MediaPath::Ws),
        "dc" | "dc-direct" => Some(MediaPath::DcDirect),
        "dc-turn" => Some(MediaPath::DcTurn),
        "quic" => Some(MediaPath::Quic),
        _ => None,
    }
}
