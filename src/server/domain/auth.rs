use std::fs;

use crate::server::{
    protocol::{self, AuthPacket},
    session::{
        Session, build_avatar_detail_data, build_avatar_dialog_bootstrap_payload,
        build_default_session, build_pending_avatar_session, extract_null_terminated_string,
    },
};

const SERVER_CHECK: u32 = 0;
const LOGIN: u32 = 1;
const REGISTRATION: u32 = 3;
const ID_CHECK: u32 = 4;
const REGISTRATION_SUBMIT: u32 = 5;
const AVATAR_SELECTION: u32 = 7;
const LOGOUT: u32 = 9;

/// Auth (채널 1) 패킷을 control 값 기준으로 세부 처리기에 분기합니다.
pub(crate) fn handle_auth(pkt: &AuthPacket, ch: u16, session: &mut Session) -> Vec<u8> {
    match pkt.control {
        SERVER_CHECK => server_check_response(ch),
        LOGIN => login_response(pkt, ch, session),
        REGISTRATION => registration_response(pkt, ch),
        ID_CHECK => id_check_response(pkt, ch, session),
        REGISTRATION_SUBMIT => registration_submit_response(pkt, ch, session),
        AVATAR_SELECTION => avatar_selection_response(pkt, ch, session),
        LOGOUT => logout_response(ch, session),
        control => {
            crate::emu_socket_log!(
                "[Auth] 미구현 code={:#x} control={} payload={}",
                pkt.code,
                control,
                hex::encode(&pkt.payload)
            );
            Vec::new()
        }
    }
}

/// 서버 체크 응답을 생성합니다.
fn server_check_response(ch: u16) -> Vec<u8> {
    let mut payload = Vec::new();

    let version = if let Ok(text) = fs::read_to_string(crate::resource_dir().join("version.dat")) {
        text.trim().parse::<u16>().unwrap_or(54)
    } else {
        54
    };
    payload.extend_from_slice(&version.to_le_bytes());
    let news_title = b"4Leaf Emulator!\r\n\0";
    payload.extend_from_slice(news_title);

    protocol::create_auth_packet(ch, 0, 0, &payload)
}

/// 로그인 요청 payload에서 사용자 ID를 보수적으로 추출합니다.
fn parse_login_user_id(payload: &[u8]) -> String {
    if payload.len() >= 16 {
        let fixed = extract_null_terminated_string(&payload[..16]);
        if !fixed.is_empty() {
            return fixed;
        }
    }

    extract_null_terminated_string(payload)
}

/// 로그인 성공 시 아바타 선택 창 후보 응답과 기존 로그인 상태 스텁을 함께 생성합니다.
fn login_response(pkt: &AuthPacket, ch: u16, session: &mut Session) -> Vec<u8> {
    let user_id = parse_login_user_id(&pkt.payload);
    let session_info = if user_id.is_empty() {
        build_default_session("test")
    } else if let Some(current) = session.info.clone() {
        if current.user_id == user_id {
            current
        } else {
            build_default_session(&user_id)
        }
    } else {
        build_default_session(&user_id)
    };

    session.info = Some(session_info.clone());

    let bootstrap_payload = build_avatar_dialog_bootstrap_payload(&session_info);
    let mut responses = protocol::create_auth_packet(ch, pkt.code, 0, &bootstrap_payload);

    let mut legacy_payload = Vec::new();
    legacy_payload.extend_from_slice(&0u64.to_le_bytes());
    legacy_payload.extend_from_slice(&0u8.to_le_bytes());
    legacy_payload.extend_from_slice(&0u64.to_le_bytes());
    legacy_payload.extend_from_slice(&0u8.to_le_bytes());
    legacy_payload.extend_from_slice(&0u8.to_le_bytes());
    legacy_payload.extend_from_slice(&0u16.to_le_bytes());
    legacy_payload.extend_from_slice(&0u8.to_le_bytes());
    legacy_payload.extend_from_slice(&0u8.to_le_bytes());
    responses.extend_from_slice(&protocol::create_auth_packet(
        ch,
        pkt.code,
        0x1e,
        &legacy_payload,
    ));

    responses
}

