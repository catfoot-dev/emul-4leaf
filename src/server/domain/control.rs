use crate::server::protocol::DNetPacket;

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
