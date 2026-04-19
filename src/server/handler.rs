use std::{
    collections::{HashMap, HashSet},
    sync::mpsc,
};

use crate::server::{
    analysis::{
        ChannelAnalysisState, ChannelPhase, emit_protocol_analysis,
        is_post_initial_handshake_phase, phase_label, raw_stage_packet_from_bytes,
        should_parse_as_raw_stage_packet, should_promote_open_to_mainframe_stage,
    },
    domain,
    protocol::{self, AuthPacket, ChannelPacket, ControlPacket, DNetPacket},
    state::GameState,
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
    analysis_states: HashMap<u16, ChannelAnalysisState>,
    game_state: GameState,
}

impl ServerRuntime {
    /// 새 연결에 대한 초기 상태를 생성합니다.
    fn new() -> Self {
        Self {
            byte_buf: Vec::new(),
            open_channels: HashSet::new(),
            analysis_states: HashMap::new(),
            game_state: GameState::new(),
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
        crate::emu_socket_log!("[SERVER] body: {}", hex::encode(body));

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
                self.emit_missing_bootstrap_response_hint(
                    ctrl.channel_id,
                    "client rejected/aborted immediately after bootstrap version response; response is likely insufficient",
                );
                self.analysis_states.remove(&ctrl.channel_id);
                crate::emu_socket_log!(
                    "[CTRL] Ch={} reject/abort received (was_open={})",
                    ctrl.channel_id,
                    was_open
                );
                Vec::new()
            }
            protocol::CTRL_CLOSE => {
                let was_open = self.open_channels.remove(&ctrl.channel_id);
                self.emit_missing_bootstrap_response_hint(
                    ctrl.channel_id,
                    "client closed channel immediately after bootstrap version response; response is likely insufficient",
                );
                self.analysis_states.remove(&ctrl.channel_id);

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

        if should_parse_as_raw_stage_packet(channel_id, &self.analysis_states, body) {
            return self.handle_raw_stage_message(channel_id, body);
        }

        if channel_id == 1 {
            let pkt = match AuthPacket::from_bytes(body) {
                Some(pkt) => pkt,
                None => {
                    crate::emu_socket_log!(
                        "[DNet] Malformed mainframe body on ch={} → sending REJECT",
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
            };

            crate::emu_socket_log!(
                "[SERVER] ch={} code={:#x} control={} payload={}B {}",
                channel_id,
                pkt.code,
                pkt.control,
                pkt.payload.len(),
                hex::encode(&pkt.payload)
            );

            let post_bootstrap_probe = self.record_post_bootstrap_probe(channel_id);
            let previous_phase = self
                .analysis_states
                .get(&channel_id)
                .map(|state| state.phase);
            let outcome = domain::auth::handle_auth(&pkt, channel_id, &mut self.game_state);
            let current_phase =
                self.apply_phase_update(channel_id, previous_phase, outcome.phase_update);

            if outcome.responses.is_empty()
                && post_bootstrap_probe.is_some()
                && is_post_initial_handshake_phase(current_phase)
            {
                emit_protocol_analysis(&format!(
                    "ch={} phase={} candidate#{} requires server response: code={:#x} control={} payload={}B {}",
                    channel_id,
                    phase_label(current_phase),
                    post_bootstrap_probe.unwrap_or(0),
                    pkt.code,
                    pkt.control,
                    pkt.payload.len(),
                    hex::encode(&pkt.payload)
                ));
            }

            return self.send_packets(server_tx, "APP", outcome.responses);
        }

        let pkt = match ChannelPacket::from_bytes(body) {
            Some(pkt) => pkt,
            None => {
                crate::emu_socket_log!(
                    "[DNet] Malformed app body on ch={} → sending REJECT",
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
        };

        crate::emu_socket_log!(
            "[SERVER] ch={} main=0x{:02x} sub=0x{:02x} payload={}B {}",
            channel_id,
            pkt.main_type,
            pkt.sub_type,
            pkt.payload.len(),
            hex::encode(&pkt.payload)
        );

        let post_bootstrap_probe = self.record_post_bootstrap_probe(channel_id);
        let previous_phase = self
            .analysis_states
            .get(&channel_id)
            .map(|state| state.phase);
        let outcome = domain::dispatch_packet(&pkt, channel_id, &mut self.game_state);
        let current_phase =
            self.apply_phase_update(channel_id, previous_phase, outcome.phase_update);

        if outcome.responses.is_empty()
            && post_bootstrap_probe.is_some()
            && is_post_initial_handshake_phase(current_phase)
        {
            emit_protocol_analysis(&format!(
                "ch={} phase={} candidate#{} requires server response: main=0x{:02x} sub=0x{:02x} payload={}B {}",
                channel_id,
                phase_label(current_phase),
                post_bootstrap_probe.unwrap_or(0),
                pkt.main_type,
                pkt.sub_type,
                pkt.payload.len(),
                hex::encode(&pkt.payload)
            ));
        }

        self.send_packets(server_tx, "APP", outcome.responses)
    }

    /// raw stage 메시지를 기록하고 분석용 힌트를 남깁니다.
    fn handle_raw_stage_message(&mut self, channel_id: u16, body: &[u8]) -> bool {
        let Some(pkt) = raw_stage_packet_from_bytes(body) else {
            crate::emu_socket_log!("[DNet] Raw stage body on ch={} malformed", channel_id);
            return true;
        };

        crate::emu_socket_log!(
            "[SERVER-RAW] ch={} msg={} payload={}B {}",
            channel_id,
            pkt.msg_id,
            pkt.payload.len(),
            hex::encode(&pkt.payload)
        );

        // let post_bootstrap_probe = self.record_post_bootstrap_probe(channel_id);
        // emit_protocol_analysis(&format!(
        //     "ch={} phase={} note=raw-stage msg={} payload={}B {}",
        //     channel_id,
        //     phase_label(ChannelPhase::AwaitingMainFrameStageInfo),
        //     pkt.msg_id,
        //     pkt.payload.len(),
        //     hex::encode(&pkt.payload)
        // ));

        // if let Some(candidate_index) = post_bootstrap_probe {
        //     emit_protocol_analysis(&format!(
        //         "ch={} phase={} candidate#{} requires server response: raw_msg={} payload={}B {}",
        //         channel_id,
        //         phase_label(ChannelPhase::AwaitingMainFrameStageInfo),
        //         candidate_index,
        //         pkt.msg_id,
        //         pkt.payload.len(),
        //         hex::encode(&pkt.payload)
        //     ));
        // }

        true
    }

    /// 채널 open 요청에 대한 승인/거절과 stage 승격 여부를 결정합니다.
    fn handle_open_request(&mut self, channel_id: u16) -> Vec<Vec<u8>> {
        let open_phase =
            if should_promote_open_to_mainframe_stage(channel_id, &self.analysis_states) {
                ChannelPhase::AwaitingMainFrameStageInfo
            } else {
                ChannelPhase::OpenAccepted
            };

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
        self.analysis_states.insert(
            channel_id,
            ChannelAnalysisState {
                phase: open_phase,
                post_bootstrap_client_packets: 0,
            },
        );

        // if open_phase == ChannelPhase::AwaitingMainFrameStageInfo {
        //     emit_protocol_analysis(&format!(
        //         "ch={} phase={} control-open accepted as likely main-frame stage channel; awaiting raw stage data",
        //         channel_id,
        //         phase_label(open_phase)
        //     ));
        // } else {
        //     emit_protocol_analysis(&format!(
        //         "ch={} phase={} control-open accepted; awaiting bootstrap packet",
        //         channel_id,
        //         phase_label(open_phase)
        //     ));
        // }

        build_stage_open_acceptance_responses(channel_id)
    }

    /// bootstrap 이후 들어온 후속 클라이언트 패킷 수를 채널별로 기록합니다.
    fn record_post_bootstrap_probe(&mut self, channel_id: u16) -> Option<usize> {
        let state = self.analysis_states.get_mut(&channel_id)?;
        if !is_post_initial_handshake_phase(state.phase) {
            return None;
        }

        state.post_bootstrap_client_packets += 1;
        Some(state.post_bootstrap_client_packets)
    }

    /// 도메인 처리기에서 반환한 phase 업데이트를 채널 상태에 반영합니다.
    fn apply_phase_update(
        &mut self,
        channel_id: u16,
        previous_phase: Option<ChannelPhase>,
        phase_update: Option<ChannelPhase>,
    ) -> ChannelPhase {
        if let Some(next_phase) = phase_update {
            let state = self
                .analysis_states
                .entry(channel_id)
                .or_insert(ChannelAnalysisState {
                    phase: next_phase,
                    post_bootstrap_client_packets: 0,
                });
            state.phase = next_phase;
            next_phase
        } else {
            previous_phase.unwrap_or(ChannelPhase::OpenAccepted)
        }
    }

    /// bootstrap 직후 채널이 닫히는 패턴을 분석 로그에 남깁니다.
    fn emit_missing_bootstrap_response_hint(&self, channel_id: u16, message: &str) {
        // if let Some(state) = self.analysis_states.get(&channel_id)
        //     && state.phase == ChannelPhase::BootstrapVersionSent
        //     && state.post_bootstrap_client_packets == 0
        // {
        //     emit_protocol_analysis(&format!(
        //         "ch={} phase={} {}",
        //         channel_id,
        //         phase_label(state.phase),
        //         message
        //     ));
        // }
    }

    /// 생성된 응답 패킷들을 공통 로깅 포맷으로 전송합니다.
    fn send_packets(
        &self,
        server_tx: &mpsc::Sender<Vec<u8>>,
        label: &str,
        responses: Vec<Vec<u8>>,
    ) -> bool {
        for data in responses {
            if !send_wire(server_tx, label, data) {
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
        responses
            .push(domain::control::build_provisional_worldmap_stage_bootstrap_response(channel_id));
    }

    responses
}

/// 서버가 처리하는 데이터 채널 범위를 판별합니다.
fn is_supported_data_channel(channel_id: u16) -> bool {
    (1..=14).contains(&channel_id)
}

/// 공통 송신 로그를 남기고 실제 채널로 바이트를 보냅니다.
fn send_wire(server_tx: &mpsc::Sender<Vec<u8>>, label: &str, data: Vec<u8>) -> bool {
    // crate::emu_socket_log!("[SERVER] {}", protocol::hex_dump(label, &data));
    server_tx.send(data).is_ok()
}
