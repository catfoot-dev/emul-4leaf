use std::{collections::HashSet, sync::mpsc};

use crate::server::{
    domain,
    protocol::{self, AuthPacket, ChannelPacket, ControlPacket, DNetPacket},
    session::Session,
};

/// 게스트 DLL과 호스트 사이에서 DNet 프레임을 중계하는 메인 핸들러 루프입니다.
pub fn run_dnet_handler(server_rx: mpsc::Receiver<Vec<u8>>, server_tx: mpsc::Sender<Vec<u8>>) {
    let mut runtime = ServerRuntime::new();

    loop {
        let chunk = match server_rx.recv() {
            Ok(chunk) => chunk,
            Err(_) => return,
        };

        runtime.byte_buf.extend(chunk);
        if !runtime.process_available_frames(&server_tx) {
            return;
        }
    }
}

/// 연결 단위의 채널/세션/분석 상태를 묶어서 관리합니다.
struct ServerRuntime {
    byte_buf: Vec<u8>,
    open_channels: HashSet<u16>,
    request: Session,
}

impl ServerRuntime {
    /// 새 연결에 대한 초기 상태를 생성합니다.
    fn new() -> Self {
        Self {
            byte_buf: Vec::new(),
            open_channels: HashSet::new(),
            request: Session::new(),
        }
    }

    /// 현재 버퍼에 쌓인 완전한 프레임을 모두 처리합니다.
    fn process_available_frames(&mut self, server_tx: &mpsc::Sender<Vec<u8>>) -> bool {
        loop {
            if self.byte_buf.len() < 4 {
                return true;
            }

            let header: [u8; 4] = self.byte_buf[..4].try_into().unwrap();
            let (channel_id, body_len) = match DNetPacket::parse_header(&header) {
                Some(value) => value,
                None => {
                    crate::emu_socket_log!("[DNet] Invalid header: {}", hex::encode(header));
                    return false;
                }
            };

            let needed = 4 + body_len as usize;
            if self.byte_buf.len() < needed {
                return true;
            }

            let frame: Vec<u8> = self.byte_buf.drain(..needed).collect();
            let body = &frame[4..];

            let mut hex = String::from(format!("[CLIENT] {:#04x}: ", 0));
            for (i, b) in frame.iter().enumerate() {
                if i > 0 && i % 32 == 0 {
                    hex.push_str(&format!("\n[CLIENT] {:#04x}: ", i));
                }
                hex.push_str(&format!("{:02x} ", b));
            }
            crate::emu_socket_log!("{}", hex);

            if channel_id == 0 {
                if !self.handle_control_message(server_tx, body) {
                    return false;
                }
            } else if !self.handle_app_message(server_tx, channel_id, body) {
                return false;
            }
        }
    }

