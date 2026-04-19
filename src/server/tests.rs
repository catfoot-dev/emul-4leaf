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
        auth::handle_terms_dialog_request,
        echo::{handle_inventory, handle_main_type_d4},
        main_frame::{
            build_agent_response, build_main_frame_raw_message,
            build_provisional_main_frame_bootstrap_response,
            build_provisional_main_frame_stage_info_response,
            build_provisional_worldmap_stage_bootstrap_response,
            build_provisional_worldmap_stage_payload, get_news_title_text, handle_avatar_selection,
            handle_id_check, handle_registration_request, handle_registration_submit,
        },
    },
    protocol::{self, DNetPacket, MainFramePacket, ProtocolPacket},
    run_dnet_handler,
    state::{GameState, SessionInfo, build_avatar_detail_data},
};

fn sample_mainframe_packet(code: u32, control: u32, payload: &[u8]) -> MainFramePacket {
    MainFramePacket {
        code,
        control,
        payload: payload.to_vec(),
    }
}

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
fn handler_returns_version_based_main_frame_bootstrap_packet() {
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

    to_handler_tx
        .send(protocol::create_mainframe_packet(
            1,
            0x400d04e0,
            0,
            &[0x11, 0x00],
        ))
        .unwrap();

    let version_resp = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let mut expected_body = Vec::new();
    expected_body.extend_from_slice(&0u32.to_le_bytes());
    expected_body.extend_from_slice(&0u32.to_le_bytes());
    expected_body.extend_from_slice(&protocol::write_u16(54));
    expected_body.extend_from_slice(get_news_title_text());
    assert_eq!(version_resp, DNetPacket::new(1, expected_body).to_bytes());

    let timeout = from_handler_rx.recv_timeout(Duration::from_millis(100));
    assert!(timeout.is_err());

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn main_frame_bootstrap_response_uses_version_file_payload() {
    let response = build_provisional_main_frame_bootstrap_response(3);
    let mut expected_body = Vec::new();
    expected_body.extend_from_slice(&0u32.to_le_bytes());
    expected_body.extend_from_slice(&0u32.to_le_bytes());
    expected_body.extend_from_slice(&protocol::write_u16(54));
    expected_body.extend_from_slice(get_news_title_text());

    assert_eq!(response, DNetPacket::new(3, expected_body).to_bytes());
}

#[test]
fn main_frame_bootstrap_response_terminates_followup_text_block() {
    let response = build_provisional_main_frame_bootstrap_response(1);
    let (channel_id, body_len) =
        DNetPacket::parse_header(response[..4].try_into().unwrap()).unwrap();
    assert_eq!(channel_id, 1);
    assert_eq!(body_len as usize, response.len() - 4);
    assert_eq!(&response[4..8], &0u32.to_le_bytes());
    assert_eq!(&response[8..12], &0u32.to_le_bytes());
    assert_eq!(&response[12..14], &54u16.to_le_bytes());
    assert_eq!(&response[14..], get_news_title_text());
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
fn build_agent_response_produces_correct_wire_format() {
    let wire = build_agent_response(1, 0x400d04e0, 3, &[0xaa]);
    let header = &wire[..4];
    assert_eq!(header[0..2], [0x01, 0x00]);
    let body = &wire[4..];
    assert_eq!(body[0..4], [0xe0, 0x04, 0x0d, 0x40]);
    assert_eq!(body[4..8], [0x03, 0x00, 0x00, 0x00]);
    assert_eq!(body[8], 0xaa);
}

#[test]
fn registration_request_returns_join_message() {
    let state = GameState::new();
    let outcome = handle_registration_request(1, &state);
    assert_eq!(outcome.responses.len(), 1);

    let resp = &outcome.responses[0];
    let body = &resp[4..];
    let control = u32::from_le_bytes(body[4..8].try_into().unwrap());
    assert_eq!(control, 0);
}

#[test]
fn inventory_handler_returns_minimal_ack() {
    let pkt = ProtocolPacket {
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
fn legacy_main_type_a4_keeps_terms_dialog_payload_shape() {
    let pkt = ProtocolPacket {
        main_type: 0xA4,
        sub_type: 0x01,
        payload: vec![0x00],
    };

    let outcome = handle_terms_dialog_request(&pkt, 5, 0xA4);

    assert_eq!(outcome.responses.len(), 1);
    let wire = &outcome.responses[0];
    assert_eq!(&wire[0..2], &[0x05, 0x00]);
    let body = &wire[4..];
    assert_eq!(body[0], 0xA4);
    assert_eq!(body[1], 0x01);
    assert_eq!(u32::from_le_bytes(body[2..6].try_into().unwrap()), 1);
    assert!(
        body.windows(b"4Leaf Terms Agreement".len())
            .any(|w| w == b"4Leaf Terms Agreement")
    );
}

#[test]
fn main_type_0x70_keeps_terms_dialog_payload_shape() {
    let pkt = ProtocolPacket {
        main_type: 0x70,
        sub_type: 0x03,
        payload: vec![0xaa, 0xbb],
    };

    let outcome = handle_terms_dialog_request(&pkt, 5, 0x70);

    assert_eq!(outcome.responses.len(), 1);
    let wire = &outcome.responses[0];
    assert_eq!(&wire[0..2], &[0x05, 0x00]);
    let body = &wire[4..];
    assert_eq!(body[0], 0x70);
    assert_eq!(body[1], 0x03);
    assert_eq!(u32::from_le_bytes(body[2..6].try_into().unwrap()), 1);
    assert!(
        body.windows(b"4Leaf Terms Agreement".len())
            .any(|w| w == b"4Leaf Terms Agreement")
    );
}

#[test]
fn main_type_d4_handler_returns_minimal_ack() {
    let pkt = ProtocolPacket {
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
fn id_check_reports_available_for_new_id() {
    let state = GameState::new();
    let pkt = sample_mainframe_packet(0x400d04e0, 4, b"newuser\0");
    let outcome = handle_id_check(&pkt, 1, &state);
    assert_eq!(outcome.responses.len(), 1);

    let resp = &outcome.responses[0];
    let body = &resp[4..];
    let result = u32::from_le_bytes(body[8..12].try_into().unwrap());
    assert_eq!(result, 12);
}

#[test]
fn id_check_reports_taken_for_existing_id() {
    let state = GameState::new();
    let pkt = sample_mainframe_packet(0x400d04e0, 4, b"test\0");
    let outcome = handle_id_check(&pkt, 1, &state);
    let resp = &outcome.responses[0];
    let body = &resp[4..];
    let result = u32::from_le_bytes(body[8..12].try_into().unwrap());
    assert_eq!(result, 0);
}

#[test]
fn registration_submit_creates_new_user() {
    let mut state = GameState::new();
    let mut payload = Vec::new();
    let mut id_field = [0u8; 16];
    id_field[..5].copy_from_slice(b"hello");
    payload.extend_from_slice(&id_field);
    payload.extend_from_slice(&[0u8; 20]);
    let mut pass_field = [0u8; 16];
    pass_field[..5].copy_from_slice(b"world");
    payload.extend_from_slice(&pass_field);
    let pkt = sample_mainframe_packet(0x400d04e0, 5, &payload);
    let outcome = handle_registration_submit(&pkt, 1, &mut state);
    assert!(!outcome.responses.is_empty());
    assert!(state.users.contains_key("hello"));
    assert_eq!(
        state
            .session
            .as_ref()
            .map(|session| session.user_id.as_str()),
        Some("hello")
    );
}

#[test]
fn registration_submit_rejects_duplicate_user() {
    let mut state = GameState::new();
    let original_password = state.users.get("test").unwrap().password.clone();
    let mut payload = Vec::new();
    let mut id_field = [0u8; 16];
    id_field[..4].copy_from_slice(b"test");
    payload.extend_from_slice(&id_field);
    payload.extend_from_slice(&[0u8; 20]);
    let mut pass_field = [0u8; 16];
    pass_field[..3].copy_from_slice(b"new");
    payload.extend_from_slice(&pass_field);
    let pkt = sample_mainframe_packet(0x400d04e0, 5, &payload);

    let outcome = handle_registration_submit(&pkt, 1, &mut state);
    let body = &outcome.responses[0][4..];
    let control = u32::from_le_bytes(body[4..8].try_into().unwrap());

    assert_eq!(control, 0);
    assert_eq!(state.users.get("test").unwrap().password, original_password);
    assert!(state.session.is_none());
}

#[test]
fn registration_submit_prepares_avatar_selection_session() {
    let mut state = GameState::new();
    let mut payload = Vec::new();
    let mut id_field = [0u8; 16];
    id_field[..5].copy_from_slice(b"fresh");
    payload.extend_from_slice(&id_field);
    payload.extend_from_slice(&[0u8; 20]);
    let mut pass_field = [0u8; 16];
    pass_field[..2].copy_from_slice(b"pw");
    payload.extend_from_slice(&pass_field);
    let pkt = sample_mainframe_packet(0x400d04e0, 5, &payload);

    let _ = handle_registration_submit(&pkt, 1, &mut state);

    let avatar_pkt = sample_mainframe_packet(0x400d04e0, 7, &[0x02]);
    let outcome = handle_avatar_selection(&avatar_pkt, 1, &mut state);

    assert_eq!(outcome.responses.len(), 2);
    let detail_body = &outcome.responses[0][4..];
    let detail_control = u32::from_le_bytes(detail_body[4..8].try_into().unwrap());
    assert_eq!(detail_control, 0);
    assert_eq!(
        state.session.as_ref().map(|session| session.character),
        Some(2)
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
fn avatar_selection_sends_detail_and_visit_reward() {
    let mut state = GameState::new();
    state.session = Some(SessionInfo {
        user_id: "test".to_string(),
        nickname: b"Tester".to_vec(),
        character: 0,
        gp: 1000,
        fp: 0,
    });
    let pkt = sample_mainframe_packet(0x400d04e0, 7, &[0x01]);
    let outcome = handle_avatar_selection(&pkt, 1, &mut state);

    assert_eq!(outcome.responses.len(), 2);
    assert_eq!(state.session.as_ref().unwrap().character, 1);
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
