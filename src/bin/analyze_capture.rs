use hex::FromHex;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

const DEFAULT_CAPTURE_DIR: &str = "docs/Capture";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    channel_id: u16,
    phase: String,
    main_type: u8,
    sub_type: u8,
}

#[derive(Debug, Clone)]
struct CandidateAggregate {
    count: usize,
    payload_lengths: BTreeMap<usize, usize>,
    first_payload_hex: String,
}

#[derive(Debug, Clone)]
struct CandidateRecord {
    channel_id: u16,
    phase: String,
    candidate_index: usize,
    main_type: u8,
    sub_type: u8,
    payload_len: usize,
    payload_hex: String,
}

#[derive(Debug, Clone)]
struct PacketRecord {
    direction: String,
    socket_id: u32,
    len: usize,
    data: Vec<u8>,
    mirrored_prev_send: bool,
    summary: String,
}

#[derive(Debug, Clone)]
struct FrameRecord {
    mirrored_prev_send: bool,
    summary: String,
}

#[derive(Debug, Clone)]
struct NoteRecord {
    channel_id: u16,
    phase: String,
    text: String,
}

fn parse_prefixed_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(key))
}

fn parse_candidate_record(line: &str) -> Option<CandidateRecord> {
    if !line.contains("candidate#") || !line.contains("requires server response:") {
        return None;
    }

    let channel_id = parse_prefixed_value(line, "ch=")?.parse().ok()?;
    let phase = parse_prefixed_value(line, "phase=")?.to_string();

    let candidate_index = line
        .split_whitespace()
        .find_map(|part| part.strip_prefix("candidate#"))?
        .parse()
        .ok()?;

    let marker = "requires server response:";
    let detail = line.split_once(marker)?.1.trim();
    let mut parts = detail.split_whitespace();

    let main_type = u8::from_str_radix(parts.next()?.strip_prefix("main=0x")?, 16).ok()?;
    let sub_type = u8::from_str_radix(parts.next()?.strip_prefix("sub=0x")?, 16).ok()?;
    let payload_len = parts
        .next()?
        .strip_prefix("payload=")?
        .strip_suffix('B')?
        .parse()
        .ok()?;
    let payload_hex = parts.collect::<Vec<_>>().join(" ");

    Some(CandidateRecord {
        channel_id,
        phase,
        candidate_index,
        main_type,
        sub_type,
        payload_len,
        payload_hex,
    })
}

fn parse_note_record(line: &str) -> Option<NoteRecord> {
    if line.contains("candidate#") {
        return None;
    }

    let channel_id = parse_prefixed_value(line, "ch=")?.parse().ok()?;
    let phase = parse_prefixed_value(line, "phase=")?.to_string();
    let text = line
        .split_whitespace()
        .skip(2)
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }

    Some(NoteRecord {
        channel_id,
        phase,
        text,
    })
}

fn parse_packet_record(line: &str) -> Option<PacketRecord> {
    let (prefix, summary) = line.split_once(" summary=")?;
    let direction = parse_prefixed_value(prefix, "dir=")?.to_string();
    let socket_id = parse_prefixed_value(prefix, "sock=")?.parse().ok()?;
    let len = parse_prefixed_value(prefix, "len=")?.parse().ok()?;
    let hex = parse_prefixed_value(prefix, "hex=")?;
    let mirrored_prev_send = parse_prefixed_value(prefix, "mirror_prev_send=")?
        .parse()
        .ok()?;
    let data = Vec::from_hex(hex).ok()?;

    Some(PacketRecord {
        direction,
        socket_id,
        len,
        data,
        mirrored_prev_send,
        summary: summary.to_string(),
    })
}

fn parse_frame_record(line: &str) -> Option<FrameRecord> {
    let (prefix, summary) = line.split_once(" summary=")?;
    let mirrored_prev_send = parse_prefixed_value(prefix, "mirror_prev_send=")?
        .parse()
        .ok()?;

    Some(FrameRecord {
        mirrored_prev_send,
        summary: summary.to_string(),
    })
}

fn collect_candidates(lines: &[String]) -> Vec<CandidateRecord> {
    lines
        .iter()
        .filter_map(|line| parse_candidate_record(line))
        .collect()
}

fn collect_packets(lines: &[String]) -> Vec<PacketRecord> {
    lines
        .iter()
        .filter_map(|line| parse_packet_record(line))
        .collect()
}

fn collect_frames(lines: &[String]) -> Vec<FrameRecord> {
    lines
        .iter()
        .filter_map(|line| parse_frame_record(line))
        .collect()
}

fn collect_notes(lines: &[String]) -> Vec<NoteRecord> {
    lines
        .iter()
        .filter_map(|line| parse_note_record(line))
        .collect()
}

