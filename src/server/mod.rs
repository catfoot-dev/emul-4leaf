pub mod packet_logger;
pub mod protocol;

use std::collections::HashSet;

use self::protocol::{ControlMessage, DNetPacket, ProtocolPacket};

/// DNet 프로토콜 핸들러를 인-프로세스 스레드로 실행합니다.
///
/// `server_rx`: 게스트(에뮬레이터) → 핸들러 (게스트가 send()한 데이터)
/// `server_tx`: 핸들러 → 게스트 (게스트가 recv()로 읽을 데이터)
///
/// 이 함수는 블로킹이며 `std::thread::spawn`으로 실행해야 합니다.
pub fn run_dnet_handler(
    server_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    server_tx: std::sync::mpsc::Sender<Vec<u8>>,
) {
    let mut byte_buf: Vec<u8> = Vec::new();
    let mut open_channels: HashSet<u16> = HashSet::new();

    loop {
        // 데이터가 올 때까지 블로킹 대기
        match server_rx.recv() {
            Ok(chunk) => byte_buf.extend(chunk),
            Err(_) => return, // 게스트 측 송신단(chan_tx)이 drop되면 종료
        }

        // 버퍼에서 완전한 DNet 프레임을 모두 처리
        loop {
            if byte_buf.len() < 4 {
                break; // 헤더 4바이트가 올 때까지 대기
            }

            let header: [u8; 4] = byte_buf[..4].try_into().unwrap();
            let (channel_id, body_len) = match DNetPacket::parse_header(&header) {
                Some(v) => v,
                None => {
                    crate::emu_socket_log!("[DNet] Invalid header: {}", hex::encode(&header));
                    return;
                }
            };

            let needed = 4 + body_len as usize;
            if byte_buf.len() < needed {
                break; // 본문이 모두 도착할 때까지 대기
            }

            // 헤더 + 본문을 버퍼에서 소비
            let frame: Vec<u8> = byte_buf.drain(..needed).collect();
            let body = &frame[4..];

            // 채널별 라우팅
            if channel_id == 0 {
                // 채널 0: 제어 메시지 [msg_type: u16 LE][target_channel_id: u16 LE]
                let ctrl = match ControlMessage::from_bytes(body) {
                    Some(c) => c,
                    None => {
                        crate::emu_socket_log!(
                            "[DNet] Malformed control body: {}",
                            hex::encode(body)
                        );
                        return;
                    }
                };

                crate::emu_socket_log!(
                    "[CTRL] msg={} target_ch={}",
                    ctrl.msg_type,
                    ctrl.channel_id
                );

                let responses = match ctrl.msg_type {
                    protocol::CTRL_OPEN => {
                        if !is_supported_data_channel(ctrl.channel_id) {
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} open request → unsupported, rejecting",
                                ctrl.channel_id
                            );
                            vec![protocol::create_control_message(
                                protocol::CTRL_REJECT_OR_ABORT,
                                ctrl.channel_id,
                            )]
                        } else if !open_channels.insert(ctrl.channel_id) {
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} open request → already open, rejecting",
                                ctrl.channel_id
                            );
                            vec![protocol::create_control_message(
                                protocol::CTRL_REJECT_OR_ABORT,
                                ctrl.channel_id,
                            )]
                        } else {
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} open request → accepting",
                                ctrl.channel_id
                            );
                            vec![protocol::create_control_message(
                                protocol::CTRL_OPEN_OK,
                                ctrl.channel_id,
                            )]
                        }
                    }
                    protocol::CTRL_OPEN_OK => {
                        crate::emu_socket_log!("[CTRL] Ch={} open acknowledged", ctrl.channel_id);
                        Vec::new()
                    }
                    protocol::CTRL_REJECT_OR_ABORT => {
                        let was_open = open_channels.remove(&ctrl.channel_id);
                        crate::emu_socket_log!(
                            "[CTRL] Ch={} reject/abort received (was_open={})",
                            ctrl.channel_id,
                            was_open
                        );
                        Vec::new()
                    }
                    protocol::CTRL_CLOSE => {
                        let was_open = open_channels.remove(&ctrl.channel_id);
                        if was_open {
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} close received → echoing close",
                                ctrl.channel_id
                            );
                            vec![protocol::create_control_message(
                                protocol::CTRL_CLOSE,
                                ctrl.channel_id,
                            )]
                        } else {
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} close received on unopened channel → ignoring",
                                ctrl.channel_id
                            );
                            Vec::new()
                        }
                    }
                    unknown => {
                        crate::emu_socket_log!(
                            "[CTRL] Unknown msg_type={} ch={}",
                            unknown,
                            ctrl.channel_id
                        );
                        Vec::new()
                    }
                };

                for pkt in responses {
                    if !send_wire(&server_tx, "CTRL", pkt) {
                        crate::emu_socket_log!(
                            "[DNet] Failed to send control response (guest disconnected)"
                        );
                        return;
                    }
                }
            } else {
                // 채널 1-15: 데이터 패킷 [main_type: u8][sub_type: u8][payload...]
                if !is_supported_data_channel(channel_id) || !open_channels.contains(&channel_id) {
                    crate::emu_socket_log!(
                        "[DNet] App packet on unopened/unsupported ch={} → sending REJECT",
                        channel_id
                    );
                    let reject = protocol::create_control_message(
                        protocol::CTRL_REJECT_OR_ABORT,
                        channel_id,
                    );
                    if !send_wire(&server_tx, "CTRL", reject) {
                        crate::emu_socket_log!(
                            "[DNet] Failed to send reject response (guest disconnected)"
                        );
                        return;
                    }
                } else if let Some(pkt) = ProtocolPacket::from_bytes(body) {
                    crate::emu_socket_log!(
                        "[RECV] ch={} main=0x{:02x} sub=0x{:02x} payload={}B {}",
                        channel_id,
                        pkt.main_type,
                        pkt.sub_type,
                        pkt.payload.len(),
                        hex::encode(&pkt.payload)
                    );
                    let resp = match pkt.main_type {
                        0x64 => handle_system(&pkt, channel_id),
                        0x0B => handle_chat_town_main(&pkt, channel_id),
                        0x0C => handle_chat_town_sub(&pkt, channel_id),
                        other => {
                            crate::emu_socket_log!("[WARN] 미구현 MainType=0x{:02x}", other);
                            None
                        }
                    };
                    if let Some(data) = resp {
                        if !send_wire(&server_tx, "APP", data) {
                            crate::emu_socket_log!(
                                "[DNet] Failed to send app response (guest disconnected)"
                            );
                            return;
                        }
                    }
                } else {
                    // 본문이 2바이트 미만이면 원본의 거절/중단 제어 메시지를 보냅니다.
                    crate::emu_socket_log!(
                        "[DNet] Malformed app body on ch={} → sending REJECT",
                        channel_id
                    );
                    let reject = protocol::create_control_message(
                        protocol::CTRL_REJECT_OR_ABORT,
                        channel_id,
                    );
                    if !send_wire(&server_tx, "CTRL", reject) {
                        crate::emu_socket_log!(
                            "[DNet] Failed to send reject response (guest disconnected)"
                        );
                        return;
                    }
                }
            }
        }
    }
}

