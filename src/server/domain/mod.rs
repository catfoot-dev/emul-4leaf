//! 기능 추가와 제거가 파일 단위로 가능하도록 패킷 처리기를 도메인별로 분리합니다.

pub(crate) mod auth;
pub(crate) mod chat;
pub(crate) mod control;
pub(crate) mod echo;
pub(crate) mod system;
pub(crate) mod world;

use crate::server::{protocol::ChannelPacket, session::Session};

/// MainType 기준으로 적절한 도메인 처리기에 패킷을 위임합니다.
pub(crate) fn dispatch_packet(
    pkt: &ChannelPacket,
    channel_id: u16,
    state: &mut Session,
) -> Vec<u8> {
    match pkt.main_type {
        0x0A => world::handle_world_map(pkt, channel_id),
        0x0B => chat::handle_chat_town_main(pkt, channel_id),
        0x0C => chat::handle_chat_town_sub(pkt, channel_id),
        0x80 => echo::handle_inventory(pkt, channel_id),
        0xD4 => echo::handle_main_type_d4(pkt, channel_id),
        0x64 => system::handle_system(pkt, channel_id, state),
        0x68 => echo::handle_ping(pkt, channel_id),
        other => {
            crate::emu_socket_log!("[WARN] 미구현 MainType=0x{:02x}", other);
            Vec::new()
        }
    }
}