fn summarize_candidates(
    candidates: &[CandidateRecord],
) -> BTreeMap<CandidateKey, CandidateAggregate> {
    let mut summary = BTreeMap::new();

    for candidate in candidates {
        let key = CandidateKey {
            channel_id: candidate.channel_id,
            phase: candidate.phase.clone(),
            main_type: candidate.main_type,
            sub_type: candidate.sub_type,
        };
        let entry = summary.entry(key).or_insert_with(|| CandidateAggregate {
            count: 0,
            payload_lengths: BTreeMap::new(),
            first_payload_hex: candidate.payload_hex.clone(),
        });
        entry.count += 1;
        *entry
            .payload_lengths
            .entry(candidate.payload_len)
            .or_insert(0) += 1;
    }

    summary
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn print_candidate_summary(candidates: &[CandidateRecord]) {
    let summary = summarize_candidates(candidates);
    println!(
        "Candidate server responses after initial handshake: {}",
        candidates.len()
    );
    for (key, aggregate) in summary {
        let payload_lengths = aggregate
            .payload_lengths
            .iter()
            .map(|(len, count)| format!("{}B x{}", len, count))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "ch={} phase={} main=0x{:02x} sub=0x{:02x} count={} payloads=[{}] first_payload={}",
            key.channel_id,
            key.phase,
            key.main_type,
            key.sub_type,
            aggregate.count,
            payload_lengths,
            aggregate.first_payload_hex
        );
    }

    if let Some(last) = candidates.last() {
        println!(
            "Last candidate observed: ch={} phase={} candidate#{} main=0x{:02x} sub=0x{:02x} payload={}B {}",
            last.channel_id,
            last.phase,
            last.candidate_index,
            last.main_type,
            last.sub_type,
            last.payload_len,
            last.payload_hex
        );
    }
}

fn print_packet_summary(packets: &[PacketRecord]) {
    let mirrored = packets
        .iter()
        .filter(|packet| packet.mirrored_prev_send)
        .count();
    println!("Captured packet lines: {}", packets.len());
    println!("Mirrored responses: {}", mirrored);

    let mut summary_counts: BTreeMap<String, usize> = BTreeMap::new();
    for packet in packets {
        *summary_counts.entry(packet.summary.clone()).or_insert(0) += 1;
    }

    for (summary, count) in summary_counts.into_iter().take(10) {
        println!("packet_summary count={} {}", count, summary);
    }
}

fn print_frame_summary(frames: &[FrameRecord]) {
    let mirrored = frames
        .iter()
        .filter(|frame| frame.mirrored_prev_send)
        .count();
    println!("Reassembled DNet frames: {}", frames.len());
    println!("Mirrored frame responses: {}", mirrored);

    let mut summary_counts: BTreeMap<String, usize> = BTreeMap::new();
    for frame in frames {
        *summary_counts.entry(frame.summary.clone()).or_insert(0) += 1;
    }

    for (summary, count) in summary_counts {
        println!("frame_summary count={} {}", count, summary);
    }
}

fn print_inferred_conclusions(frames: &[FrameRecord]) {
    let summaries = frames
        .iter()
        .map(|frame| frame.summary.as_str())
        .collect::<Vec<_>>();
    if summaries.len() == 3
        && summaries[0].starts_with("ctrl msg=2 ch=1")
        && summaries[1].starts_with("app ch=1 main=0xe0 sub=0x04")
        && summaries[2].starts_with("ctrl msg=4 ch=1")
    {
        println!(
            "inference bootstrap_response_insufficient=true reason=\"client closed channel 1 immediately after receiving the main-frame bootstrap response\""
        );
    }

    let has_stage_channel_open = summaries
        .iter()
        .any(|summary| summary.starts_with("ctrl msg=1 ch=2"))
        || summaries
            .iter()
            .any(|summary| summary.starts_with("ctrl msg=1 ch=3"))
        || summaries
            .iter()
            .any(|summary| summary.starts_with("ctrl msg=2 ch=2"))
        || summaries
            .iter()
            .any(|summary| summary.starts_with("ctrl msg=2 ch=3"));
    if has_stage_channel_open {
        println!("inference mainframe_stage_channels_detected=true channels=\"2 or 3\"");
    }

    let has_channel1_message8 = summaries.iter().any(|summary| {
        summary.starts_with("app ch=1 main=0x08 sub=0x00")
            || summary.starts_with("raw ch=1 msg=8")
            || summary.starts_with("raw ch=1 handler=0 msg=8")
    });
    if has_channel1_message8 {
        println!("inference mainframe_stage_open_followup_sent=true message_id=8");
    }

    let has_channel1_message9 = summaries.iter().any(|summary| {
        summary.starts_with("app ch=1 main=0x09 sub=0x00")
            || summary.starts_with("raw ch=1 msg=9")
            || summary.starts_with("raw ch=1 handler=0 msg=9")
    });
    if has_channel1_message9 {
        println!(
            "inference mainframe_stage_info_followup_sent=true message_id=9 provisional_stub=true"
        );
    }
}

