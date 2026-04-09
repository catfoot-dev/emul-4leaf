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

// =========================================================
// 게임 상태 (인메모리 유저 DB 및 세션)
// =========================================================

struct UserRecord {
    password: Vec<u8>, // EUC-KR raw bytes
}

struct SessionInfo {
    user_id: String,
    nickname: Vec<u8>, // EUC-KR, max 22 bytes
    character: u8,
    gp: u32,
    fp: u32,
}

struct GameState {
    users: HashMap<String, UserRecord>,
    session: Option<SessionInfo>,
    client_version_code: u32,
}

impl GameState {
    fn new() -> Self {
        let mut users = HashMap::new();
        users.insert(
            "test".to_string(),
            UserRecord {
                password: b"test".to_vec(),
            },
        );
        GameState {
            users,
            session: None,
            client_version_code: 0x400d04e0,
        }
    }
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

/// 버전 코드를 ProtocolPacket의 main_type/sub_type/payload에서 재구성합니다.
fn extract_version_code(pkt: &ProtocolPacket) -> u32 {
    let mut bytes = [0u8; 4];
    bytes[0] = pkt.main_type;
    bytes[1] = pkt.sub_type;
    if pkt.payload.len() >= 2 {
        bytes[2] = pkt.payload[0];
        bytes[3] = pkt.payload[1];
    }
    u32::from_le_bytes(bytes)
}

/// payload에서 control 필드를 추출합니다 (offset 2..6).
fn extract_control(payload: &[u8]) -> Option<u32> {
    if payload.len() < 6 {
        return None;
    }
    Some(u32::from_le_bytes(
        payload[2..6].try_into().unwrap_or([0; 4]),
    ))
}

/// 에이전트 프로토콜 응답 패킷을 생성합니다.
///
/// Wire: `[ch:u16][body_len:u16][version_code:u32 LE][control:u32 LE][data...]`
fn build_agent_response(ch: u16, version_code: u32, control: u32, data: &[u8]) -> Vec<u8> {
    let vc = version_code.to_le_bytes();
    let mut payload = Vec::with_capacity(2 + 4 + data.len());
    payload.extend_from_slice(&vc[2..4]);
    payload.extend_from_slice(&control.to_le_bytes());
    payload.extend_from_slice(data);
    protocol::create_app_packet(ch, vc[0], vc[1], &payload)
}

/// null-terminated 바이트 배열에서 문자열을 추출합니다.
fn extract_null_terminated_string(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).to_string()
}

/// 아바타 상세정보 바이트를 빌드합니다.
///
/// 포맷: `<character>.1b <nick>.22b 00*5 <equip>.short*16 00*24`
///       `<knights>.24b <gp>.long <fp>.long 00*16`
///       `(<inventory_item_code><inventory_item_type>).4b*35`
fn build_avatar_detail_data(session: &SessionInfo) -> Vec<u8> {
    let mut data = Vec::with_capacity(272);

    // character (1 byte)
    data.push(session.character);

    // nickname (22 bytes, null-padded)
    let mut nick = [0u8; 22];
    let len = session.nickname.len().min(22);
    nick[..len].copy_from_slice(&session.nickname[..len]);
    data.extend_from_slice(&nick);

    // padding (5 bytes)
    data.extend_from_slice(&[0u8; 5]);

    // equip (16 * u16 = 32 bytes)
    data.extend_from_slice(&[0u8; 32]);

    // padding (24 bytes)
    data.extend_from_slice(&[0u8; 24]);

    // knights name (24 bytes)
    data.extend_from_slice(&[0u8; 24]);

    // GP (4 bytes)
    data.extend_from_slice(&session.gp.to_le_bytes());

    // FP (4 bytes)
    data.extend_from_slice(&session.fp.to_le_bytes());

    // padding (16 bytes)
    data.extend_from_slice(&[0u8; 16]);

    // inventory (35 items * 4 bytes = 140 bytes)
    data.extend_from_slice(&[0u8; 140]);

    data
}

