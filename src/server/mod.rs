pub mod packet_logger;
pub mod protocol;

use std::{
    collections::{HashMap, HashSet},
    fs,
};

use self::protocol::{ControlMessage, DNetPacket, ProtocolPacket};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ChannelPhase {
    OpenAccepted,
    BootstrapVersionSent,
    AwaitingMainFrameStageInfo,
    VersionNegotiated,
    LoginAccepted,
}

#[derive(Debug, Clone, Copy)]
struct ChannelAnalysisState {
    phase: ChannelPhase,
    post_bootstrap_client_packets: usize,
}

#[derive(Debug)]
struct HandlerOutcome {
    responses: Vec<Vec<u8>>,
    phase_update: Option<ChannelPhase>,
    analysis_note: Option<String>,
}

#[derive(Debug, Clone)]
struct RawStagePacket {
    msg_id: u32,
    payload: Vec<u8>,
}

fn phase_label(phase: ChannelPhase) -> &'static str {
    match phase {
        ChannelPhase::OpenAccepted => "open-accepted",
        ChannelPhase::BootstrapVersionSent => "bootstrap-version-sent",
        ChannelPhase::AwaitingMainFrameStageInfo => "awaiting-mainframe-stage-info",
        ChannelPhase::VersionNegotiated => "version-negotiated",
        ChannelPhase::LoginAccepted => "login-accepted",
    }
}

fn is_post_initial_handshake_phase(phase: ChannelPhase) -> bool {
    phase >= ChannelPhase::BootstrapVersionSent
}

fn emit_protocol_analysis(line: &str) {
    crate::emu_socket_log!("[ANALYZE] {}", line);
    crate::append_capture_line("protocol_analysis.log", line);
}

fn raw_stage_packet_from_bytes(data: &[u8]) -> Option<RawStagePacket> {
    if data.len() < 4 {
        return None;
    }

    Some(RawStagePacket {
        msg_id: u32::from_le_bytes(data[..4].try_into().ok()?),
        payload: data[4..].to_vec(),
    })
}

fn is_stage_channel(channel_id: u16) -> bool {
    matches!(channel_id, 2 | 3)
}

fn should_promote_open_to_mainframe_stage(
    channel_id: u16,
    analysis_states: &HashMap<u16, ChannelAnalysisState>,
) -> bool {
    is_stage_channel(channel_id)
        && analysis_states.values().any(|state| {
            matches!(
                state.phase,
                ChannelPhase::BootstrapVersionSent | ChannelPhase::AwaitingMainFrameStageInfo
            )
        })
}

fn should_parse_as_raw_stage_packet(
    channel_id: u16,
    analysis_states: &HashMap<u16, ChannelAnalysisState>,
    body: &[u8],
) -> bool {
    if body.len() < 4 {
        return false;
    }

    analysis_states
        .get(&channel_id)
        .map(|state| state.phase == ChannelPhase::AwaitingMainFrameStageInfo)
        .unwrap_or(false)
        && is_stage_channel(channel_id)
}

