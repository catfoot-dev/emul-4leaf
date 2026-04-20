use crate::server::protocol::ChannelPacket;

/// 월드맵 채널의 요청을 기록해 후속 역공학과 구현의 기준선으로 사용합니다.
pub(crate) fn handle_world_map(pkt: &ChannelPacket, _ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[WorldMap] sub=0x{:02x} payload={}B",
        pkt.sub_type,
        pkt.payload.len()
    );

    Vec::new()
}
