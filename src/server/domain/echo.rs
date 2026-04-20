use crate::server::protocol::{self, ChannelPacket};

/// 핑/KeepAlive 요청을 그대로 되돌려 보내 연결 생존 여부를 유지합니다.
pub(crate) fn handle_ping(pkt: &ChannelPacket, ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[PING] 핑 요청 수신 (sub=0x{:02x}, len={}) → 에코 응답",
        pkt.sub_type,
        pkt.payload.len()
    );

    protocol::create_app_packet(ch, pkt.main_type, pkt.sub_type, &pkt.payload)
}

/// 인벤토리/쪽지 계열 요청은 포맷이 확정될 때까지 보수적으로 에코합니다.
pub(crate) fn handle_inventory(pkt: &ChannelPacket, ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[Inventory] sub=0x{:02x} payload={}B {}",
        pkt.sub_type,
        pkt.payload.len(),
        hex::encode(&pkt.payload)
    );

    protocol::create_app_packet(ch, 0x80, pkt.sub_type, &pkt.payload)
}

/// 아직 구조가 정리되지 않은 `0xD4` 계열 요청을 원본 길이 그대로 에코합니다.
pub(crate) fn handle_main_type_d4(pkt: &ChannelPacket, ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[D4] sub=0x{:02x} payload={}B {}",
        pkt.sub_type,
        pkt.payload.len(),
        hex::encode(&pkt.payload)
    );

    protocol::create_app_packet(ch, 0xD4, pkt.sub_type, &pkt.payload)
}
