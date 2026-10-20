//! The driver's body codec.
//!
//! The harness drives the protocol's machines, so the bytes it puts on the
//! link are real MLP fragments: they are built with the protocol's own
//! fragment header, they travel over the modelled link, and they are parsed
//! by the protocol's own receiver machine. What this module provides is the
//! **body** encoding for the classes whose bodies the specification leaves
//! to the deployment — the acknowledgement, the feedback report, the probe
//! echo and the input envelope.
//!
//! The encoding is compact and binary on purpose: the harness predicts *how
//! many bytes should cross the link*, and a body padded with text would make
//! that number a property of the harness rather than of the protocol.
//!
//! Every body carries a short marker, so a peer can tell the classes apart
//! without parsing every body.

use mlp_core::media::input::{InputFlags, InputKind, MediaInputEnvelope};
use mlp_core::media::{MediaAck, MediaFeedback, ProbeEcho};

/// The acknowledgement body's marker.
pub const ACK_MARKER: &[u8; 3] = b"ack";
/// The feedback report's marker.
pub const FEEDBACK_MARKER: &[u8; 2] = b"fb";
/// The probe echo's marker.
pub const ECHO_MARKER: &[u8; 4] = b"echo";
/// The input envelope's marker.
pub const INPUT_MARKER: &[u8; 5] = b"input";
/// The probe's marker.
pub const PROBE_MARKER: &[u8; 5] = b"probe";

/// Encode an acknowledgement.
pub fn encode_ack(ack: &MediaAck) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(ACK_MARKER);
    out.extend_from_slice(&ack.highest_applied_update_id.to_le_bytes());
    out.extend_from_slice(&ack.ack_bitmap.to_le_bytes());
    out.extend_from_slice(&ack.chain_epoch.to_le_bytes());
    out.extend_from_slice(&ack.applied_cell_count.to_le_bytes());
    out.extend_from_slice(&ack.stale_cells.to_le_bytes());
    out.push(u8::from(ack.converged));
    out.extend_from_slice(&ack.render_latency_ms.to_le_bytes());
    out.push(u8::from(ack.has_render_latency_ms));
    out.extend_from_slice(&ack.echo_of_input_seq.to_le_bytes());
    out.extend_from_slice(&ack.ack_epoch_ms.to_le_bytes());
    out
}

/// Decode an acknowledgement.
pub fn decode_ack(payload: &[u8]) -> Option<MediaAck> {
    if payload.len() < ACK_MARKER.len() + 8 + 8 + 4 + 4 + 4 + 1 + 8 + 1 + 8 + 8
        || &payload[..ACK_MARKER.len()] != ACK_MARKER
    {
        return None;
    }
    let mut cursor = ACK_MARKER.len();
    let mut take = |count: usize| -> Option<Vec<u8>> {
        if cursor + count > payload.len() {
            return None;
        }
        let slice = payload[cursor..cursor + count].to_vec();
        cursor += count;
        Some(slice)
    };
    let ack = MediaAck {
        highest_applied_update_id: u64::from_le_bytes(take(8)?.try_into().ok()?),
        ack_bitmap: u64::from_le_bytes(take(8)?.try_into().ok()?),
        chain_epoch: u32::from_le_bytes(take(4)?.try_into().ok()?),
        applied_cell_count: u32::from_le_bytes(take(4)?.try_into().ok()?),
        stale_cells: u32::from_le_bytes(take(4)?.try_into().ok()?),
        converged: take(1)?[0] != 0,
        render_latency_ms: f64::from_le_bytes(take(8)?.try_into().ok()?),
        has_render_latency_ms: take(1)?[0] != 0,
        echo_of_input_seq: u64::from_le_bytes(take(8)?.try_into().ok()?),
        ack_epoch_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
    };
    Some(ack)
}

