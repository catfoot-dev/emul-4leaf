use crate::emu_socket_log;
use crate::server::protocol::DNetPacket;
use std::collections::HashMap;

/// 패킷 송신/수신 방향을 나타냄
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PacketDirection {
    /// 클라이언트 -> 서버 방향 (Send)
    Send,
    /// 서버 -> 클라이언트 방향 (Recv)
    Recv,
}

/// 에뮬레이터 내 TCP/UDP 통신을 캡처하여 시간, 방향, 내용을 기록하고 관리하는 모듈
pub struct PacketLogger {
    stream_buffers: HashMap<(PacketDirection, u32), Vec<u8>>,
    pub enabled: bool,
}

impl PacketLogger {
    pub fn new() -> Self {
        PacketLogger {
            stream_buffers: HashMap::new(),
            enabled: true,
        }
    }

    /// 주어진 방향, 소켓 ID, 파일 데이터를 로거에 추가하고 터미널 버퍼(`println!`)에도 16진수/ASCII 포맷으로 보기 좋게 출력
    ///
    /// # 인자
    /// * `direction`: `Send` 인지 `Recv` 인지 여부 (`PacketDirection`)
    /// * `socket_id`: 관련된 소켓 핸들 번호 식별자
    /// * `data`: 전송/수신된 바이트 슬라이스 (`&[u8]`)
    pub fn log(
        &mut self,
        direction: PacketDirection,
        socket_id: u32,
        data: &[u8],
        advance_stream: bool,
    ) {
        if !self.enabled {
            return;
        }

        self.write_frame_lines(direction, socket_id, data, advance_stream);
    }

    fn drain_complete_frames(
        &mut self,
        direction: PacketDirection,
        socket_id: u32,
        data: &[u8],
        advance_stream: bool,
    ) -> Vec<Vec<u8>> {
        if !advance_stream || data.is_empty() {
            return Vec::new();
        }

        let buffer = self
            .stream_buffers
            .entry((direction, socket_id))
            .or_default();
        buffer.extend_from_slice(data);

        let mut frames = Vec::new();
        loop {
            if buffer.len() < 4 {
                break;
            }

            let header: [u8; 4] = match buffer[..4].try_into() {
                Ok(header) => header,
                Err(_) => break,
            };

            let Some((_channel_id, body_len)) = DNetPacket::parse_header(&header) else {
                buffer.drain(..1);
                continue;
            };

            let frame_len = 4 + body_len as usize;
            if buffer.len() < frame_len {
                break;
            }

            frames.push(buffer.drain(..frame_len).collect());
        }

        frames
    }

    fn write_frame_lines(
        &mut self,
        direction: PacketDirection,
        socket_id: u32,
        data: &[u8],
        advance_stream: bool,
    ) {
        for frame in self.drain_complete_frames(direction, socket_id, data, advance_stream) {
            let mut hex = String::from(format!("[SERVER] {:#04x}: ", 0));
            for (i, b) in frame.iter().enumerate() {
                if i > 0 && i % 32 == 0 {
                    hex.push_str(&format!("\n[SERVER] {:#04x}: ", i));
                }
                hex.push_str(&format!("{:02x} ", b));
            }
            emu_socket_log!("{}", hex);
        }
    }
}