fn derive_frames_from_packet_chunks(packets: &[PacketRecord]) -> Vec<FrameRecord> {
    let mut buffers: BTreeMap<(String, u32), Vec<u8>> = BTreeMap::new();
    let mut frames = Vec::new();

    for packet in packets {
        if packet.direction == "CLIENT" && packet.len == 1 && packet.summary == "raw/unparsed" {
            continue;
        }

        let key = (packet.direction.clone(), packet.socket_id);
        let buffer = buffers.entry(key).or_default();
        buffer.extend_from_slice(&packet.data);

        loop {
            if buffer.len() < 4 {
                break;
            }

            let Some(header) = buffer.get(..4).and_then(|bytes| bytes.try_into().ok()) else {
                break;
            };
            let Some((_channel_id, body_len)) = parse_dnet_header(header) else {
                buffer.drain(..1);
                continue;
            };
            let frame_len = 4 + body_len as usize;
            if buffer.len() < frame_len {
                break;
            }

            let frame: Vec<u8> = buffer.drain(..frame_len).collect();
            frames.push(FrameRecord {
                mirrored_prev_send: packet.mirrored_prev_send,
                summary: summarize_frame(&frame),
            });
        }
    }

    frames
}

fn parse_dnet_header(header: [u8; 4]) -> Option<(u16, u16)> {
    let channel_id = u16::from_le_bytes([header[0], header[1]]);
    let body_len = u16::from_le_bytes([header[2], header[3]]);
    if channel_id > 15 || body_len > 0x1ffc {
        return None;
    }
    if channel_id == 0 && body_len != 4 {
        return None;
    }
    Some((channel_id, body_len))
}

fn summarize_frame(frame: &[u8]) -> String {
    let Some(header) = frame.get(..4).and_then(|bytes| bytes.try_into().ok()) else {
        return "raw/unparsed".to_string();
    };
    let Some((channel_id, body_len)) = parse_dnet_header(header) else {
        return "raw/unparsed".to_string();
    };
    if frame.len() != 4 + body_len as usize {
        return "raw/unparsed".to_string();
    }

    let body = &frame[4..];
    if channel_id == 0 {
        if body.len() < 4 {
            return format!("ctrl malformed len={}", body.len());
        }
        let msg = u16::from_le_bytes([body[0], body[1]]);
        let target = u16::from_le_bytes([body[2], body[3]]);
        return format!("ctrl msg={} ch={}", msg, target);
    }
    if body.len() >= 8 && body[..4] == [0, 0, 0, 0] {
        let msg_id = u32::from_le_bytes(body[4..8].try_into().unwrap());
        return format!(
            "raw ch={} handler=0 msg={} payload={}B {}",
            channel_id,
            msg_id,
            body.len() - 8,
            hex::encode(&body[8..])
        );
    }
    if body.len() >= 4 && body[1] == 0 && body[2] == 0 && body[3] == 0 {
        let msg_id = u32::from_le_bytes(body[..4].try_into().unwrap());
        return format!(
            "raw ch={} msg={} payload={}B {}",
            channel_id,
            msg_id,
            body.len() - 4,
            hex::encode(&body[4..])
        );
    }
    if body.len() < 2 {
        return format!("app ch={} malformed len={}", channel_id, body.len());
    }

    format!(
        "app ch={} main=0x{:02x} sub=0x{:02x} payload={}B {}",
        channel_id,
        body[0],
        body[1],
        body.len() - 2,
        hex::encode(&body[2..])
    )
}

fn print_note_summary(notes: &[NoteRecord]) {
    println!("Protocol analysis notes: {}", notes.len());
    for note in notes {
        println!("ch={} phase={} {}", note.channel_id, note.phase, note.text);
    }
}

fn main() {
    let capture_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CAPTURE_DIR));

    let protocol_analysis_path = capture_dir.join("protocol_analysis.log");
    let packets_path = capture_dir.join("packets.log");
    let frames_path = capture_dir.join("frames.log");

    let analysis_lines = read_lines(&protocol_analysis_path);
    let packet_lines = read_lines(&packets_path);
    let frame_lines = read_lines(&frames_path);
    let candidates = collect_candidates(&analysis_lines);
    let packets = collect_packets(&packet_lines);
    let frames = collect_frames(&frame_lines);
    let notes = collect_notes(&analysis_lines);

    println!("Capture directory: {}", capture_dir.display());
    println!(
        "protocol_analysis.log lines={} packets.log lines={} frames.log lines={}",
        analysis_lines.len(),
        packet_lines.len(),
        frame_lines.len()
    );

    if candidates.is_empty() {
        println!("No post-handshake candidate responses found.");
    } else {
        print_candidate_summary(&candidates);
    }

    if notes.is_empty() {
        println!("No protocol analysis notes found.");
    } else {
        print_note_summary(&notes);
    }

    if packets.is_empty() {
        println!("No packet summary lines found.");
    } else {
        print_packet_summary(&packets);
    }

    if frames.is_empty() {
        let derived = derive_frames_from_packet_chunks(&packets);
        if derived.is_empty() {
            println!("No reassembled frame lines found.");
        } else {
            println!("No frames.log found; derived frames from packet chunks.");
            print_frame_summary(&derived);
            print_inferred_conclusions(&derived);
        }
    } else {
        print_frame_summary(&frames);
        print_inferred_conclusions(&frames);
    }
}
