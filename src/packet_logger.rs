use std::time::Instant;

/// 패킷 방향
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PacketDirection {
    Send,
    Recv,
}

/// 캡처된 패킷
#[derive(Debug, Clone)]
pub struct CapturedPacket {
    pub timestamp_ms: u64,
    pub direction: PacketDirection,
    pub socket_id: u32,
    pub data: Vec<u8>,
}

/// 패킷 로거
pub struct PacketLogger {
    start_time: Instant,
    packets: Vec<CapturedPacket>,
    pub enabled: bool,
}

impl PacketLogger {
    pub fn new() -> Self {
        PacketLogger {
            start_time: Instant::now(),
            packets: Vec::new(),
            enabled: true,
        }
    }

    /// 패킷 캡처 및 로그 출력
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
                if b >= 0x20 && b <= 0x7e {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();

        println!(
            "[PACKET][{}] t={:>8}ms | sock={:>5} | len={:>5} | {}",
            dir_str,
            timestamp_ms,
            socket_id,
            data.len(),
            hex
        );
        if data.len() > 0 {
            println!("[PACKET][{}] ASCII: {}", dir_str, ascii);
        }

        // 캡처 저장
        self.packets.push(CapturedPacket {
            timestamp_ms,
            direction,
            socket_id,
            data: data.to_vec(),
        });
    }

    /// 캡처된 패킷 수 반환
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    /// 모든 캡처된 패킷 반환
    pub fn get_packets(&self) -> &[CapturedPacket] {
        &self.packets
    }

    /// 패킷 요약 출력
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
        println!("\n=== Packet Summary ===");
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
