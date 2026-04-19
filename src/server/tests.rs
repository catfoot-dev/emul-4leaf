use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::{
    analysis::{
        ChannelAnalysisState, ChannelPhase, is_post_initial_handshake_phase,
        raw_stage_packet_from_bytes, should_parse_as_raw_stage_packet,
        should_promote_open_to_mainframe_stage,
    },
    domain::{
        control::{
            build_main_frame_raw_message, build_provisional_main_frame_stage_info_response,
            build_provisional_worldmap_stage_bootstrap_response,
            build_provisional_worldmap_stage_payload,
        },
        echo::{handle_inventory, handle_main_type_d4},
    },
    protocol::{self, ChannelPacket, DNetPacket},
    run_dnet_handler,
    state::{SessionInfo, build_avatar_detail_data},
};

#[test]
fn handler_does_not_send_app_data_before_client_opens_a_channel() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    let timeout = from_handler_rx.recv_timeout(Duration::from_millis(100));
    assert!(timeout.is_err());

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn handler_acknowledges_open_without_sending_extra_app_data() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 1))
        .unwrap();

    let open_ok = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert_eq!(
        open_ok,
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 1)
    );
    let timeout = from_handler_rx.recv_timeout(Duration::from_millis(100));
    assert!(timeout.is_err());

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn handler_rejects_app_packets_on_unopened_channel() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_app_packet(2, 0x64, 0x01, &[]))
        .unwrap();

    let reject = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        reject,
        protocol::create_control_message(protocol::CTRL_REJECT_OR_ABORT, 2)
    );

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn main_frame_stage_info_response_uses_zeroed_sixteen_byte_stub() {
    let mut expected_body = Vec::new();
    expected_body.extend_from_slice(&0u32.to_le_bytes());
    expected_body.extend_from_slice(&9u32.to_le_bytes());
    expected_body.extend_from_slice(&[0u8; 16]);

    assert_eq!(
        build_provisional_main_frame_stage_info_response(2),
        DNetPacket::new(2, expected_body).to_bytes()
    );
}

#[test]
fn main_frame_raw_message_prefixes_zero_handler_pointer() {
    let wire = build_main_frame_raw_message(4, 6, &[0xaa, 0xbb]);
    let mut expected_body = Vec::new();
    expected_body.extend_from_slice(&0u32.to_le_bytes());
    expected_body.extend_from_slice(&6u32.to_le_bytes());
    expected_body.extend_from_slice(&[0xaa, 0xbb]);

    assert_eq!(wire, DNetPacket::new(4, expected_body).to_bytes());
}

#[test]
fn provisional_worldmap_stage_payload_is_sized_and_named() {
    let payload = build_provisional_worldmap_stage_payload();

    assert_eq!(payload.len(), 0x120);
    assert_eq!(&payload[..8], b"WorldMap");
    assert!(payload[8..].iter().all(|byte| *byte == 0));
}

#[test]
fn stage_channel_two_open_pushes_provisional_worldmap_bootstrap() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 1))
        .unwrap();
    assert_eq!(
        from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 1)
    );

    to_handler_tx
        .send(protocol::create_mainframe_packet(
            1,
            0x400d04e0,
            0,
            &[0x11, 0x00],
        ))
        .unwrap();
    let _bootstrap = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 2))
        .unwrap();

    assert_eq!(
        from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 2)
    );
    assert_eq!(
        from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        build_provisional_worldmap_stage_bootstrap_response(2)
    );

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn post_initial_handshake_detection_starts_after_bootstrap() {
    assert!(!is_post_initial_handshake_phase(ChannelPhase::OpenAccepted));
    assert!(is_post_initial_handshake_phase(
        ChannelPhase::BootstrapVersionSent
    ));
    assert!(is_post_initial_handshake_phase(
        ChannelPhase::AwaitingMainFrameStageInfo
    ));
    assert!(is_post_initial_handshake_phase(
        ChannelPhase::VersionNegotiated
    ));
    assert!(is_post_initial_handshake_phase(ChannelPhase::LoginAccepted));
}

#[test]
fn raw_stage_packet_parser_extracts_msg_id_and_payload() {
    let pkt =
        raw_stage_packet_from_bytes(&[0x09, 0x00, 0x00, 0x00, 0xaa, 0xbb, 0xcc, 0xdd]).unwrap();

    assert_eq!(pkt.msg_id, 9);
    assert_eq!(pkt.payload, vec![0xaa, 0xbb, 0xcc, 0xdd]);
}

#[test]
fn stage_channel_open_is_promoted_when_mainframe_is_waiting() {
    let mut states = HashMap::new();
    states.insert(
        1,
        ChannelAnalysisState {
            phase: ChannelPhase::AwaitingMainFrameStageInfo,
            post_bootstrap_client_packets: 0,
        },
    );

    assert!(should_promote_open_to_mainframe_stage(2, &states));
    assert!(!should_promote_open_to_mainframe_stage(4, &states));
}

