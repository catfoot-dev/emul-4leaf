use crate::server::protocol::{ControlMessage, DNetPacket, ProtocolPacket};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

const MAX_PACKET_HISTORY: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAppFrame {
    channel_id: u16,
    main_type: u8,
    sub_type: u8,
    payload: Vec<u8>,
}

/// 패킷 송신/수신 방향을 나타냄
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PacketDirection {
    /// 클라이언트 -> 서버 방향 (Send)
    Send,
    /// 서버 -> 클라이언트 방향 (Recv)
    Recv,
}

/// 디버깅 및 분석용으로 네트워크 층에서 가로챈 패킷 하나의 정보를 담음
#[derive(Debug, Clone)]
pub struct CapturedPacket {
    /// 패킷 로거 시작 이후로 흐른 시간 (밀리초 단위)
    #[allow(dead_code)]
    pub timestamp_ms: u64,
    /// 해당 패킷이 송신된 것인지, 수신된 것인지 방향 식별
    pub direction: PacketDirection,
    /// 패킷을 보낸 혹은 받은 소켓의 핸들/아이디
    pub socket_id: u32,
    /// 메모리에 복사된 실제 패킷 데이터 페이로드
    pub data: Vec<u8>,
}

/// 에뮬레이터 내 TCP/UDP 통신을 캡처하여 시간, 방향, 내용을 기록하고 관리하는 모듈
pub struct PacketLogger {
    start_time: Instant,
    packets: VecDeque<CapturedPacket>,
    stream_buffers: HashMap<(PacketDirection, u32), Vec<u8>>,
    pub enabled: bool,
}