    /// 제어 채널 메시지를 처리합니다.
    fn handle_control_message(&mut self, server_tx: &mpsc::Sender<Vec<u8>>, body: &[u8]) -> bool {
        let ctrl = match ControlPacket::from_bytes(body) {
            Some(ctrl) => ctrl,
            None => {
                crate::emu_socket_log!("[DNet] Malformed control body: {}", hex::encode(body));
                return false;
            }
        };

        crate::emu_socket_log!("[CTRL] msg={} target_ch={}", ctrl.msg_type, ctrl.channel_id);

        let responses = match ctrl.msg_type {
            protocol::CTRL_OPEN => self.handle_open_request(ctrl.channel_id),
            protocol::CTRL_OPEN_OK => {
                crate::emu_socket_log!("[CTRL] Ch={} open acknowledged", ctrl.channel_id);
                Vec::new()
            }
            protocol::CTRL_REJECT_OR_ABORT => {
                let was_open = self.open_channels.remove(&ctrl.channel_id);
                crate::emu_socket_log!(
                    "[CTRL] Ch={} reject/abort received (was_open={})",
                    ctrl.channel_id,
                    was_open
                );
                Vec::new()
            }
            protocol::CTRL_CLOSE => {
                let was_open = self.open_channels.remove(&ctrl.channel_id);

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

        self.send_packets(server_tx, "CTRL", responses)
    }

    /// 앱 채널 메시지를 해석하고 도메인 처리기로 전달합니다.
    fn handle_app_message(
        &mut self,
        server_tx: &mpsc::Sender<Vec<u8>>,
        channel_id: u16,
        body: &[u8],
    ) -> bool {
        if !is_supported_data_channel(channel_id) || !self.open_channels.contains(&channel_id) {
            crate::emu_socket_log!(
                "[DNet] App packet on unopened/unsupported ch={} → sending REJECT",
                channel_id
            );
            return self.send_packets(
                server_tx,
                "CTRL",
                vec![protocol::create_control_message(
                    protocol::CTRL_REJECT_OR_ABORT,
                    channel_id,
                )],
            );
        }

        if channel_id == 1 {
            let pkt = match AuthPacket::from_bytes(body) {
                Some(pkt) => pkt,
                None => {
                    crate::emu_socket_log!("[DNet] auth ch={} → sending REJECT", channel_id);
                    return self.send_packets(
                        server_tx,
                        "CTRL",
                        vec![protocol::create_control_message(
                            protocol::CTRL_REJECT_OR_ABORT,
                            channel_id,
                        )],
                    );
                }
            };

            crate::emu_socket_log!(
                "[CLIENT] ch={} code={:#x} control={} payload={}B {}",
                channel_id,
                pkt.code,
                pkt.control,
                pkt.payload.len(),
                hex::encode(&pkt.payload)
            );

            let responses = vec![domain::auth::handle_auth(
                &pkt,
                channel_id,
                &mut self.request,
            )];

            return self.send_packets(server_tx, "APP", responses);
        }

        let pkt = match ChannelPacket::from_bytes(body) {
            Some(pkt) => pkt,
            None => {
                crate::emu_socket_log!("[DNet] ch={} → sending REJECT", channel_id);
                return self.send_packets(
                    server_tx,
                    "CTRL",
                    vec![protocol::create_control_message(
                        protocol::CTRL_REJECT_OR_ABORT,
                        channel_id,
                    )],
                );
            }
        };

        crate::emu_socket_log!(
            "[CLIENT] ch={} main=0x{:02x} sub=0x{:02x} payload={}B {}",
            channel_id,
            pkt.main_type,
            pkt.sub_type,
            pkt.payload.len(),
            hex::encode(&pkt.payload)
        );

        let responses = vec![domain::dispatch_packet(&pkt, channel_id, &mut self.request)];

        self.send_packets(server_tx, "APP", responses)
    }

    /// 채널 open 요청에 대한 승인/거절과 기본 후속 응답을 결정합니다.
    fn handle_open_request(&mut self, channel_id: u16) -> Vec<Vec<u8>> {
        if !is_supported_data_channel(channel_id) {
            crate::emu_socket_log!(
                "[CTRL] Ch={} open request → unsupported, rejecting",
                channel_id
            );
            return vec![protocol::create_control_message(
                protocol::CTRL_REJECT_OR_ABORT,
                channel_id,
            )];
        }

        if !self.open_channels.insert(channel_id) {
            crate::emu_socket_log!(
                "[CTRL] Ch={} open request → already open, rejecting",
                channel_id
            );
            return vec![protocol::create_control_message(
                protocol::CTRL_REJECT_OR_ABORT,
                channel_id,
            )];
        }

        crate::emu_socket_log!("[CTRL] Ch={} open request → accepting", channel_id);

        build_stage_open_acceptance_responses(channel_id)
    }

    /// 생성된 응답 패킷들을 공통 로깅 포맷으로 전송합니다.
    fn send_packets(
        &self,
        server_tx: &mpsc::Sender<Vec<u8>>,
        label: &str,
        responses: Vec<Vec<u8>>,
    ) -> bool {
        for data in responses {
            // 응답 생성기가 빈 벡터를 돌려준 경우 실제 wire packet으로 보내지 않습니다.
            if data.is_empty() {
                continue;
            }

            if !send_wire(server_tx, data) {
                crate::emu_socket_log!(
                    "[DNet] Failed to send {} response (guest disconnected)",
                    label.to_lowercase()
                );
                return false;
            }
        }

        true
    }
}

/// 오픈 직후 채널별 기본 응답 시퀀스를 생성합니다.
fn build_stage_open_acceptance_responses(channel_id: u16) -> Vec<Vec<u8>> {
    let mut responses = vec![protocol::create_control_message(
        protocol::CTRL_OPEN_OK,
        channel_id,
    )];

    if channel_id == 2 {
        responses.push(domain::control::build_worldmap_response(channel_id));
    }

    responses
}

/// 서버가 처리하는 데이터 채널 범위를 판별합니다.
fn is_supported_data_channel(channel_id: u16) -> bool {
    (1..=14).contains(&channel_id)
}

/// 공통 송신 로그를 남기고 실제 채널로 바이트를 보냅니다.
fn send_wire(server_tx: &mpsc::Sender<Vec<u8>>, data: Vec<u8>) -> bool {
    server_tx.send(data).is_ok()
}
