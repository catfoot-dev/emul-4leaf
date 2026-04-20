use crate::server::protocol::ChannelPacket;

/// ChatTown Main 채널의 입장/퇴장/월드 연동 요청을 기록합니다.
pub(crate) fn handle_chat_town_main(pkt: &ChannelPacket, _ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[ChatTown Main] sub=0x{:02x} payload={}",
        pkt.sub_type,
        hex::encode(&pkt.payload)
    );

    Vec::new()
}

/// ChatTown Sub 채널의 대화/액션/아이템 사용 요청을 기록합니다.
pub(crate) fn handle_chat_town_sub(pkt: &ChannelPacket, _ch: u16) -> Vec<u8> {
    crate::emu_socket_log!(
        "[ChatTown Sub] sub=0x{:02x} payload={}",
        pkt.sub_type,
        hex::encode(&pkt.payload)
    );

    Vec::new()
}
