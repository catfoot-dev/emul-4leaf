use crate::server::{
    analysis::HandlerOutcome,
    protocol::{self, ProtocolPacket},
};

/// 약관 동의 다이얼로그 본문 문자열을 반환합니다.
fn get_terms_dialog_body() -> &'static [u8] {
    b"Please read and agree to the service terms before creating your account.\r\nSelect Agree to continue.\r\n\0"
}

/// 약관 동의 다이얼로그 응답 payload를 직렬화합니다.
fn build_terms_dialog_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    // 첫 DWORD는 성공/표시 플래그로 가정하고 1을 사용합니다.
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&protocol::write_string(b"4Leaf Terms Agreement\0"));
    payload.extend_from_slice(&protocol::write_string(get_terms_dialog_body()));
    payload
}

/// 약관 동의 다이얼로그 요청에 응답합니다.
pub(crate) fn handle_terms_dialog_request(
    pkt: &ProtocolPacket,
    ch: u16,
    response_main_type: u8,
) -> HandlerOutcome {
    let payload = build_terms_dialog_payload();

    HandlerOutcome {
        responses: vec![protocol::create_app_packet(
            ch,
            response_main_type,
            pkt.sub_type,
            &payload,
        )],
        phase_update: None,
    }
}
