use std::time::Instant;

/// 패킷 송신/수신 방향을 나타냄
#[derive(Debug, Clone, Copy, PartialEq)]
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
    packets: Vec<CapturedPacket>,
    pub enabled: bool,
}

impl PacketLogger {
    /// 새로운 패킷 로거의 인스턴스를 생성하고 기록 시작 시간을 `Instant::now()`로 선언
    pub fn new() -> Self {
        PacketLogger {
            start_time: Instant::now(),
            packets: Vec::new(),
            enabled: true,
        }
    }

    /// 주어진 방향, 소켓 ID, 파일 데이터를 로거에 추가하고 터미널 버퍼(`emu_log!`)에도 16진수/ASCII 포맷으로 보기 좋게 출력
    ///
    /// # 인자
    /// - `direction`: `Send` 인지 `Recv` 인지 여부 (`PacketDirection`)
    /// - `socket_id`: 관련된 소켓 핸들 번호 식별자
    /// - `data`: 전송/수신된 바이트 슬라이스 (`&[u8]`)
    pub fn log(&mut self, direction: PacketDirection, socket_id: u32, data: &[u8]) {
        if !self.enabled {
            return;
        }

        let timestamp_ms = self.start_time.elapsed().as_millis() as u64;
        let dir_str = match direction {
            PacketDirection::Send => "SEND",
            PacketDirection::Recv => "RECV",
        };

        // Hex dump
        let hex = data
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");

        // ASCII dump (printable characters only)
        let ascii: String = data
            .iter()
            .map(|&b| {
                if (0x20..=0x7e).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();

        crate::emu_log!(
            "[PACKET][{}] t={:>8}ms | sock={:>5} | len={:>5} | {}",
            dir_str,
            timestamp_ms,
            socket_id,
            data.len(),
            hex
        );
        if !data.is_empty() {
            crate::emu_log!("[PACKET][{}] ASCII: {}", dir_str, ascii);
        }

        // 캡처 저장
        self.packets.push(CapturedPacket {
            timestamp_ms,
            direction,
            socket_id,
            data: data.to_vec(),
        });
    }

    /// 현재 로거에 기록된(캡처된) 총 패킷의 개수를 반환
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    /// 현재 캡처된 모든 패킷 데이터의 읽기 전용 슬라이스를 반환
    pub fn get_packets(&self) -> &[CapturedPacket] {
        &self.packets
    }

    /// 여태까지 캡처한 전체 패킷의 개수(`Send`/`Recv` 별)와 총 바이트 크기를 계산하여 콘솔에 요약 정보로 출력
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
        crate::emu_log!("\n=== Packet Summary ===");
        crate::emu_log!(
            "Total packets: {} (Send: {}, Recv: {})",
            self.packets.len(),
            send_count,
            recv_count
        );
        crate::emu_log!("Total bytes: {}", total_bytes);
        crate::emu_log!("======================\n");
    }
}
