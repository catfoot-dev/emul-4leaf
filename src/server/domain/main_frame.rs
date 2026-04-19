use std::fs;

use crate::server::{
    analysis::{ChannelPhase, HandlerOutcome},
    protocol::{self, DNetPacket, ProtocolPacket},
    state::{
        GameState, build_avatar_detail_data, build_default_session, extract_null_terminated_string,
    },
};

/// MainFrame 계열 패킷에서 32비트 버전 코드를 다시 조립합니다.
pub(crate) fn extract_version_code(pkt: &ProtocolPacket) -> u32 {
    let mut bytes = [0u8; 4];
    bytes[0] = pkt.main_type;
    bytes[1] = pkt.sub_type;
    if pkt.payload.len() >= 2 {
        bytes[2] = pkt.payload[0];
        bytes[3] = pkt.payload[1];
    }
    u32::from_le_bytes(bytes)
}

/// MainFrame payload의 control 필드를 `payload[2..6]`에서 추출합니다.
pub(crate) fn extract_control(payload: &[u8]) -> Option<u32> {
    if payload.len() < 6 {
        return None;
    }

    Some(u32::from_le_bytes(
        payload[2..6].try_into().unwrap_or([0; 4]),
    ))
}

/// Agent 응답 패킷을 MainFrame wire 포맷으로 포장합니다.
pub(crate) fn build_agent_response(
    ch: u16,
    version_code: u32,
    control: u32,
    data: &[u8],
) -> Vec<u8> {
    let vc = version_code.to_le_bytes();
    let mut payload = Vec::with_capacity(2 + 4 + data.len());
    payload.extend_from_slice(&vc[2..4]);
    payload.extend_from_slice(&control.to_le_bytes());
    payload.extend_from_slice(data);
    protocol::create_app_packet(ch, vc[0], vc[1], &payload)
}

/// 로그인 화면에 표시할 공지 제목 텍스트를 반환합니다.
pub(crate) fn get_news_title_text() -> &'static [u8] {
    b"4Leaf Emulator!\r\n\0"
}

/// `version.dat` 파일에서 서버 패키지 버전을 읽습니다.
pub(crate) fn read_local_package_version() -> u16 {
    let Ok(text) = fs::read_to_string(crate::resource_dir().join("version.dat")) else {
        return 54;
    };

    text.trim().parse::<u16>().unwrap_or(54)
}

/// MainFrame bootstrap 요청에 대응하는 임시 raw 응답을 생성합니다.
pub(crate) fn build_provisional_main_frame_bootstrap_response(
    pkt: &ProtocolPacket,
    ch: u16,
) -> Vec<u8> {
    let _ = pkt;
    let mut payload = Vec::new();
    payload.extend_from_slice(&read_local_package_version().to_le_bytes());
    payload.extend_from_slice(get_news_title_text());
    build_main_frame_raw_message(ch, 0, &payload)
}

/// MainFrame state 11이 읽는 stage-info 메시지의 임시 스텁을 생성합니다.
#[allow(dead_code)]
pub(crate) fn build_provisional_main_frame_stage_info_response(ch: u16) -> Vec<u8> {
    build_main_frame_raw_message(ch, 9, &[0u8; 16])
}

/// MainFrame 전용 raw 바디를 `[handler_ptr=0][msg_id][payload...]` 형태로 생성합니다.
pub(crate) fn build_main_frame_raw_message(ch: u16, msg_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + payload.len());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&msg_id.to_le_bytes());
    body.extend_from_slice(payload);
    DNetPacket::new(ch, body).to_bytes()
}

/// 채널 2 오픈 직후 기본 WorldMap stage를 만들 수 있게 임시 payload를 준비합니다.
pub(crate) fn build_provisional_worldmap_stage_payload() -> Vec<u8> {
    let mut payload = vec![0u8; 0x120];
    let stage_name = b"WorldMap";
    payload[..stage_name.len()].copy_from_slice(stage_name);
    payload
}

/// 채널 2에 전달하는 임시 WorldMap bootstrap raw 메시지를 생성합니다.
pub(crate) fn build_provisional_worldmap_stage_bootstrap_response(ch: u16) -> Vec<u8> {
    build_main_frame_raw_message(ch, 0, &build_provisional_worldmap_stage_payload())
}