/// Encode a feedback report.
pub fn encode_feedback(feedback: &MediaFeedback) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(FEEDBACK_MARKER);
    out.extend_from_slice(&feedback.rtt_ms.to_le_bytes());
    out.extend_from_slice(&feedback.rtt_min_ms.to_le_bytes());
    out.extend_from_slice(&feedback.jitter_ms.to_le_bytes());
    out.extend_from_slice(&feedback.received_bytes.to_le_bytes());
    out.extend_from_slice(&feedback.received_objects.to_le_bytes());
    out.extend_from_slice(&feedback.lost_objects.to_le_bytes());
    out.extend_from_slice(&feedback.duplicated_objects.to_le_bytes());
    out.extend_from_slice(&feedback.reordered_objects.to_le_bytes());
    out.extend_from_slice(&feedback.window_ms.to_le_bytes());
    out.push(u8::from(feedback.ack.is_some()));
    if let Some(ack) = &feedback.ack {
        let encoded = encode_ack(ack);
        out.extend_from_slice(&(encoded.len() as u16).to_le_bytes());
        out.extend_from_slice(&encoded);
    }
    out.extend_from_slice(&(feedback.probes.len() as u16).to_le_bytes());
    for probe in &feedback.probes {
        out.extend_from_slice(&probe.object_id.to_le_bytes());
        out.extend_from_slice(&probe.send_epoch_ms.to_le_bytes());
        out.extend_from_slice(&probe.echo_epoch_ms.to_le_bytes());
        out.extend_from_slice(&probe.payload_bytes.to_le_bytes());
    }
    out.extend_from_slice(&feedback.buffered_amount_bytes.to_le_bytes());
    out.extend_from_slice(&feedback.display_fps.to_le_bytes());
    out.extend_from_slice(&feedback.render_ms_p50.to_le_bytes());
    out.extend_from_slice(&feedback.render_ms_p95.to_le_bytes());
    out.extend_from_slice(&feedback.input_queue_depth.to_le_bytes());
    out.extend_from_slice(&feedback.input_submitted.to_le_bytes());
    out.extend_from_slice(&feedback.input_expired.to_le_bytes());
    out.extend_from_slice(&feedback.path_switches.to_le_bytes());
    out.push(feedback.active_path.as_u8());
    out
}

/// Decode a feedback report.
pub fn decode_feedback(payload: &[u8]) -> Option<MediaFeedback> {
    if payload.len() < FEEDBACK_MARKER.len() + 4 + 4 + 4 + 8 * 5 + 4 + 1
        || &payload[..FEEDBACK_MARKER.len()] != FEEDBACK_MARKER
    {
        return None;
    }
    let mut cursor = FEEDBACK_MARKER.len();
    let mut take = |count: usize| -> Option<Vec<u8>> {
        if cursor + count > payload.len() {
            return None;
        }
        let slice = payload[cursor..cursor + count].to_vec();
        cursor += count;
        Some(slice)
    };
    let mut feedback = MediaFeedback {
        rtt_ms: u32::from_le_bytes(take(4)?.try_into().ok()?),
        rtt_min_ms: u32::from_le_bytes(take(4)?.try_into().ok()?),
        jitter_ms: u32::from_le_bytes(take(4)?.try_into().ok()?),
        received_bytes: u64::from_le_bytes(take(8)?.try_into().ok()?),
        received_objects: u64::from_le_bytes(take(8)?.try_into().ok()?),
        lost_objects: u64::from_le_bytes(take(8)?.try_into().ok()?),
        duplicated_objects: u64::from_le_bytes(take(8)?.try_into().ok()?),
        reordered_objects: u64::from_le_bytes(take(8)?.try_into().ok()?),
        window_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
        ..MediaFeedback::default()
    };
    if take(1)?[0] != 0 {
        let length = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        let body = take(length)?;
        feedback.ack = decode_ack(&body);
    }
    let probe_count = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
    for _ in 0..probe_count {
        feedback.probes.push(ProbeEcho {
            object_id: u32::from_le_bytes(take(4)?.try_into().ok()?),
            send_epoch_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
            echo_epoch_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
            payload_bytes: u32::from_le_bytes(take(4)?.try_into().ok()?),
        });
    }
    feedback.buffered_amount_bytes = u32::from_le_bytes(take(4)?.try_into().ok()?);
    feedback.display_fps = u32::from_le_bytes(take(4)?.try_into().ok()?);
    feedback.render_ms_p50 = u32::from_le_bytes(take(4)?.try_into().ok()?);
    feedback.render_ms_p95 = u32::from_le_bytes(take(4)?.try_into().ok()?);
    feedback.input_queue_depth = u32::from_le_bytes(take(4)?.try_into().ok()?);
    feedback.input_submitted = u64::from_le_bytes(take(8)?.try_into().ok()?);
    feedback.input_expired = u64::from_le_bytes(take(8)?.try_into().ok()?);
    feedback.path_switches = u64::from_le_bytes(take(8)?.try_into().ok()?);
    feedback.active_path = mlp_core::media::MediaPath::from_u8(take(1)?[0]);
    Some(feedback)
}

/// Encode a probe echo.
pub fn encode_echo(echo: &ProbeEcho) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    out.extend_from_slice(ECHO_MARKER);
    out.extend_from_slice(&echo.object_id.to_le_bytes());
    out.extend_from_slice(&echo.send_epoch_ms.to_le_bytes());
    out.extend_from_slice(&echo.echo_epoch_ms.to_le_bytes());
    out.extend_from_slice(&echo.payload_bytes.to_le_bytes());
    out
}