/// 가입 안내 메시지를 응답합니다.
fn registration_response(pkt: &AuthPacket, ch: u16) -> Vec<u8> {
    crate::emu_socket_log!("[REG] 가입 요청 수신 → 가입 안내 메시지 송신");
    let join_msg = b"Welcome to 4Leaf Server!\0";
    protocol::create_auth_packet(ch, pkt.code, 0, join_msg)
}

/// 아이디 중복 확인 요청을 처리합니다.
fn id_check_response(pkt: &AuthPacket, ch: u16, session: &Session) -> Vec<u8> {
    let id = if !pkt.payload.is_empty() {
        extract_null_terminated_string(&pkt.payload)
    } else {
        String::new()
    };

    let available = !id.is_empty() && !session.profile.contains_key(&id);
    let result: u32 = if available { 12 } else { 0 };

    crate::emu_socket_log!("[REG] 아이디 중복 확인: id={} available={}", id, available);

    protocol::create_auth_packet(ch, pkt.code, 1, &result.to_le_bytes())
}

/// 가입 정보를 받아 유저 DB와 세션 상태를 갱신합니다.
fn registration_submit_response(pkt: &AuthPacket, ch: u16, session: &mut Session) -> Vec<u8> {
    let base = 0;
    if pkt.payload.len() < base + 52 {
        crate::emu_socket_log!("[REG] 가입 정보 패킷이 너무 짧음");
        return Vec::new();
    }

    let id = extract_null_terminated_string(&pkt.payload[base..base + 16]);
    let pass = extract_null_terminated_string(&pkt.payload[base + 36..base + 52]);

    if id.is_empty() {
        crate::emu_socket_log!("[REG] 빈 아이디 가입 요청 거부");
        return protocol::create_auth_packet(ch, pkt.code, 0, b"Registration failed.\r\n\0");
    }

    if session.profile.contains_key(&id) {
        crate::emu_socket_log!("[REG] 이미 존재하는 아이디 가입 요청: id={}", id);
        return protocol::create_auth_packet(ch, pkt.code, 0, b"ID already in use.\r\n\0");
    }

    crate::emu_socket_log!("[REG] 가입 처리: id={}", id);
    session.profile.insert(id.clone(), pass.clone());
    let pending_session = build_pending_avatar_session(&id);
    let bootstrap_payload = build_avatar_dialog_bootstrap_payload(&pending_session);
    session.info = Some(pending_session);

    protocol::create_auth_packet(ch, pkt.code, 0, &bootstrap_payload)
}

/// 아바타 선택 요청에 대해 상세정보와 방문수당을 응답합니다.
fn avatar_selection_response(pkt: &AuthPacket, ch: u16, session: &mut Session) -> Vec<u8> {
    let avatar_index = pkt.payload.first().copied().unwrap_or(0);
    crate::emu_socket_log!("[AVATAR] 아바타 선택: index={}", avatar_index);

    if let Some(ref mut session) = session.info {
        session.character = avatar_index;
    }

    let mut responses: Vec<u8> = Vec::new();

    if let Some(ref session) = session.info {
        let detail = build_avatar_detail_data(session);
        responses.append(&mut protocol::create_auth_packet(ch, pkt.code, 0, &detail));
    }

    let mut visit_data = Vec::new();
    visit_data.extend_from_slice(&0u32.to_le_bytes());
    visit_data.extend_from_slice(&100u32.to_le_bytes());
    visit_data.extend_from_slice(&0u32.to_le_bytes());
    responses.append(&mut protocol::create_auth_packet(
        ch,
        pkt.code,
        6,
        &visit_data,
    ));

    responses
}

/// 로그아웃 요청을 처리하고 채널 종료 제어 메시지를 생성합니다.
fn logout_response(ch: u16, session: &mut Session) -> Vec<u8> {
    crate::emu_socket_log!("[LOGOUT] 종료 정산 처리");
    session.info = None;

    protocol::create_control_message(protocol::CTRL_CLOSE, ch)
}