/// MainFrame 계열 패킷을 control 값 기준으로 세부 처리기에 분기합니다.
pub(crate) fn handle_main_frame(
    pkt: &ProtocolPacket,
    ch: u16,
    state: &mut GameState,
) -> HandlerOutcome {
    match pkt.sub_type {
        0x04 | 0x05 => {
            state.client_version_code = extract_version_code(pkt);

            match extract_control(&pkt.payload) {
                Some(0) => {
                    let response = build_provisional_main_frame_bootstrap_response(pkt, ch);
                    HandlerOutcome {
                        responses: vec![response],
                        phase_update: Some(ChannelPhase::BootstrapVersionSent),
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
                    }
                }
                None => {
                    let response = build_provisional_main_frame_bootstrap_response(pkt, ch);
                    HandlerOutcome {
                        responses: vec![response],
                        phase_update: Some(ChannelPhase::BootstrapVersionSent),
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
            }
        }
    }
}

/// 가입 안내 메시지를 응답합니다.
pub(crate) fn handle_registration_request(ch: u16, state: &GameState) -> HandlerOutcome {
    crate::emu_socket_log!("[REG] 가입 요청 수신 → 가입 안내 메시지 송신");
    let join_msg = b"Welcome to 4Leaf Server!\r\n\0";
    let response = build_agent_response(ch, state.client_version_code, 0, join_msg);

    HandlerOutcome {
        responses: vec![response],
        phase_update: None,
    }
}

/// 아이디 중복 확인 요청을 처리합니다.
pub(crate) fn handle_id_check(pkt: &ProtocolPacket, ch: u16, state: &GameState) -> HandlerOutcome {
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
    }
}

/// 가입 정보를 받아 유저 DB와 세션 상태를 갱신합니다.
pub(crate) fn handle_registration_submit(
    pkt: &ProtocolPacket,
    ch: u16,
    state: &mut GameState,
) -> HandlerOutcome {
    let base = 6;
    if pkt.payload.len() < base + 52 {
        crate::emu_socket_log!("[REG] 가입 정보 패킷이 너무 짧음");
        return HandlerOutcome {
            responses: Vec::new(),
            phase_update: None,
        };
    }

    let id = extract_null_terminated_string(&pkt.payload[base..base + 16]);
    let pass = pkt.payload[base + 36..base + 52].to_vec();

    if id.is_empty() {
        crate::emu_socket_log!("[REG] 빈 아이디 가입 요청 거부");
        let response = build_agent_response(
            ch,
            state.client_version_code,
            0,
            b"Registration failed.\r\n\0",
        );
        return HandlerOutcome {
            responses: vec![response],
            phase_update: None,
        };
    }

    if state.users.contains_key(&id) {
        crate::emu_socket_log!("[REG] 이미 존재하는 아이디 가입 요청: id={}", id);
        let response = build_agent_response(
            ch,
            state.client_version_code,
            0,
            b"ID already in use.\r\n\0",
        );
        return HandlerOutcome {
            responses: vec![response],
            phase_update: None,
        };
    }

    crate::emu_socket_log!("[REG] 가입 처리: id={}", id);
    state.users.insert(
        id.clone(),
        crate::server::state::UserRecord { password: pass },
    );
    state.session = Some(build_default_session(&id));

    let response = build_agent_response(
        ch,
        state.client_version_code,
        0,
        b"Registration complete!\r\n\0",
    );
    HandlerOutcome {
        responses: vec![response],
        phase_update: None,
    }
}

/// 아바타 선택 요청에 대해 상세정보와 방문수당을 응답합니다.
pub(crate) fn handle_avatar_selection(
    pkt: &ProtocolPacket,
    ch: u16,
    state: &mut GameState,
) -> HandlerOutcome {
    let avatar_index = pkt.payload.get(6).copied().unwrap_or(0);
    crate::emu_socket_log!("[AVATAR] 아바타 선택: index={}", avatar_index);

    if let Some(ref mut session) = state.session {
        session.character = avatar_index;
    }

    let mut responses = Vec::new();

    if let Some(ref session) = state.session {
        let detail = build_avatar_detail_data(session);
        responses.push(build_agent_response(
            ch,
            state.client_version_code,
            0,
            &detail,
        ));
    }

    let mut visit_data = Vec::new();
    visit_data.extend_from_slice(&0u32.to_le_bytes());
    visit_data.extend_from_slice(&100u32.to_le_bytes());
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
    }
}

/// 로그아웃 요청을 처리하고 채널 종료 제어 메시지를 생성합니다.
pub(crate) fn handle_logout(ch: u16, state: &mut GameState) -> HandlerOutcome {
    crate::emu_socket_log!("[LOGOUT] 종료 정산 처리");
    state.session = None;

    HandlerOutcome {
        responses: vec![protocol::create_control_message(protocol::CTRL_CLOSE, ch)],
        phase_update: None,
    }
}