impl PacketLogger {
    /// 새로운 패킷 로거의 인스턴스를 생성하고 기록 시작 시간을 `Instant::now()`로 선언
    pub fn new() -> Self {
        PacketLogger {
            start_time: Instant::now(),
            packets: VecDeque::new(),
            stream_buffers: HashMap::new(),
            enabled: crate::should_write_capture_files(),
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

        let timestamp_ms = self.start_time.elapsed().as_millis() as u64;
        let mirrors_previous_send =
            direction == PacketDirection::Recv && self.mirrors_previous_send(socket_id, data);
        if crate::debug::should_send_debug_messages() {
            let dir_str = match direction {
                PacketDirection::Send => "SEND",
                PacketDirection::Recv => "RECV",
            };

            // 디버그 창이 열려 있을 때만 16진수/ASCII 덤프를 생성해 핫패스 비용을 줄입니다.
            let hex = data
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ");
            crate::emu_socket_log!(
                "[{}] t={}ms sock={} len={} | {}",
                dir_str,
                timestamp_ms,
                socket_id,
                data.len(),
                hex
            );
        }

        self.write_capture_line(
            timestamp_ms,
            direction,
            socket_id,
            data,
            mirrors_previous_send,
        );
        self.write_frame_lines(timestamp_ms, direction, socket_id, data, advance_stream);

        // 캡처 이력은 고정 크기로 유지해 장시간 실행 시 메모리 사용량이 계속 커지지 않게 합니다.
        if self.packets.len() >= MAX_PACKET_HISTORY {
            self.packets.pop_front();
        }
        self.packets.push_back(CapturedPacket {
            timestamp_ms,
            direction,
            socket_id,
            data: data.to_vec(),
        });
    }

    fn parse_single_app_frame(data: &[u8]) -> Option<ParsedAppFrame> {
        if data.len() < 6 {
            return None;
        }

        let header: [u8; 4] = data[..4].try_into().ok()?;
        let (channel_id, body_len) = DNetPacket::parse_header(&header)?;
        if channel_id == 0 || data.len() != 4 + body_len as usize {
            return None;
        }

        let body = &data[4..];
        if (body.len() >= 8 && body[..4] == [0, 0, 0, 0])
            || (body.len() >= 4 && body[1] == 0 && body[2] == 0 && body[3] == 0)
        {
            return None;
        }

        let packet = ProtocolPacket::from_bytes(body)?;
        Some(ParsedAppFrame {
            channel_id,
            main_type: packet.main_type,
            sub_type: packet.sub_type,
            payload: packet.payload,
        })
    }

    fn summarize_dnet_frames(data: &[u8]) -> String {
        let mut cursor = 0usize;
        let mut parts = Vec::new();

        while cursor + 4 <= data.len() {
            let header: [u8; 4] = match data[cursor..cursor + 4].try_into() {
                Ok(value) => value,
                Err(_) => break,
            };
            let Some((channel_id, body_len)) = DNetPacket::parse_header(&header) else {
                break;
            };
            let frame_len = 4 + body_len as usize;
            if cursor + frame_len > data.len() {
                parts.push(format!(
                    "dnet_partial ch={} expected={} available={}",
                    channel_id,
                    body_len,
                    data.len().saturating_sub(cursor + 4)
                ));
                cursor = data.len();
                break;
            }

            let body = &data[cursor + 4..cursor + frame_len];
            if channel_id == 0 {
                if let Some(ctrl) = ControlMessage::from_bytes(body) {
                    parts.push(format!("ctrl msg={} ch={}", ctrl.msg_type, ctrl.channel_id));
                } else {
                    parts.push(format!("ctrl malformed len={}", body.len()));
                }
            } else if body.len() >= 8 && body[..4] == [0, 0, 0, 0] {
                let msg_id = u32::from_le_bytes(body[4..8].try_into().unwrap());
                parts.push(format!(
                    "raw ch={} handler=0 msg={} payload={}B {}",
                    channel_id,
                    msg_id,
                    body.len() - 8,
                    hex::encode(&body[8..])
                ));
            } else if body.len() >= 4 && body[1] == 0 && body[2] == 0 && body[3] == 0 {
                let msg_id = u32::from_le_bytes(body[..4].try_into().unwrap());
                parts.push(format!(
                    "raw ch={} msg={} payload={}B {}",
                    channel_id,
                    msg_id,
                    body.len() - 4,
                    hex::encode(&body[4..])
                ));
            } else if let Some(packet) = ProtocolPacket::from_bytes(body) {
                parts.push(format!(
                    "app ch={} main=0x{:02x} sub=0x{:02x} payload={}B {}",
                    channel_id,
                    packet.main_type,
                    packet.sub_type,
                    packet.payload.len(),
                    hex::encode(&packet.payload)
                ));
            } else {
                parts.push(format!(
                    "app ch={} malformed len={}",
                    channel_id,
                    body.len()
                ));
            }

            cursor += frame_len;
        }

        if parts.is_empty() {
            "raw/unparsed".to_string()
        } else if cursor < data.len() {
            format!(
                "{} | remainder={}B {}",
                parts.join(" | "),
                data.len() - cursor,
                hex::encode(&data[cursor..])
            )
        } else {
            parts.join(" | ")
        }
    }

    fn mirrors_previous_send(&self, socket_id: u32, data: &[u8]) -> bool {
        let Some(current) = Self::parse_single_app_frame(data) else {
            return false;
        };

        self.packets
            .iter()
            .rev()
            .find(|packet| {
                packet.socket_id == socket_id && packet.direction == PacketDirection::Send
            })
            .and_then(|packet| Self::parse_single_app_frame(&packet.data))
            .map(|previous| previous == current)
            .unwrap_or(false)
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
        timestamp_ms: u64,
        direction: PacketDirection,
        socket_id: u32,
        data: &[u8],
        advance_stream: bool,
    ) {
        if !crate::should_write_capture_files() {
            return;
        }

        let dir = match direction {
            PacketDirection::Send => "SEND",
            PacketDirection::Recv => "RECV",
        };

        for frame in self.drain_complete_frames(direction, socket_id, data, advance_stream) {
            let mirror_prev_send =
                direction == PacketDirection::Recv && self.mirrors_previous_send(socket_id, &frame);
            crate::append_capture_line(
                "frames.log",
                &format!(
                    "t={}ms dir={} sock={} mirror_prev_send={} hex={} summary={}",
                    timestamp_ms,
                    dir,
                    socket_id,
                    mirror_prev_send,
                    hex::encode(&frame),
                    Self::summarize_dnet_frames(&frame)
                ),
            );
        }
    }

    fn write_capture_line(
        &self,
        timestamp_ms: u64,
        direction: PacketDirection,
        socket_id: u32,
        data: &[u8],
        mirrors_previous_send: bool,
    ) {
        if !crate::should_write_capture_files() {
            return;
        }

        let dir = match direction {
            PacketDirection::Send => "SEND",
            PacketDirection::Recv => "RECV",
        };
        let summary = Self::summarize_dnet_frames(data);
        crate::append_capture_line(
            "packets.log",
            &format!(
                "t={}ms dir={} sock={} len={} mirror_prev_send={} hex={} summary={}",
                timestamp_ms,
                dir,
                socket_id,
                data.len(),
                mirrors_previous_send,
                hex::encode(data),
                summary
            ),
        );
    }

    /// 현재 로거에 기록된(캡처된) 총 패킷의 개수를 반환
    #[allow(dead_code)]
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    /// 현재 캡처된 모든 패킷 데이터의 읽기 전용 슬라이스를 반환
    #[allow(dead_code)]
    pub fn get_packets(&self) -> &VecDeque<CapturedPacket> {
        &self.packets
    }

    /// 여태까지 캡처한 전체 패킷의 개수(`Send`/`Recv` 별)와 총 바이트 크기를 계산하여 콘솔에 요약 정보로 출력
    #[allow(dead_code)]
    pub fn print_summary(&self) {
        let send_count = self
            .packets
            .iter()
            .filter(|p| p.direction == PacketDirection::Send)
            .count();
        let recv_count = self
            .packets
            .iter()
            .filter(|p| p.direction == PacketDirection::Recv)
            .count();
        let total_bytes: usize = self.packets.iter().map(|p| p.data.len()).sum();
        println!("=== Packet Summary ===");
        println!(
            "Total packets: {} (Send: {}, Recv: {})",
            self.packets.len(),
            send_count,
            recv_count
        );
        println!("Total bytes: {}", total_bytes);
        println!("======================\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol;

    #[test]
    fn packet_history_is_capped() {
        let mut logger = PacketLogger::new();
        logger.enabled = true;

        for i in 0..(MAX_PACKET_HISTORY + 8) {
            logger.log(PacketDirection::Recv, i as u32, &[1, 2, 3], true);
        }

        assert_eq!(logger.packet_count(), MAX_PACKET_HISTORY);
        assert_eq!(logger.get_packets().front().unwrap().socket_id, 8);
    }

    #[test]
    fn disabled_logger_does_not_store_packets() {
        let mut logger = PacketLogger::new();
        logger.enabled = false;
        logger.log(PacketDirection::Send, 1, &[0xaa], true);

        assert_eq!(logger.packet_count(), 0);
    }

    #[test]
    fn parser_extracts_single_app_frame_fields() {
        let frame = protocol::create_app_packet(3, 0xe0, 0x04, &[0x11, 0x22]);

        let parsed = PacketLogger::parse_single_app_frame(&frame).unwrap();

        assert_eq!(
            parsed,
            ParsedAppFrame {
                channel_id: 3,
                main_type: 0xe0,
                sub_type: 0x04,
                payload: vec![0x11, 0x22],
            }
        );
    }

    #[test]
    fn recv_can_be_classified_as_mirror_of_previous_send() {
        let mut logger = PacketLogger::new();
        logger.enabled = true;
        let send = protocol::create_app_packet(1, 0xe0, 0x04, &[0xaa, 0xbb]);
        let recv = protocol::create_app_packet(1, 0xe0, 0x04, &[0xaa, 0xbb]);

        logger.log(PacketDirection::Send, 7, &send, true);

        assert!(logger.mirrors_previous_send(7, &recv));
    }

    #[test]
    fn fragmented_recv_chunks_are_reassembled_into_single_frame() {
        let mut logger = PacketLogger::new();
        logger.enabled = true;
        let frame = protocol::create_app_packet(1, 0xe0, 0x04, &[0xaa, 0xbb]);

        let frames_a = logger.drain_complete_frames(PacketDirection::Recv, 11, &frame[..1], true);
        let frames_b = logger.drain_complete_frames(PacketDirection::Recv, 11, &frame[1..4], true);
        let frames_c = logger.drain_complete_frames(PacketDirection::Recv, 11, &frame[4..], true);

        assert!(frames_a.is_empty());
        assert!(frames_b.is_empty());
        assert_eq!(frames_c, vec![frame]);
    }

    #[test]
    fn raw_stage_frames_are_not_classified_as_app_frames() {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&8u32.to_le_bytes());
        let frame = DNetPacket::new(2, body).to_bytes();

        assert!(PacketLogger::parse_single_app_frame(&frame).is_none());
    }

    #[test]
    fn summary_recognizes_mainframe_raw_message_with_zero_handler() {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&9u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 16]);
        let frame = DNetPacket::new(1, body).to_bytes();

        assert_eq!(
            PacketLogger::summarize_dnet_frames(&frame),
            "raw ch=1 handler=0 msg=9 payload=16B 00000000000000000000000000000000"
        );
    }
}
