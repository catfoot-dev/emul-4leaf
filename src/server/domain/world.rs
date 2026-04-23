use crate::server::protocol::{ChannelPacket, create_auth_packet};

/// 월드맵 채널의 요청을 기록해 후속 역공학과 구현의 기준선으로 사용합니다.
pub(crate) fn handle_world_map(pkt: &ChannelPacket, _ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[WorldMap] sub=0x{:02x} payload={}B",
        pkt.sub_type,
        pkt.payload.len()
    );

    Vec::new()
}

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
