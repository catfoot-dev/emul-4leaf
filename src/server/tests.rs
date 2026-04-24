use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::{
    domain::world::{build_provisional_worldmap_stage_payload, build_worldmap_response},
    protocol, run_dnet_handler,
    session::{
        AVATAR_DIALOG_FLAG_ALLOW_CREATE, SessionInfo, build_avatar_detail_data,
        build_avatar_dialog_bootstrap_payload, build_avatar_dialog_record,
    },
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
        .send(protocol::create_auth_packet(
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
        build_worldmap_response(2)
    );

    drop(to_handler_tx);
    handle.join().unwrap();
}

#[test]
fn avatar_detail_data_has_expected_size() {
    let session = SessionInfo {
        user_id: "test".to_string(),
        nickname: b"TestUser".to_vec(),
        character: 1,
        gp: 1000,
        fp: 500,
        has_avatar: true,
        avatar_dialog_flags: 0,
    };
    let data = build_avatar_detail_data(&session);

    assert_eq!(data.len(), 272);
    assert_eq!(data[0], 1);
    assert_eq!(&data[1..9], b"TestUser");
}

#[test]
fn avatar_dialog_record_has_expected_size_and_nickname_slot() {
    let session = SessionInfo {
        user_id: "test".to_string(),
        nickname: b"TestUser".to_vec(),
        character: 1,
        gp: 1000,
        fp: 500,
        has_avatar: true,
        avatar_dialog_flags: 0,
    };

    let record = build_avatar_dialog_record(&session);

    assert_eq!(record.len(), 0x80);
    assert_eq!(record[0], 1);
    assert_eq!(&record[0x54..0x54 + 8], b"TestUser");
}

#[test]
fn avatar_dialog_bootstrap_payload_has_expected_layout() {
    let session = SessionInfo {
        user_id: "test".to_string(),
        nickname: b"TestUser".to_vec(),
        character: 1,
        gp: 1000,
        fp: 500,
        has_avatar: true,
        avatar_dialog_flags: 0,
    };

    let payload = build_avatar_dialog_bootstrap_payload(&session);

    assert_eq!(payload.len(), 0x431);
    assert_eq!(payload[0x39], 0);
    assert_eq!(payload[0x3a], 1);
    assert_eq!(
        &payload[0x39 + 0x7c + 0x54..0x39 + 0x7c + 0x54 + 8],
        b"TestUser"
    );
}

#[test]
fn avatar_dialog_bootstrap_payload_supports_empty_avatar_slots() {
    let session = SessionInfo {
        user_id: "newbie".to_string(),
        nickname: b"Newbie".to_vec(),
        character: 0,
        gp: 1000,
        fp: 0,
        has_avatar: false,
        avatar_dialog_flags: AVATAR_DIALOG_FLAG_ALLOW_CREATE,
    };

    let payload = build_avatar_dialog_bootstrap_payload(&session);

    assert_eq!(payload.len(), 0x431);
    assert_eq!(payload[0x38], AVATAR_DIALOG_FLAG_ALLOW_CREATE);
    assert_eq!(payload[0x39], 0);
    assert_eq!(payload[0x3a], 0);
    assert!(
        payload[0x39 + 0x7c..0x39 + 0x7c + 0x80]
            .iter()
            .all(|byte| *byte == 0)
    );
}

#[test]
fn login_response_streams_avatar_bootstrap_then_legacy_login_packet() {
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
        .send(protocol::create_auth_packet(1, 54, 1, b"test\0"))
        .unwrap();

    let response_stream = from_handler_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(response_stream.len() > 0x431);

    let first_len = u16::from_le_bytes([response_stream[2], response_stream[3]]) as usize;
    assert_eq!(
        u32::from_le_bytes(response_stream[4..8].try_into().unwrap()),
        54
    );
    assert_eq!(
        u32::from_le_bytes(response_stream[8..12].try_into().unwrap()),
        0
    );
    assert_eq!(first_len, 8 + 0x431);
    assert_eq!(response_stream[12 + 0x38], AVATAR_DIALOG_FLAG_ALLOW_CREATE);

    let second_offset = 4 + first_len;
    assert_eq!(
        u16::from_le_bytes(
            response_stream[second_offset..second_offset + 2]
                .try_into()
                .unwrap()
        ),
        1
    );
    let second_len = u16::from_le_bytes(
        response_stream[second_offset + 2..second_offset + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    assert_eq!(
        u32::from_le_bytes(
            response_stream[second_offset + 4..second_offset + 8]
                .try_into()
                .unwrap()
        ),
        54
    );
    assert_eq!(
        u32::from_le_bytes(
            response_stream[second_offset + 8..second_offset + 12]
                .try_into()
                .unwrap()
        ),
        0x1e
    );
    assert_eq!(second_len, 8 + 23);

    drop(to_handler_tx);
    handle.join().unwrap();
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
        .send(protocol::create_auth_packet(
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
        .send(protocol::create_auth_packet(1, 0x400d04e0, 3, &[]))
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
