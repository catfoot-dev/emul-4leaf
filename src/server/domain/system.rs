use crate::server::{
    protocol::{self, ChannelPacket},
    session::{Session, build_default_session, extract_null_terminated_string},
};

const LOGIN_OK: u32 = 0;
const LOGIN_ERR_NO_ID: u32 = 1;
const LOGIN_ERR_BAD_PASSWORD: u32 = 2;
const LOGIN_ERR_AUTH_FAILED: u32 = 4;

/// 로그인 페이로드 필드를 NUL 이전까지만 잘라 비교용 바이트 슬라이스로 반환합니다.
fn trim_login_field(data: &[u8]) -> &[u8] {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    &data[..end]
}

/// 로그인 페이로드에서 아이디와 비밀번호를 최대한 보수적으로 추출합니다.
fn parse_login_credentials(payload: &[u8]) -> Option<(String, Vec<u8>)> {
    if payload.is_empty() {
        return None;
    }

    // 일부 패킷은 16바이트 고정폭 ID/PW 블록을 쓰므로 먼저 그 형식을 허용합니다.
    if payload.len() >= 32 {
        let fixed_id = trim_login_field(&payload[..16]);
        let fixed_password = trim_login_field(&payload[16..32]);
        if !fixed_id.is_empty() && !fixed_password.is_empty() {
            return Some((
                String::from_utf8_lossy(fixed_id).to_string(),
                fixed_password.to_vec(),
            ));
        }
    }

    let user_id = extract_null_terminated_string(payload);
    if user_id.is_empty() {
        return None;
    }

    let password = payload
        .iter()
        .position(|&b| b == 0)
        .and_then(|end| payload.get(end + 1..))
        .map(trim_login_field)
        .filter(|field| !field.is_empty())
        .map(|field| field.to_vec())
        .or_else(|| {
            if payload.len() >= 32 {
                let fixed_password = trim_login_field(&payload[16..32]);
                (!fixed_password.is_empty()).then(|| fixed_password.to_vec())
            } else {
                None
            }
        })
        // 예전 스텁처럼 ID만 보내는 경우도 기본 계정(test/test)으로 계속 통과시킵니다.
        .unwrap_or_else(|| user_id.as_bytes().to_vec());

    Some((user_id, password))
}

/// 인메모리 계정 목록으로 로그인 자격 증명을 검사합니다.
fn authenticate_login(state: &Session, user_id: &str, password: &[u8]) -> u32 {
    let Some(stored_password) = state.profile.get(user_id) else {
        return LOGIN_ERR_NO_ID;
    };

    if stored_password.eq("test") && !password.is_empty() {
        LOGIN_OK
    } else {
        LOGIN_ERR_BAD_PASSWORD
    }
}

/// 공통 시스템 채널의 버전 협상과 로그인 절차를 처리합니다.
pub(crate) fn handle_system(pkt: &ChannelPacket, ch: u16, state: &mut Session) -> Vec<u8> {
    match pkt.sub_type {
        0x01 => {
            crate::emu_socket_log!("[SYS] 버전 핸드셰이크 요청 → 버전 54 응답");
            let mut payload = Vec::new();
            payload.extend_from_slice(&protocol::write_u32(54));

            protocol::create_app_packet(ch, 0x64, 0x01, &payload)
        }
        0x02 => {
            crate::emu_socket_log!("[SYS] 로그인 요청 수신");

            let (user_id, password) = match parse_login_credentials(&pkt.payload) {
                Some(credentials) => credentials,
                None => {
                    crate::emu_socket_log!("[SYS] 로그인 요청 파싱 실패");
                    state.info = None;

                    let mut login_payload = Vec::new();
                    login_payload.extend_from_slice(&protocol::write_u32(LOGIN_ERR_AUTH_FAILED));

                    return protocol::create_app_packet(ch, 0x64, 0x02, &login_payload);
                }
            };

            let login_result = authenticate_login(state, &user_id, &password);
            crate::emu_socket_log!("[SYS] 로그인 ID={} result={}", user_id, login_result);

            if login_result == LOGIN_OK {
                state.info = Some(build_default_session(&user_id));
            } else {
                state.info = None;
            }

            let mut login_payload = Vec::new();
            login_payload.extend_from_slice(&protocol::write_u32(login_result));

            protocol::create_app_packet(ch, 0x64, 0x02, &login_payload)
        }
        sub => {
            crate::emu_socket_log!(
                "[SYS] 미구현 sub=0x{:02x} payload={}",
                sub,
                hex::encode(&pkt.payload)
            );
            Vec::new()
        }
    }
}
