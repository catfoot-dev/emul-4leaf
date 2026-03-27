// ─────────────────────────────────────────────────────────────────────────────
// 전송 계층: DNet 패킷
// ─────────────────────────────────────────────────────────────────────────────

/// DNet 전송 계층 패킷
///
/// 와이어 포맷: [channel_id: u16 LE][body_len: u16 LE][body: body_len bytes]
///
/// 클라이언트(TConnection::OnReceived) 유효성 규칙:
///   - channel_id: 0..=15
///   - body_len: 0..=0x1FFC (8188)
///   - channel_id == 0 이면 body_len은 반드시 4 (제어 메시지)
#[derive(Debug, Clone)]
pub struct DNetPacket {
    pub channel_id: u16,
    pub body: Vec<u8>,
}

impl DNetPacket {
    pub fn new(channel_id: u16, body: Vec<u8>) -> Self {
        Self { channel_id, body }
    }

    /// 4바이트 헤더에서 (channel_id, body_len)을 파싱하고 유효성을 검사합니다.
    pub fn parse_header(buf: &[u8; 4]) -> Option<(u16, u16)> {
        let channel_id = u16::from_le_bytes([buf[0], buf[1]]);
        let body_len   = u16::from_le_bytes([buf[2], buf[3]]);
        if channel_id > 15 || body_len > 0x1FFC { return None; }
        if channel_id == 0 && body_len != 4      { return None; }
        Some((channel_id, body_len))
    }

    /// 패킷을 와이어 바이트로 직렬화합니다.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.body.len());
        buf.extend_from_slice(&self.channel_id.to_le_bytes());
        buf.extend_from_slice(&(self.body.len() as u16).to_le_bytes());
        buf.extend_from_slice(&self.body);
        buf
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 애플리케이션 계층: 채널 1-15 본문 패킷
// ─────────────────────────────────────────────────────────────────────────────

/// 애플리케이션 계층 패킷 (DNetPacket.body 내부, 채널 1-15 전용)
///
/// 포맷: [main_type: u8][sub_type: u8][payload...]
#[derive(Debug, Clone)]
pub struct ProtocolPacket {
    pub main_type: u8,
    pub sub_type: u8,
    pub payload: Vec<u8>,
}

impl ProtocolPacket {
    /// DNet 본문 바이트에서 파싱합니다.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 2 { return None; }
        Some(Self {
            main_type: data[0],
            sub_type:  data[1],
            payload:   data[2..].to_vec(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 제어 계층: 채널 0 제어 메시지
// ─────────────────────────────────────────────────────────────────────────────

/// TConnection::ProcessControlMessage 역공학으로 확인된 메시지 타입 상수
///
/// SendControlMessage(this, msg_type, channel_id):
///   HIWORD(a2) = channel_id → 4바이트 본문 [msg_type LE][channel_id LE]
pub const CTRL_OPEN:      u16 = 1; // 클라이언트 → 서버: 채널 N 열기 요청
pub const CTRL_OPEN_ACK:  u16 = 2; // 서버 → 클라이언트: 채널 N 수락
pub const CTRL_CLOSE:     u16 = 3; // 양방향: 채널 N 거절/종료
pub const CTRL_CLOSE_ACK: u16 = 4; // 양방향: 채널 N 종료 확인

/// 채널 0 제어 메시지 본문 (항상 4바이트)
///
/// 와이어 포맷: [msg_type: u16 LE][channel_id: u16 LE]
#[derive(Debug, Clone)]
pub struct ControlMessage {
    pub msg_type:   u16, // CTRL_OPEN / CTRL_OPEN_ACK / CTRL_CLOSE / CTRL_CLOSE_ACK
    pub channel_id: u16, // 대상 채널 번호 (1-15)
}

impl ControlMessage {
    /// 채널 0 본문 4바이트에서 파싱합니다.
    pub fn from_bytes(body: &[u8]) -> Option<Self> {
        if body.len() < 4 { return None; }
        Some(Self {
            msg_type:   u16::from_le_bytes([body[0], body[1]]),
            channel_id: u16::from_le_bytes([body[2], body[3]]),
        })
    }

    /// DNet 채널 0 패킷으로 직렬화합니다.
    pub fn to_wire(&self) -> Vec<u8> {
        let mut body = [0u8; 4];
        body[0..2].copy_from_slice(&self.msg_type.to_le_bytes());
        body[2..4].copy_from_slice(&self.channel_id.to_le_bytes());
        DNetPacket::new(0, body.to_vec()).to_bytes()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 패킷 팩토리
// ─────────────────────────────────────────────────────────────────────────────

/// 제어 메시지 생성 헬퍼
pub fn create_control_message(msg_type: u16, channel_id: u16) -> Vec<u8> {
    ControlMessage { msg_type, channel_id }.to_wire()
}

/// 디버깅용 16진수 덤프 문자열
pub fn hex_dump(label: &str, data: &[u8]) -> String {
    format!("{} ({}B): {}", label, data.len(), hex::encode(data))
}