fn build_stage_open_acceptance_responses(channel_id: u16) -> Vec<Vec<u8>> {
    let mut responses = vec![protocol::create_control_message(
        protocol::CTRL_OPEN_OK,
        channel_id,
    )];
    if channel_id == 2 || channel_id == 3 {
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
    let mut game_state = GameState::new();

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
                        0xE0 | 0x60 => handle_main_frame(&pkt, channel_id, &mut game_state),
                        0x64 => handle_system(&pkt, channel_id, &mut game_state),
                        0x68 => handle_ping(&pkt, channel_id),
                        0x0A => handle_world_map(&pkt, channel_id),
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

/// MainType 0xE0/0x60 — 메인 프레임 채널 메시지를 처리합니다.
///
/// 본문 구조: `[version_code:u32 LE][control:u32 LE][data...]`
/// ProtocolPacket 파싱 후: main=vc[0], sub=vc[1], payload=[vc[2..4], control[4], data...]
fn handle_main_frame(pkt: &ProtocolPacket, ch: u16, state: &mut GameState) -> HandlerOutcome {
    match pkt.sub_type {
        0x04 | 0x05 => {
            // 클라이언트 버전 코드를 캡처합니다
            state.client_version_code = extract_version_code(pkt);

            // control 필드에 따라 분기합니다
            match extract_control(&pkt.payload) {
                Some(0) => {
                    // 초기 부트스트랩 (버전 + 공지)
                    let response = build_provisional_main_frame_bootstrap_response(pkt, ch);
                    HandlerOutcome {
                        responses: vec![response],
                        phase_update: Some(ChannelPhase::BootstrapVersionSent),
                        analysis_note: Some(format!(
                            "bootstrap responded with raw msg=0(version={})",
                            read_local_package_version()
                        )),
                    }
                }
                Some(3) => handle_registration_request(ch, state),
                Some(4) => handle_id_check(pkt, ch, state),
                Some(5) => handle_registration_submit(pkt, ch, state),
                Some(7) => handle_avatar_selection(pkt, ch, state),
                Some(9) => handle_logout(ch, state),
                Some(control) => {
                    crate::emu_socket_log!(
                        "[MainFrame] 미구현 control={} payload={}",
                        control,
                        hex::encode(&pkt.payload)
                    );
                    HandlerOutcome {
                        responses: Vec::new(),
                        phase_update: None,
                        analysis_note: Some(format!(
                            "unhandled main-frame control={} payload={}B",
                            control,
                            pkt.payload.len()
                        )),
                    }
                }
                None => {
                    // payload가 너무 짧으면 부트스트랩으로 처리
                    let response = build_provisional_main_frame_bootstrap_response(pkt, ch);
                    HandlerOutcome {
                        responses: vec![response],
                        phase_update: Some(ChannelPhase::BootstrapVersionSent),
                        analysis_note: Some("bootstrap (short payload fallback)".to_string()),
                    }
                }
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

/// `version.dat`의 패키지 버전을 읽습니다.
fn read_local_package_version() -> u16 {
    let Ok(text) = fs::read_to_string(crate::resource_dir().join("version.dat")) else {
        return 54;
    };

    text.trim().parse::<u16>().unwrap_or(54)
}

/// `DMainFrame` 로그인 화면에 표시되는 공지 타이틀 텍스트입니다.
fn get_news_title_text() -> &'static [u8] {
    b"4Leaf Emulator!\r\n\0"
}

/// `DMainFrame` 채널의 초기 부트스트랩 요청에 대해 서버 패키지 버전 응답을 생성합니다.
///
/// 정적 분석상 `DMainFrame` raw body는 선두 `handler_ptr`과 `msg_id`를 분리해 다루고,
/// `msg_id == 0`일 때 payload 선두 `u16`을 서버 패키지 버전으로 읽습니다.
/// 그 뒤에는 바로 `char*`처럼 해석되는 CRLF/NUL 종료 텍스트 블록을 추가로 읽습니다.
/// 따라서 bootstrap 응답 body는 최소
/// `[handler_ptr:u32=0][msg_id:u32=0][server_version:u16][text...\r\n\0]`
/// 형식이어야 합니다.
fn build_provisional_main_frame_bootstrap_response(pkt: &ProtocolPacket, ch: u16) -> Vec<u8> {
    let _ = pkt;
    let mut payload = Vec::new();
    payload.extend_from_slice(&read_local_package_version().to_le_bytes());
    // 원본은 version 뒤를 공지 문자열처럼 계속 읽으므로 euc-kr 텍스트를 포함합니다.
    payload.extend_from_slice(get_news_title_text());
    build_main_frame_raw_message(ch, 0, &payload)
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

/// MainType 0x68 — 핑(Ping) / KeepAlive 메시지를 처리합니다.
fn handle_ping(pkt: &ProtocolPacket, ch: u16) -> HandlerOutcome {
    crate::emu_socket_log!(
        "[PING] 핑 요청 수신 (sub=0x{:02x}, len={}) → 에코 응답",
        pkt.sub_type,
        pkt.payload.len()
    );

    // 수신한 서브타입과 페이로드를 그대로 에코 응답합니다.
    HandlerOutcome {
        responses: vec![protocol::create_app_packet(
            ch,
            pkt.main_type,
            pkt.sub_type,
            &pkt.payload,
        )],
        phase_update: None,
        analysis_note: Some(format!(
            "ping echoed main=0x{:02x} sub=0x{:02x} payload={}B",
            pkt.main_type,
            pkt.sub_type,
            pkt.payload.len()
        )),
    }
}

/// MainType 0x64 — 공통 시스템 메시지 (버전 핸드셰이크, 로그인 등)를 처리합니다.
fn handle_system(pkt: &ProtocolPacket, ch: u16, state: &mut GameState) -> HandlerOutcome {
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
            // 로그인 요청: 세션을 생성하고 아바타 상세정보를 전송합니다
            crate::emu_socket_log!("[SYS] 로그인 요청 수신");

            // payload에서 ID를 추출합니다 (있으면)
            let user_id = if !pkt.payload.is_empty() {
                extract_null_terminated_string(&pkt.payload)
            } else {
                "test".to_string()
            };

            crate::emu_socket_log!("[SYS] 로그인 ID={}", user_id);

            // 세션 생성 (모든 로그인을 수락합니다)
            let nickname = if user_id.len() > 22 {
                user_id[..22].as_bytes().to_vec()
            } else {
                user_id.as_bytes().to_vec()
            };
            state.session = Some(SessionInfo {
                user_id: user_id.clone(),
                nickname,
                character: 0,
                gp: 1000,
                fp: 0,
            });

            // 로그인 성공 응답 (result=0)
            let mut responses = Vec::new();
            let mut login_payload = Vec::new();
            login_payload.extend_from_slice(&protocol::write_u32(0));
            responses.push(protocol::create_app_packet(ch, 0x64, 0x02, &login_payload));

            HandlerOutcome {
                responses,
                phase_update: Some(ChannelPhase::LoginAccepted),
                analysis_note: Some(format!("login accepted for user={}", user_id)),
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

// =========================================================
// 회원가입 핸들러
// =========================================================

/// control=3 — 가입 요청: 가입 안내 메시지를 회신합니다.
fn handle_registration_request(ch: u16, state: &GameState) -> HandlerOutcome {
    crate::emu_socket_log!("[REG] 가입 요청 수신 → 가입 안내 메시지 송신");
    // 가입 안내 메시지를 control=0으로 응답
    let join_msg = b"Welcome to 4Leaf Server!\r\n\0";
    let response = build_agent_response(ch, state.client_version_code, 0, join_msg);
    HandlerOutcome {
        responses: vec![response],
        phase_update: None,
        analysis_note: Some("registration request → join message sent".to_string()),
    }
}

/// control=4 — 아이디 중복 확인: 사용 가능 여부를 회신합니다.
///
/// 요청 payload: `[version_hi:2][control=4:u32][id_data...]`
/// 응답: `[version_code:u32][control=1:u32][result:u32]` (12=가능, 0=사용중)
fn handle_id_check(pkt: &ProtocolPacket, ch: u16, state: &GameState) -> HandlerOutcome {
    // payload[6..] 에서 ID를 추출합니다
    let id = if pkt.payload.len() > 6 {
        extract_null_terminated_string(&pkt.payload[6..])
    } else {
        String::new()
    };

    let available = !id.is_empty() && !state.users.contains_key(&id);
    let result: u32 = if available { 12 } else { 0 };

    crate::emu_socket_log!("[REG] 아이디 중복 확인: id={} available={}", id, available);

    let response = build_agent_response(ch, state.client_version_code, 1, &result.to_le_bytes());
    HandlerOutcome {
        responses: vec![response],
        phase_update: None,
        analysis_note: Some(format!("id-check id={} result={}", id, result)),
    }
}

/// control=5 — 가입 정보 수신: 유저를 DB에 등록합니다.
///
/// 요청 payload: `[version_hi:2][control=5:u32][id:16b][unknown:20b][pass:16b][...]`
fn handle_registration_submit(
    pkt: &ProtocolPacket,
    ch: u16,
    state: &mut GameState,
) -> HandlerOutcome {
    let base = 6; // version_hi(2) + control(4) 이후
    if pkt.payload.len() < base + 52 {
        crate::emu_socket_log!("[REG] 가입 정보 패킷이 너무 짧음");
        return HandlerOutcome {
            responses: Vec::new(),
            phase_update: None,
            analysis_note: Some("registration submit too short".to_string()),
        };
    }

    let id = extract_null_terminated_string(&pkt.payload[base..base + 16]);
    let pass = pkt.payload[base + 36..base + 52].to_vec();

    crate::emu_socket_log!("[REG] 가입 처리: id={}", id);

    state
        .users
        .insert(id.clone(), UserRecord { password: pass });

    // 가입 완료 메시지를 control=0으로 응답
    let msg = b"Registration complete!\r\n\0";
    let response = build_agent_response(ch, state.client_version_code, 0, msg);
    HandlerOutcome {
        responses: vec![response],
        phase_update: None,
        analysis_note: Some(format!("registration complete id={}", id)),
    }
}

// =========================================================
// 아바타 및 로그인 후 핸들러
// =========================================================

/// control=7 — 아바타 선택: 아바타 상세정보를 회신합니다.
///
/// 요청 payload: `[version_hi:2][control=7:u32][avatar_index:u8]`
/// 응답: control=0 아바타 상세정보 + control=6 방문수당
fn handle_avatar_selection(pkt: &ProtocolPacket, ch: u16, state: &mut GameState) -> HandlerOutcome {
    let avatar_index = pkt.payload.get(6).copied().unwrap_or(0);
    crate::emu_socket_log!("[AVATAR] 아바타 선택: index={}", avatar_index);

    // 세션에 아바타 정보를 설정합니다
    if let Some(ref mut session) = state.session {
        session.character = avatar_index;
    }

    let mut responses = Vec::new();

    // 아바타 상세정보 응답 (control=0)
    if let Some(ref session) = state.session {
        let detail = build_avatar_detail_data(session);
        responses.push(build_agent_response(
            ch,
            state.client_version_code,
            0,
            &detail,
        ));
    }

    // 방문수당 응답 (control=6): [0:u32][visit_gp:u32][0:u32]
    let mut visit_data = Vec::new();
    visit_data.extend_from_slice(&0u32.to_le_bytes());
    visit_data.extend_from_slice(&100u32.to_le_bytes()); // 방문수당 100 GP
    visit_data.extend_from_slice(&0u32.to_le_bytes());
    responses.push(build_agent_response(
        ch,
        state.client_version_code,
        6,
        &visit_data,
    ));

    HandlerOutcome {
        responses,
        phase_update: None,
        analysis_note: Some(format!("avatar selected index={}", avatar_index)),
    }
}

/// control=9 — 종료: 이용시간/GP 정산 후 채널을 닫습니다.
fn handle_logout(ch: u16, state: &mut GameState) -> HandlerOutcome {
    crate::emu_socket_log!("[LOGOUT] 종료 정산 처리");

    state.session = None;

    // 종료 신호 (채널 닫기 시퀀스)
    let responses = vec![protocol::create_control_message(protocol::CTRL_CLOSE, ch)];

    HandlerOutcome {
        responses,
        phase_update: None,
        analysis_note: Some("logout settlement processed".to_string()),
    }
}

// =========================================================
// 월드맵 핸들러
// =========================================================

/// 월드맵 채널(0x0a) 데이터를 처리합니다.
///
/// 공통 포맷: `[control_type:u32 LE][message...]`
fn handle_world_map(pkt: &ProtocolPacket, _ch: u16) -> HandlerOutcome {
    crate::emu_socket_log!(
        "[WorldMap] sub=0x{:02x} payload={}B",
        pkt.sub_type,
        pkt.payload.len()
    );

    // 월드맵 응답: 현재는 수신 확인만 합니다
    // 클라이언트가 구역 이동을 요청하면 해당 구역의 유저 리스트 등을 응답해야 합니다
    HandlerOutcome {
        responses: Vec::new(),
        phase_update: None,
        analysis_note: Some(format!(
            "world-map sub=0x{:02x} payload={}B {}",
            pkt.sub_type,
            pkt.payload.len(),
            hex::encode(&pkt.payload)
        )),
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
        expected_body.extend_from_slice(get_news_title_text());

        assert_eq!(response, DNetPacket::new(3, expected_body).to_bytes());
    }

    #[test]
    fn main_frame_bootstrap_response_terminates_followup_text_block() {
        let request = ProtocolPacket {
            main_type: 0xE0,
            sub_type: 0x04,
            payload: vec![0x0d, 0x35, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00],
        };

        let response = build_provisional_main_frame_bootstrap_response(&request, 1);
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
        let _bootstrap = from_handler_rx
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

    // =========================================================
    // 게임 상태 및 신규 핸들러 테스트
    // =========================================================

    #[test]
    fn extract_version_code_reconstructs_original_value() {
        let pkt = ProtocolPacket {
            main_type: 0xe0,
            sub_type: 0x04,
            payload: vec![0x0d, 0x40, 0x00, 0x00, 0x00, 0x00],
        };
        assert_eq!(extract_version_code(&pkt), 0x400d04e0);
    }

    #[test]
    fn extract_control_parses_from_payload_offset_two() {
        // payload: [version_hi:2][control:4] = [0x0d, 0x40, 0x03, 0x00, 0x00, 0x00]
        let payload = vec![0x0d, 0x40, 0x03, 0x00, 0x00, 0x00];
        assert_eq!(extract_control(&payload), Some(3));
    }

    #[test]
    fn extract_control_returns_none_for_short_payload() {
        assert_eq!(extract_control(&[0x0d, 0x40, 0x03]), None);
    }

    #[test]
    fn build_agent_response_produces_correct_wire_format() {
        let wire = build_agent_response(1, 0x400d04e0, 3, &[0xaa]);
        // Expected: [ch=1:u16][body_len:u16][0xe0][0x04][0x0d][0x40][03 00 00 00][0xaa]
        let header = &wire[..4];
        assert_eq!(header[0..2], [0x01, 0x00]); // channel 1
        let body = &wire[4..];
        assert_eq!(body[0], 0xe0); // main_type
        assert_eq!(body[1], 0x04); // sub_type
        assert_eq!(body[2..4], [0x0d, 0x40]); // version_hi
        assert_eq!(body[4..8], [0x03, 0x00, 0x00, 0x00]); // control=3
        assert_eq!(body[8], 0xaa); // data
    }

    #[test]
    fn registration_request_returns_join_message() {
        let state = GameState::new();
        let outcome = handle_registration_request(1, &state);
        assert_eq!(outcome.responses.len(), 1);
        // 응답은 control=0이어야 합니다
        let resp = &outcome.responses[0];
        let body = &resp[4..];
        // control at offset 4..8
        let control = u32::from_le_bytes(body[4..8].try_into().unwrap());
        assert_eq!(control, 0);
    }

    #[test]
    fn id_check_reports_available_for_new_id() {
        let state = GameState::new();
        // control=4, id="newuser\0" after version_hi + control
        let mut payload = vec![0x0d, 0x40, 0x04, 0x00, 0x00, 0x00];
        payload.extend_from_slice(b"newuser\0");
        let pkt = ProtocolPacket {
            main_type: 0xe0,
            sub_type: 0x04,
            payload,
        };
        let outcome = handle_id_check(&pkt, 1, &state);
        assert_eq!(outcome.responses.len(), 1);
        // 응답에서 result=12 (사용 가능)를 확인
        let resp = &outcome.responses[0];
        let body = &resp[4..];
        let result = u32::from_le_bytes(body[8..12].try_into().unwrap());
        assert_eq!(result, 12);
    }

    #[test]
    fn id_check_reports_taken_for_existing_id() {
        let state = GameState::new();
        let mut payload = vec![0x0d, 0x40, 0x04, 0x00, 0x00, 0x00];
        payload.extend_from_slice(b"test\0");
        let pkt = ProtocolPacket {
            main_type: 0xe0,
            sub_type: 0x04,
            payload,
        };
        let outcome = handle_id_check(&pkt, 1, &state);
        let resp = &outcome.responses[0];
        let body = &resp[4..];
        let result = u32::from_le_bytes(body[8..12].try_into().unwrap());
        assert_eq!(result, 0); // 이미 사용 중
    }

    #[test]
    fn registration_submit_creates_new_user() {
        let mut state = GameState::new();
        // payload: [version_hi:2][control=5:4][id:16][unknown:20][pass:16]
        let mut payload = vec![0x0d, 0x40, 0x05, 0x00, 0x00, 0x00];
        let mut id_field = [0u8; 16];
        id_field[..5].copy_from_slice(b"hello");
        payload.extend_from_slice(&id_field);
        payload.extend_from_slice(&[0u8; 20]); // unknown
        let mut pass_field = [0u8; 16];
        pass_field[..5].copy_from_slice(b"world");
        payload.extend_from_slice(&pass_field);
        let pkt = ProtocolPacket {
            main_type: 0xe0,
            sub_type: 0x04,
            payload,
        };
        let outcome = handle_registration_submit(&pkt, 1, &mut state);
        assert!(!outcome.responses.is_empty());
        assert!(state.users.contains_key("hello"));
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
        // 1 + 22 + 5 + 32 + 24 + 24 + 4 + 4 + 16 + 140 = 272
        assert_eq!(data.len(), 272);
        assert_eq!(data[0], 1); // character
        assert_eq!(&data[1..9], b"TestUser"); // nickname start
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
        let payload = vec![0x0d, 0x40, 0x07, 0x00, 0x00, 0x00, 0x01];
        let pkt = ProtocolPacket {
            main_type: 0xe0,
            sub_type: 0x04,
            payload,
        };
        let outcome = handle_avatar_selection(&pkt, 1, &mut state);
        // 아바타 상세정보 + 방문수당 = 2개 응답
        assert_eq!(outcome.responses.len(), 2);
        assert_eq!(state.session.as_ref().unwrap().character, 1);
    }

    #[test]
    fn main_frame_dispatches_registration_request_on_control_three() {
        let (to_handler_tx, to_handler_rx) = mpsc::channel();
        let (from_handler_tx, from_handler_rx) = mpsc::channel();
        let handle = thread::spawn(move || run_dnet_handler(to_handler_rx, from_handler_tx));

        // 채널 1 오픈
        to_handler_tx
            .send(protocol::create_control_message(protocol::CTRL_OPEN, 1))
            .unwrap();
        let _open_ok = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        // 부트스트랩 (control=0)
        let bootstrap_payload = [0x0d, 0x40, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00];
        to_handler_tx
            .send(protocol::create_app_packet(
                1,
                0xE0,
                0x04,
                &bootstrap_payload,
            ))
            .unwrap();
        let _bootstrap_resp = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        // 가입 요청 (control=3)
        let reg_payload = [0x0d, 0x40, 0x03, 0x00, 0x00, 0x00];
        to_handler_tx
            .send(protocol::create_app_packet(1, 0xE0, 0x04, &reg_payload))
            .unwrap();
        let reg_resp = from_handler_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        // 응답이 agent protocol format (control=0, 가입 안내 메시지)인지 확인
        let body = &reg_resp[4..];
        assert_eq!(body[0], 0xe0); // main_type
        assert_eq!(body[1], 0x04); // sub_type
        let control = u32::from_le_bytes(body[4..8].try_into().unwrap());
        assert_eq!(control, 0); // control=0 (가입 안내)

        drop(to_handler_tx);
        handle.join().unwrap();
    }
}