/// Decode a probe echo.
pub fn decode_echo(payload: &[u8]) -> Option<ProbeEcho> {
    if payload.len() < ECHO_MARKER.len() + 4 + 8 + 8 + 4
        || &payload[..ECHO_MARKER.len()] != ECHO_MARKER
    {
        return None;
    }
    let mut cursor = ECHO_MARKER.len();
    let mut take = |count: usize| -> Option<Vec<u8>> {
        if cursor + count > payload.len() {
            return None;
        }
        let slice = payload[cursor..cursor + count].to_vec();
        cursor += count;
        Some(slice)
    };
    Some(ProbeEcho {
        object_id: u32::from_le_bytes(take(4)?.try_into().ok()?),
        send_epoch_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
        echo_epoch_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
        payload_bytes: u32::from_le_bytes(take(4)?.try_into().ok()?),
    })
}

/// Encode a probe.
pub fn encode_probe(probe: &mlp_core::media::MediaProbe) -> Vec<u8> {
    let mut out = Vec::with_capacity(21);
    out.extend_from_slice(PROBE_MARKER);
    out.extend_from_slice(&probe.probe_id.to_le_bytes());
    out.extend_from_slice(&probe.send_epoch_ms.to_le_bytes());
    out.extend_from_slice(&probe.payload_bytes.to_le_bytes());
    out
}

/// Decode a probe.
pub fn decode_probe(payload: &[u8]) -> Option<mlp_core::media::MediaProbe> {
    if payload.len() < PROBE_MARKER.len() + 4 + 8 + 4
        || &payload[..PROBE_MARKER.len()] != PROBE_MARKER
    {
        return None;
    }
    let mut cursor = PROBE_MARKER.len();
    let mut take = |count: usize| -> Option<Vec<u8>> {
        if cursor + count > payload.len() {
            return None;
        }
        let slice = payload[cursor..cursor + count].to_vec();
        cursor += count;
        Some(slice)
    };
    Some(mlp_core::media::MediaProbe {
        probe_id: u32::from_le_bytes(take(4)?.try_into().ok()?),
        send_epoch_ms: u64::from_le_bytes(take(8)?.try_into().ok()?),
        payload_bytes: u32::from_le_bytes(take(4)?.try_into().ok()?),
    })
}

/// Encode an input envelope.
pub fn encode_input(envelope: &MediaInputEnvelope) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + envelope.payload.len());
    out.extend_from_slice(INPUT_MARKER);
    out.extend_from_slice(&envelope.input_seq.to_le_bytes());
    out.extend_from_slice(&envelope.deadline_epoch_ms.to_le_bytes());
    out.push(envelope.kind.as_u32() as u8);
    out.push(envelope.flags.bits());
    out.extend_from_slice(&(envelope.payload.len() as u16).to_le_bytes());
    out.extend_from_slice(&envelope.payload);
    out
}

/// Decode an input envelope.
pub fn decode_input(payload: &[u8]) -> Option<MediaInputEnvelope> {
    if payload.len() < INPUT_MARKER.len() + 8 + 8 + 1 + 1 + 2
        || &payload[..INPUT_MARKER.len()] != INPUT_MARKER
    {
        return None;
    }
    let mut cursor = INPUT_MARKER.len();
    let mut take = |count: usize| -> Option<Vec<u8>> {
        if cursor + count > payload.len() {
            return None;
        }
        let slice = payload[cursor..cursor + count].to_vec();
        cursor += count;
        Some(slice)
    };
    let input_seq = u64::from_le_bytes(take(8)?.try_into().ok()?);
    let deadline_epoch_ms = u64::from_le_bytes(take(8)?.try_into().ok()?);
    let kind = InputKind::from_u32(u32::from(take(1)?[0]))?;
    let flags = InputFlags::from_bits(take(1)?[0]);
    let length = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
    let body = take(length)?;
    Some(MediaInputEnvelope::new(
        input_seq,
        deadline_epoch_ms,
        kind,
        flags,
        body,
    ))
}

/// The bytes the harness's media source carries per cell.
///
/// The protocol does not interpret an update's content (MLP spec §5.1), so
/// the harness's media source is free to choose its own cost per cell. It
/// chooses **256**: the protocol's scheduler budgets 512 bytes per cell as
/// the *upper bound* of any codec's cost, and the object cap the sender
/// enforces is at least the scheduler's per-update budget, so a source that
/// costs half the upper bound always fits inside one object.
pub const MEDIA_CELL_BYTES: u64 = 256;

/// The bytes a cell bitmap needs for a grid of `cells` cells.
pub const fn bitmap_bytes(cells: u32) -> usize {
    (cells as usize).div_ceil(8)
}

