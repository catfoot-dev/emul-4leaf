use crate::server::protocol::create_auth_packet;

/// 채널 2 오픈 직후 기본 WorldMap stage를 만들 수 있게 임시 payload를 준비합니다.
pub(crate) fn build_provisional_worldmap_stage_payload() -> Vec<u8> {
    let mut payload = vec![0u8; 0x120];
    let stage_name = b"WorldMap";
    payload[..stage_name.len()].copy_from_slice(stage_name);
    payload
}

/// 채널 2에 전달하는 임시 WorldMap bootstrap raw 메시지를 생성합니다.
pub(crate) fn build_worldmap_response(ch: u16) -> Vec<u8> {
    create_auth_packet(ch, 0, 0, &build_provisional_worldmap_stage_payload())
}
