use crate::server::{
    analysis::{ChannelPhase, HandlerOutcome},
    protocol::{self, ProtocolPacket},
    state::{GameState, build_default_session, extract_null_terminated_string},
};

/// 공통 시스템 채널의 버전 협상과 로그인 절차를 처리합니다.
pub(crate) fn handle_system(
    pkt: &ProtocolPacket,
    ch: u16,
    state: &mut GameState,
) -> HandlerOutcome {
    match pkt.sub_type {
        0x01 => {
            crate::emu_socket_log!("[SYS] 버전 핸드셰이크 요청 → 버전 54 응답");
            let mut payload = Vec::new();
            payload.extend_from_slice(&protocol::write_u32(54));

            HandlerOutcome {
                responses: vec![protocol::create_app_packet(ch, 0x64, 0x01, &payload)],
                phase_update: Some(ChannelPhase::VersionNegotiated),
            }
        }
        0x02 => {
            crate::emu_socket_log!("[SYS] 로그인 요청 수신");

            let user_id = if !pkt.payload.is_empty() {
                extract_null_terminated_string(&pkt.payload)
            } else {
                "test".to_string()
            };
            crate::emu_socket_log!("[SYS] 로그인 ID={}", user_id);

            state.session = Some(build_default_session(&user_id));

            let mut login_payload = Vec::new();
            login_payload.extend_from_slice(&protocol::write_u32(0));

            HandlerOutcome {
                responses: vec![protocol::create_app_packet(ch, 0x64, 0x02, &login_payload)],
                phase_update: Some(ChannelPhase::LoginAccepted),
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
            }
        }
    }
}