/// Encode a media payload: the cells it covers, then padding.
///
/// The transport does not interpret an update's content (MLP spec §5.1), so
/// the harness's media source is the smallest thing that says which cells
/// an update covers: a bitmap. The padding makes the payload the size the
/// scheduler estimated, so the byte counts the harness predicts are the
/// byte counts the protocol would carry.
pub fn encode_media_payload(
    cells: &[u32],
    total_cells: u32,
    total_bytes: usize,
    update_id: u64,
) -> Vec<u8> {
    let bitmap_len = bitmap_bytes(total_cells);
    let mut out = vec![0u8; bitmap_len.max(1)];
    for cell in cells {
        let index = *cell as usize;
        if index >= total_cells as usize {
            continue;
        }
        out[index / 8] |= 1 << (index % 8);
    }
    let padding = total_bytes.saturating_sub(out.len());
    let filler = (update_id & 0xff) as u8;
    out.resize(out.len() + padding, filler);
    out
}

/// Decode the cells a media payload covers.
pub fn decode_media_payload(payload: &[u8], total_cells: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for cell in 0..total_cells {
        let index = cell as usize;
        let byte = index / 8;
        if byte >= payload.len() {
            break;
        }
        if payload[byte] & (1 << (index % 8)) != 0 {
            out.push(cell);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_acknowledgement_survives_the_round_trip() {
        let ack = MediaAck {
            highest_applied_update_id: 42,
            ack_bitmap: 0b1010_1010,
            chain_epoch: 3,
            applied_cell_count: 510,
            stale_cells: 2,
            converged: true,
            render_latency_ms: 12.5,
            has_render_latency_ms: true,
            echo_of_input_seq: 9,
            ack_epoch_ms: 1_234,
        };
        let decoded = decode_ack(&encode_ack(&ack)).expect("a decoded acknowledgement");
        assert_eq!(decoded, ack);
    }

    #[test]
    fn a_feedback_report_survives_the_round_trip() {
        let feedback = MediaFeedback {
            rtt_ms: 150,
            rtt_min_ms: 120,
            jitter_ms: 20,
            received_bytes: 12_345,
            received_objects: 42,
            lost_objects: 3,
            duplicated_objects: 1,
            reordered_objects: 2,
            window_ms: 1_000,
            ack: Some(MediaAck {
                highest_applied_update_id: 7,
                ack_bitmap: 0b11,
                ..MediaAck::default()
            }),
            probes: vec![ProbeEcho {
                object_id: 99,
                send_epoch_ms: 1_000,
                echo_epoch_ms: 1_150,
                payload_bytes: 16,
            }],
            buffered_amount_bytes: 512,
            display_fps: 30,
            render_ms_p50: 8,
            render_ms_p95: 16,
            input_queue_depth: 2,
            input_submitted: 10,
            input_expired: 1,
            path_switches: 1,
            active_path: mlp_core::media::MediaPath::Ws,
            ..MediaFeedback::default()
        };
        let decoded = decode_feedback(&encode_feedback(&feedback)).expect("a decoded report");
        assert_eq!(decoded.rtt_ms, feedback.rtt_ms);
        assert_eq!(decoded.received_objects, feedback.received_objects);
        assert_eq!(decoded.ack, feedback.ack);
        assert_eq!(decoded.probes, feedback.probes);
        assert_eq!(decoded.active_path, feedback.active_path);
    }

    #[test]
    fn a_probe_echo_survives_the_round_trip() {
        let echo = ProbeEcho {
            object_id: 5,
            send_epoch_ms: 100,
            echo_epoch_ms: 250,
            payload_bytes: 8,
        };
        assert_eq!(decode_echo(&encode_echo(&echo)), Some(echo));
    }

    #[test]
    fn an_input_envelope_survives_the_round_trip() {
        let envelope = MediaInputEnvelope::new(
            17,
            5_000,
            InputKind::Key,
            InputFlags::CRITICAL,
            vec![1, 2, 3],
        );
        assert_eq!(decode_input(&encode_input(&envelope)), Some(envelope));
    }

    #[test]
    fn a_body_of_another_class_is_refused() {
        assert!(decode_ack(b"fb\x00\x00").is_none());
        assert!(decode_feedback(b"ack\x00\x00").is_none());
        assert!(decode_echo(b"nope").is_none());
    }

    #[test]
    fn a_media_payload_names_its_cells() {
        let cells = vec![0u32, 3, 9, 200];
        let payload = encode_media_payload(&cells, 512, 1_000, 7);
        assert_eq!(payload.len(), 1_000);
        assert_eq!(decode_media_payload(&payload, 512), cells);
    }

    #[test]
    fn a_media_payload_is_deterministic() {
        let first = encode_media_payload(&[1, 2], 64, 128, 3);
        let second = encode_media_payload(&[1, 2], 64, 128, 3);
        assert_eq!(first, second);
    }
}