#[test]
fn awaiting_stage_channels_use_raw_parser() {
    let mut states = HashMap::new();
    states.insert(
        2,
        ChannelAnalysisState {
            phase: ChannelPhase::AwaitingMainFrameStageInfo,
            post_bootstrap_client_packets: 0,
        },
    );

    assert!(should_parse_as_raw_stage_packet(
        2,
        &states,
        &[0x08, 0x00, 0x00, 0x00]
    ));
    assert!(!should_parse_as_raw_stage_packet(
        1,
        &states,
        &[0x08, 0x00, 0x00, 0x00]
    ));
}

#[test]
fn inventory_handler_returns_minimal_ack() {
    let pkt = ChannelPacket {
        main_type: 0x80,
        sub_type: 0x09,
        payload: vec![0xaa, 0xbb, 0xcc],
    };

    let outcome = handle_inventory(&pkt, 3);

    assert_eq!(outcome.responses.len(), 1);
    assert_eq!(
        outcome.responses[0],
        protocol::create_app_packet(3, 0x80, 0x09, &[0xaa, 0xbb, 0xcc])
    );
}

#[test]
fn main_type_d4_handler_returns_minimal_ack() {
    let pkt = ChannelPacket {
        main_type: 0xD4,
        sub_type: 0x02,
        payload: vec![0x10, 0x20, 0x30, 0x40],
    };

    let outcome = handle_main_type_d4(&pkt, 4);

    assert_eq!(outcome.responses.len(), 1);
    assert_eq!(
        outcome.responses[0],
        protocol::create_app_packet(4, 0xD4, 0x02, &[0x10, 0x20, 0x30, 0x40])
    );
}

#[test]
fn avatar_detail_data_has_expected_size() {
    let session = SessionInfo {
        user_id: "test".to_string(),
        nickname: b"TestUser".to_vec(),
        character: 1,
        gp: 1000,
        fp: 500,
    };
    let data = build_avatar_detail_data(&session);

    assert_eq!(data.len(), 272);
    assert_eq!(data[0], 1);
    assert_eq!(&data[1..9], b"TestUser");
}

#[test]
fn main_frame_dispatches_registration_request_on_control_three() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 1))
        .unwrap();
    let _open_ok = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    to_handler_tx
        .send(protocol::create_mainframe_packet(
            1,
            0x400d04e0,
            0,
            &[0x11, 0x00],
        ))
        .unwrap();
    let _bootstrap_resp = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    to_handler_tx
        .send(protocol::create_mainframe_packet(1, 0x400d04e0, 3, &[]))
        .unwrap();
    let reg_resp = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    let body = &reg_resp[4..];
    assert_eq!(
        u32::from_le_bytes(body[0..4].try_into().unwrap()),
        0x400d04e0
    );
    let control = u32::from_le_bytes(body[4..8].try_into().unwrap());
    assert_eq!(control, 0);

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn dnet_handler_dispatches_main_type_0x80() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 3))
        .unwrap();

    let open_ok = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        open_ok,
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 3)
    );

    let payload = [0x12, 0x34, 0x56];
    to_handler_tx
        .send(protocol::create_app_packet(3, 0x80, 0x01, &payload))
        .unwrap();

    let ack = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(ack, protocol::create_app_packet(3, 0x80, 0x01, &payload));

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn dnet_handler_dispatches_main_type_0xd4() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 4))
        .unwrap();

    let open_ok = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        open_ok,
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 4)
    );

    let payload = [0xab, 0xcd];
    to_handler_tx
        .send(protocol::create_app_packet(4, 0xD4, 0x01, &payload))
        .unwrap();

    let ack = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(ack, protocol::create_app_packet(4, 0xD4, 0x01, &payload));

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn dnet_handler_dispatches_main_type_0x70() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 5))
        .unwrap();

    let open_ok = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        open_ok,
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 5)
    );

    to_handler_tx
        .send(protocol::create_app_packet(5, 0x70, 0x03, &[0xaa, 0xbb]))
        .unwrap();

    let response = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let body = &response[4..];
    assert_eq!(body[0], 0x70);
    assert_eq!(body[1], 0x03);
    assert_eq!(u32::from_le_bytes(body[2..6].try_into().unwrap()), 1);

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn dnet_handler_dispatches_main_type_0xa4() {
    let (to_handler_tx, to_handler_rx) = mpsc::channel();
    let (from_handler_tx, from_handler_rx) = mpsc::channel();
    let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

    to_handler_tx
        .send(protocol::create_control_message(protocol::CTRL_OPEN, 5))
        .unwrap();

    let open_ok = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        open_ok,
        protocol::create_control_message(protocol::CTRL_OPEN_OK, 5)
    );

    to_handler_tx
        .send(protocol::create_app_packet(5, 0xA4, 0x01, &[0x00]))
        .unwrap();

    let response = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let body = &response[4..];
    assert_eq!(body[0], 0xA4);
    assert_eq!(body[1], 0x01);
    assert_eq!(u32::from_le_bytes(body[2..6].try_into().unwrap()), 1);

    drop(to_handler_tx);
    handle.join().unwrap();
}