// =========================================================
// MainType별 핸들러
// =========================================================

/// MainType 0x64 — 공통 시스템 메시지 (버전 핸드셰이크, 로그인 등)를 처리합니다.
fn handle_system(pkt: &ProtocolPacket, ch: u16) -> Option<Vec<u8>> {
    match pkt.sub_type {
        0x01 => {
            // 버전 확인 요청 → 프로토콜 버전 54로 응답
            crate::emu_socket_log!("[SYS] 버전 핸드셰이크 요청 → 버전 54 응답");
            let mut payload = Vec::new();
            payload.extend_from_slice(&protocol::write_u32(54));
            Some(protocol::create_app_packet(ch, 0x64, 0x01, &payload))
        }
        0x02 => {
            // 로그인 요청 → result=0 (성공) 응답
            crate::emu_socket_log!("[SYS] 로그인 요청 → 성공 응답");
            let mut payload = Vec::new();
            payload.extend_from_slice(&protocol::write_u32(0));
            Some(protocol::create_app_packet(ch, 0x64, 0x02, &payload))
        }
        sub => {
            crate::emu_socket_log!(
                "[SYS] 미구현 sub=0x{:02x} payload={}",
                sub,
                hex::encode(&pkt.payload)
            );
            None
        }
    }
}

/// MainType 0x0B — ChatTown Main (입장, 퇴장, 월드 로직)을 처리합니다.
fn handle_chat_town_main(pkt: &ProtocolPacket, _ch: u16) -> Option<Vec<u8>> {
    crate::emu_socket_log!(
        "[ChatTown Main] sub=0x{:02x} payload={}",
        pkt.sub_type,
        hex::encode(&pkt.payload)
    );
    // TODO: 패킷 로그 분석 후 sub_type별 응답 구현
    None
}

/// MainType 0x0C — ChatTown Sub (대화, 액션, 아이템 사용)을 처리합니다.
fn handle_chat_town_sub(pkt: &ProtocolPacket, _ch: u16) -> Option<Vec<u8>> {
    crate::emu_socket_log!(
        "[ChatTown Sub] sub=0x{:02x} payload={}",
        pkt.sub_type,
        hex::encode(&pkt.payload)
    );
    // TODO: 채팅 메시지 에코 등 구현 예정
    None
}

fn is_supported_data_channel(channel_id: u16) -> bool {
    (1..=14).contains(&channel_id)
}

fn send_wire(server_tx: &std::sync::mpsc::Sender<Vec<u8>>, label: &str, data: Vec<u8>) -> bool {
    crate::emu_socket_log!("[SEND] {}", protocol::hex_dump(label, &data));
    server_tx.send(data).is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

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
}