fn build_stage_open_acceptance_responses(channel_id: u16) -> Vec<Vec<u8>> {
    let mut responses = vec![protocol::create_control_message(
        protocol::CTRL_OPEN_OK,
        channel_id,
    )];
    if channel_id == 3 {
        responses.push(build_provisional_worldmap_stage_bootstrap_response(
            channel_id,
        ));
    }
    responses
}

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
    crate::append_capture_line("socket.log", "[DNet] handler thread started");
    let mut byte_buf: Vec<u8> = Vec::new();
    let mut open_channels: HashSet<u16> = HashSet::new();
    let mut analysis_states: HashMap<u16, ChannelAnalysisState> = HashMap::new();

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
                        let open_phase = if should_promote_open_to_mainframe_stage(
                            ctrl.channel_id,
                            &analysis_states,
                        ) {
                            ChannelPhase::AwaitingMainFrameStageInfo
                        } else {
                            ChannelPhase::OpenAccepted
                        };
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
                            analysis_states.insert(
                                ctrl.channel_id,
                                ChannelAnalysisState {
                                    phase: open_phase,
                                    post_bootstrap_client_packets: 0,
                                },
                            );
                            if open_phase == ChannelPhase::AwaitingMainFrameStageInfo {
                                emit_protocol_analysis(&format!(
                                    "ch={} phase={} control-open accepted as likely main-frame stage channel; awaiting raw stage data",
                                    ctrl.channel_id,
                                    phase_label(open_phase)
                                ));
                            } else {
                                emit_protocol_analysis(&format!(
                                    "ch={} phase={} control-open accepted; awaiting bootstrap packet",
                                    ctrl.channel_id,
                                    phase_label(open_phase)
                                ));
                            }
                            build_stage_open_acceptance_responses(ctrl.channel_id)
                        }
                    }
                    protocol::CTRL_OPEN_OK => {
                        crate::emu_socket_log!("[CTRL] Ch={} open acknowledged", ctrl.channel_id);
                        Vec::new()
                    }
                    protocol::CTRL_REJECT_OR_ABORT => {
                        let was_open = open_channels.remove(&ctrl.channel_id);
                        if let Some(state) = analysis_states.get(&ctrl.channel_id)
                            && state.phase == ChannelPhase::BootstrapVersionSent
                            && state.post_bootstrap_client_packets == 0
                        {
                            emit_protocol_analysis(&format!(
                                "ch={} phase={} client rejected/aborted immediately after bootstrap version response; response is likely insufficient",
                                ctrl.channel_id,
                                phase_label(state.phase)
                            ));
                        }
                        analysis_states.remove(&ctrl.channel_id);
                        crate::emu_socket_log!(
                            "[CTRL] Ch={} reject/abort received (was_open={})",
                            ctrl.channel_id,
                            was_open
                        );
                        Vec::new()
                    }
                    protocol::CTRL_CLOSE => {
                        let was_open = open_channels.remove(&ctrl.channel_id);
                        if let Some(state) = analysis_states.get(&ctrl.channel_id)
                            && state.phase == ChannelPhase::BootstrapVersionSent
                            && state.post_bootstrap_client_packets == 0
                        {
                            emit_protocol_analysis(&format!(
                                "ch={} phase={} client closed channel immediately after bootstrap version response; response is likely insufficient",
                                ctrl.channel_id,
                                phase_label(state.phase)
                            ));
                        }
                        analysis_states.remove(&ctrl.channel_id);
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
                } else if should_parse_as_raw_stage_packet(channel_id, &analysis_states, body) {
                    if let Some(pkt) = raw_stage_packet_from_bytes(body) {
                        crate::emu_socket_log!(
                            "[RECV-RAW] ch={} msg={} payload={}B {}",
                            channel_id,
                            pkt.msg_id,
                            pkt.payload.len(),
                            hex::encode(&pkt.payload)
                        );

                        let post_bootstrap_probe =
                            if let Some(state) = analysis_states.get_mut(&channel_id) {
                                if is_post_initial_handshake_phase(state.phase) {
                                    state.post_bootstrap_client_packets += 1;
                                    Some(state.post_bootstrap_client_packets)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                        emit_protocol_analysis(&format!(
                            "ch={} phase={} note=raw-stage msg={} payload={}B {}",
                            channel_id,
                            phase_label(ChannelPhase::AwaitingMainFrameStageInfo),
                            pkt.msg_id,
                            pkt.payload.len(),
                            hex::encode(&pkt.payload)
                        ));

                        if let Some(candidate_index) = post_bootstrap_probe {
                            emit_protocol_analysis(&format!(
                                "ch={} phase={} candidate#{} requires server response: raw_msg={} payload={}B {}",
                                channel_id,
                                phase_label(ChannelPhase::AwaitingMainFrameStageInfo),
                                candidate_index,
                                pkt.msg_id,
                                pkt.payload.len(),
                                hex::encode(&pkt.payload)
                            ));
                        }
                    } else {
                        crate::emu_socket_log!(
                            "[DNet] Raw stage body on ch={} malformed",
                            channel_id
                        );
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
                    let post_bootstrap_probe =
                        if let Some(state) = analysis_states.get_mut(&channel_id) {
                            if is_post_initial_handshake_phase(state.phase) {
                                state.post_bootstrap_client_packets += 1;
                                Some(state.post_bootstrap_client_packets)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                    let previous_phase = analysis_states.get(&channel_id).map(|state| state.phase);
                    let outcome = match pkt.main_type {
                        0xE0 => handle_main_frame(&pkt, channel_id),
                        0x64 => handle_system(&pkt, channel_id),
                        0x0B => handle_chat_town_main(&pkt, channel_id),
                        0x0C => handle_chat_town_sub(&pkt, channel_id),
                        other => {
                            crate::emu_socket_log!("[WARN] 미구현 MainType=0x{:02x}", other);
                            HandlerOutcome {
                                responses: Vec::new(),
                                phase_update: None,
                                analysis_note: Some(format!(
                                    "main=0x{:02x} sub=0x{:02x} payload={}B {}",
                                    other,
                                    pkt.sub_type,
                                    pkt.payload.len(),
                                    hex::encode(&pkt.payload)
                                )),
                            }
                        }
                    };

                    let current_phase = if let Some(next_phase) = outcome.phase_update {
                        let state =
                            analysis_states
                                .entry(channel_id)
                                .or_insert(ChannelAnalysisState {
                                    phase: next_phase,
                                    post_bootstrap_client_packets: 0,
                                });
                        state.phase = next_phase;
                        next_phase
                    } else {
                        previous_phase.unwrap_or(ChannelPhase::OpenAccepted)
                    };

                    if let Some(note) = &outcome.analysis_note {
                        emit_protocol_analysis(&format!(
                            "ch={} phase={} note={}",
                            channel_id,
                            phase_label(current_phase),
                            note
                        ));
                    }

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

                    for data in outcome.responses {
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

/// MainType 0xE0 — 메인 프레임 초기 채널 메시지를 임시 처리합니다.
fn handle_main_frame(pkt: &ProtocolPacket, ch: u16) -> HandlerOutcome {
    match pkt.sub_type {
        0x04 => {
            // 정적 분석상 `DMainFrame::OnReceived`는 `Version.dat`를 읽어 서버 값과 비교합니다.
            // 최신 캡처에서는 버전 응답만 보내면 곧바로 채널 1을 닫습니다.
            // 반면 이전 런타임 로그에는 bootstrap 직후 커스텀 메시지 4/8/6이
            // 순서대로 올라간 흔적이 있어, 현재는 그중 가장 보수적인 채널 1
            // compatibility sequence `msg=4 -> msg=0(version) -> msg=6`를 시도합니다.
            let prelude = build_main_frame_status_message_response(ch, 4);
            let response = build_provisional_main_frame_bootstrap_response(pkt, ch);
            let epilogue = build_main_frame_status_message_response(ch, 6);
            HandlerOutcome {
                responses: vec![prelude, response, epilogue],
                phase_update: Some(ChannelPhase::BootstrapVersionSent),
                analysis_note: Some(format!(
                    "bootstrap responded with compatibility sequence msg=4 -> msg=0(version={}) -> msg=6; deferred msg=8/msg=9 until real client stage-channel opens are observed",
                    read_local_package_version()
                )),
            }
        }
        sub => {
            crate::emu_socket_log!(
                "[MainFrame] 미구현 sub=0x{:02x} payload={}",
                sub,
                hex::encode(&pkt.payload)
            );
            HandlerOutcome {
                responses: Vec::new(),
                phase_update: None,
                analysis_note: Some(format!(
                    "unhandled main-frame sub=0x{:02x} payload={}B {}",
                    sub,
                    pkt.payload.len(),
                    hex::encode(&pkt.payload)
                )),
            }
        }
    }
}

/// `Resources/version.dat`의 패키지 버전을 읽습니다.
fn read_local_package_version() -> u16 {
    let Ok(text) = fs::read_to_string("Resources/version.dat") else {
        return 54;
    };

    text.trim().parse::<u16>().unwrap_or(54)
}

/// `DMainFrame` 채널의 초기 부트스트랩 요청에 대해 서버 패키지 버전 응답을 생성합니다.
///
/// 정적 분석상 `DMainFrame` raw body는 선두 `handler_ptr`과 `msg_id`를 분리해 다루고,
/// `msg_id == 0`일 때 payload 선두 `u16`을 서버 패키지 버전으로 읽습니다.
/// 따라서 bootstrap 응답 body는 최소 `[handler_ptr:u32=0][msg_id:u32=0][server_version:u16]`
/// 형식이어야 합니다.
fn build_provisional_main_frame_bootstrap_response(pkt: &ProtocolPacket, ch: u16) -> Vec<u8> {
    let _ = pkt;
    build_main_frame_raw_message(ch, 0, &read_local_package_version().to_le_bytes())
}

/// `DMainFrame` state 10이 채널 2/3을 열도록 유도하는 후속 메시지 8입니다.
fn build_main_frame_stage_open_response(ch: u16) -> Vec<u8> {
    build_main_frame_raw_message(ch, 8, &[])
}

/// `DMainFrame` bootstrap 직후 사용하는 상태 알림 raw 메시지를 생성합니다.
fn build_main_frame_status_message_response(ch: u16, msg_id: u32) -> Vec<u8> {
    build_main_frame_raw_message(ch, msg_id, &[])
}

/// `DMainFrame` state 11이 소비하는 임시 stage-info 메시지 9를 생성합니다.
///
/// 정적 분석상 `msg_id == 9`는 16바이트 구조체를 `this+0x118`에 그대로 복사합니다.
/// 아직 각 필드 의미를 확정하지 못했으므로, 현재는 가장 안전한 0으로 채운 스텁을 보냅니다.
fn build_provisional_main_frame_stage_info_response(ch: u16) -> Vec<u8> {
    build_main_frame_raw_message(ch, 9, &[0u8; 16])
}

/// `DMainFrame` 전용 raw body `[handler_ptr=0][msg_id][payload...]`를 생성합니다.
fn build_main_frame_raw_message(ch: u16, msg_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + payload.len());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&msg_id.to_le_bytes());
    body.extend_from_slice(payload);
    DNetPacket::new(ch, body).to_bytes()
}

/// 채널 2 오픈 직후 `DMainFrame`이 기본 월드맵을 만들 수 있게 임시 stage payload를 구성합니다.
fn build_provisional_worldmap_stage_payload() -> Vec<u8> {
    let mut payload = vec![0u8; 0x120];
    let stage_name = b"WorldMap";
    payload[..stage_name.len()].copy_from_slice(stage_name);
    payload
}

/// 채널 2에 보내는 임시 WorldMap stage bootstrap raw `msg=0`입니다.
fn build_provisional_worldmap_stage_bootstrap_response(ch: u16) -> Vec<u8> {
    build_main_frame_raw_message(ch, 0, &build_provisional_worldmap_stage_payload())
}

/// MainType 0x64 — 공통 시스템 메시지 (버전 핸드셰이크, 로그인 등)를 처리합니다.
fn handle_system(pkt: &ProtocolPacket, ch: u16) -> HandlerOutcome {
    match pkt.sub_type {
        0x01 => {
            // 버전 확인 요청 → 프로토콜 버전 54로 응답
            crate::emu_socket_log!("[SYS] 버전 핸드셰이크 요청 → 버전 54 응답");
            let mut payload = Vec::new();
            payload.extend_from_slice(&protocol::write_u32(54));
            HandlerOutcome {
                responses: vec![protocol::create_app_packet(ch, 0x64, 0x01, &payload)],
                phase_update: Some(ChannelPhase::VersionNegotiated),
                analysis_note: Some("system version handshake acknowledged".to_string()),
            }
        }
        0x02 => {
            // 로그인 요청 → result=0 (성공) 응답
            crate::emu_socket_log!("[SYS] 로그인 요청 → 성공 응답");
            let mut payload = Vec::new();
            payload.extend_from_slice(&protocol::write_u32(0));
            HandlerOutcome {
                responses: vec![protocol::create_app_packet(ch, 0x64, 0x02, &payload)],
                phase_update: Some(ChannelPhase::LoginAccepted),
                analysis_note: Some("login request acknowledged with success".to_string()),
            }
        }
        sub => {
            crate::emu_socket_log!(
                "[SYS] 미구현 sub=0x{:02x} payload={}",
                sub,
                hex::encode(&pkt.payload)
            );
            HandlerOutcome {
                responses: Vec::new(),
                phase_update: None,
                analysis_note: Some(format!(
                    "unhandled system sub=0x{:02x} payload={}B {}",
                    sub,
                    pkt.payload.len(),
                    hex::encode(&pkt.payload)
                )),
            }
        }
    }
}

/// MainType 0x0B — ChatTown Main (입장, 퇴장, 월드 로직)을 처리합니다.
fn handle_chat_town_main(pkt: &ProtocolPacket, _ch: u16) -> HandlerOutcome {
    crate::emu_socket_log!(
        "[ChatTown Main] sub=0x{:02x} payload={}",
        pkt.sub_type,
        hex::encode(&pkt.payload)
    );
    // TODO: 패킷 로그 분석 후 sub_type별 응답 구현
    HandlerOutcome {
        responses: Vec::new(),
        phase_update: None,
        analysis_note: Some(format!(
            "chat-town-main sub=0x{:02x} payload={}B {}",
            pkt.sub_type,
            pkt.payload.len(),
            hex::encode(&pkt.payload)
        )),
    }
}

/// MainType 0x0C — ChatTown Sub (대화, 액션, 아이템 사용)을 처리합니다.
fn handle_chat_town_sub(pkt: &ProtocolPacket, _ch: u16) -> HandlerOutcome {
    crate::emu_socket_log!(
        "[ChatTown Sub] sub=0x{:02x} payload={}",
        pkt.sub_type,
        hex::encode(&pkt.payload)
    );
    // TODO: 채팅 메시지 에코 등 구현 예정
    HandlerOutcome {
        responses: Vec::new(),
        phase_update: None,
        analysis_note: Some(format!(
            "chat-town-sub sub=0x{:02x} payload={}B {}",
            pkt.sub_type,
            pkt.payload.len(),
            hex::encode(&pkt.payload)
        )),
    }
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

        let payload = [0x0d, 0x35, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00];
        to_handler_tx
            .send(protocol::create_app_packet(1, 0xE0, 0x04, &payload))
            .unwrap();

        let version_resp = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(version_resp, build_main_frame_status_message_response(1, 4));
        let version_resp = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let mut expected_body = Vec::new();
        expected_body.extend_from_slice(&0u32.to_le_bytes());
        expected_body.extend_from_slice(&0u32.to_le_bytes());
        expected_body.extend_from_slice(&protocol::write_u16(54));
        assert_eq!(version_resp, DNetPacket::new(1, expected_body).to_bytes());
        let final_status = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(final_status, build_main_frame_status_message_response(1, 6));

        drop(to_handler_tx);
        handle.join().unwrap();
    }

    #[test]
    fn main_frame_bootstrap_response_uses_version_file_payload() {
        let request = ProtocolPacket {
            main_type: 0xE0,
            sub_type: 0x04,
            payload: vec![0x0d, 0x35, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00],
        };

        let response = build_provisional_main_frame_bootstrap_response(&request, 3);
        let mut expected_body = Vec::new();
        expected_body.extend_from_slice(&0u32.to_le_bytes());
        expected_body.extend_from_slice(&0u32.to_le_bytes());
        expected_body.extend_from_slice(&protocol::write_u16(54));

        assert_eq!(response, DNetPacket::new(3, expected_body).to_bytes());
    }

    #[test]
    fn main_frame_stage_open_response_uses_raw_message_id_eight() {
        assert_eq!(
            build_main_frame_stage_open_response(2),
            DNetPacket::new(
                2,
                [0u32.to_le_bytes().as_slice(), 8u32.to_le_bytes().as_slice()].concat(),
            )
            .to_bytes()
        );
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
    fn main_frame_status_message_response_has_empty_payload() {
        assert_eq!(
            build_main_frame_status_message_response(1, 4),
            DNetPacket::new(
                1,
                [0u32.to_le_bytes().as_slice(), 4u32.to_le_bytes().as_slice()].concat(),
            )
            .to_bytes()
        );
    }

    #[test]
    fn provisional_worldmap_stage_payload_is_sized_and_named() {
        let payload = build_provisional_worldmap_stage_payload();

        assert_eq!(payload.len(), 0x120);
        assert_eq!(&payload[..8], b"WorldMap");
        assert!(payload[8..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn stage_channel_three_open_pushes_provisional_worldmap_bootstrap() {
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

        let payload = [0x0d, 0x35, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00];
        to_handler_tx
            .send(protocol::create_app_packet(1, 0xE0, 0x04, &payload))
            .unwrap();
        let _status_four = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let _bootstrap = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let _status_six = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        to_handler_tx
            .send(protocol::create_control_message(protocol::CTRL_OPEN, 3))
            .unwrap();

        assert_eq!(
            from_handler_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            protocol::create_control_message(protocol::CTRL_OPEN_OK, 3)
        );
        assert_eq!(
            from_handler_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            build_provisional_worldmap_stage_bootstrap_response(3)
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
        assert!(should_promote_open_to_mainframe_stage(3, &states));
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
}
