use std::collections::HashMap;

/// 로그인 이후 클라이언트가 참조하는 세션 상태를 나타냅니다.
#[derive(Debug, Clone)]
pub(crate) struct SessionInfo {
    #[allow(dead_code)]
    pub(crate) user_id: String,
    pub(crate) nickname: Vec<u8>,
    pub(crate) character: u8,
    pub(crate) gp: u32,
    pub(crate) fp: u32,
    pub(crate) has_avatar: bool,
}

/// 서버 에뮬레이터가 유지하는 인메모리 게임 상태입니다.
#[derive(Debug)]
pub(crate) struct Session {
    pub(crate) profile: HashMap<String, String>,
    pub(crate) info: Option<SessionInfo>,
    #[allow(dead_code)]
    pub(crate) state: u32,
}

impl Session {
    /// 테스트용 기본 계정과 초기 버전 코드를 포함한 상태를 생성합니다.
    pub(crate) fn new() -> Self {
        let mut profile = HashMap::new();
        profile.insert("test".to_string(), "test".to_string());

        Self {
            profile,
            info: None,
            state: 0,
        }
    }
}

/// 가입 직후 바로 사용할 기본 세션 정보를 생성합니다.
pub(crate) fn build_default_session(user_id: &str) -> SessionInfo {
    let nickname = if user_id.len() > 22 {
        user_id.as_bytes()[..22].to_vec()
    } else {
        user_id.as_bytes().to_vec()
    };

    SessionInfo {
        user_id: user_id.to_string(),
        nickname,
        character: 0,
        gp: 1000,
        fp: 0,
        has_avatar: true,
    }
}

/// 아직 아바타가 없는 신규 계정용 기본 세션 정보를 생성합니다.
pub(crate) fn build_pending_avatar_session(user_id: &str) -> SessionInfo {
    let mut session = build_default_session(user_id);
    session.has_avatar = false;
    session
}

/// NUL 종료 바이트 배열에서 문자열을 안전하게 추출합니다.
pub(crate) fn extract_null_terminated_string(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).to_string()
}

/// 아바타 상세정보 블록을 클라이언트 기대 레이아웃에 맞춰 직렬화합니다.
pub(crate) fn build_avatar_detail_data(session: &SessionInfo) -> Vec<u8> {
    let mut data = Vec::with_capacity(272);

    // 캐릭터 식별자와 닉네임은 고정 오프셋을 사용하므로 먼저 채웁니다.
    data.push(session.character);

    let mut nick = [0u8; 22];
    let len = session.nickname.len().min(22);
    nick[..len].copy_from_slice(&session.nickname[..len]);
    data.extend_from_slice(&nick);

    // 나머지 장비/길드/인벤토리 필드는 아직 분석 중이므로 0으로 유지합니다.
    data.extend_from_slice(&[0u8; 5]);
    data.extend_from_slice(&[0u8; 32]);
    data.extend_from_slice(&[0u8; 24]);
    data.extend_from_slice(&[0u8; 24]);
    data.extend_from_slice(&session.gp.to_le_bytes());
    data.extend_from_slice(&session.fp.to_le_bytes());
    data.extend_from_slice(&[0u8; 16]);
    data.extend_from_slice(&[0u8; 140]);

    data
}

/// 아바타 선택 창 후보 레코드 1개를 `0x80` 바이트로 직렬화합니다.
pub(crate) fn build_avatar_dialog_record(session: &SessionInfo) -> Vec<u8> {
    let mut record = vec![0u8; 0x80];

    record[0] = session.character;
    record[0x15] = 0;

    let nick_len = session.nickname.len().min(22);
    record[0x54..0x54 + nick_len].copy_from_slice(&session.nickname[..nick_len]);

    let gp = session.gp.min(u16::MAX as u32) as u16;
    let fp = session.fp.min(u16::MAX as u32) as u16;
    record[0x70..0x72].copy_from_slice(&gp.to_le_bytes());
    record[0x72..0x74].copy_from_slice(&fp.to_le_bytes());

    record
}

/// 아바타 선택 창을 여는 후보 payload를 `0x431` 바이트 구조로 생성합니다.
pub(crate) fn build_avatar_dialog_bootstrap_payload(session: &SessionInfo) -> Vec<u8> {
    const HEADER_LEN: usize = 0x1c;
    const SUMMARY_LEN: usize = 0x1c;
    const FLAG_LEN: usize = 0x01;
    const BLOCK_LEN: usize = 0x1fc;
    const TOTAL_LEN: usize = HEADER_LEN + SUMMARY_LEN + FLAG_LEN + BLOCK_LEN + BLOCK_LEN;
    const BLOCK_A_OFFSET: usize = HEADER_LEN + SUMMARY_LEN + FLAG_LEN;
    const BLOCK_B_OFFSET: usize = BLOCK_A_OFFSET + BLOCK_LEN;
    const RECORD_OFFSET: usize = 0x7c;

    let mut payload = vec![0u8; TOTAL_LEN];

    let summary = &mut payload[HEADER_LEN..HEADER_LEN + SUMMARY_LEN];
    summary[0] = session.character;
    summary[4..8].copy_from_slice(&session.gp.to_le_bytes());
    summary[8..12].copy_from_slice(&session.fp.to_le_bytes());

    let (prefix, block_b_tail) = payload.split_at_mut(BLOCK_B_OFFSET);
    let block_a = &mut prefix[BLOCK_A_OFFSET..BLOCK_A_OFFSET + BLOCK_LEN];
    block_a[0] = 0;
    block_a[1] = u8::from(session.has_avatar);

    let block_b = &mut block_b_tail[..BLOCK_LEN];
    block_b[0] = 0;
    block_b[1] = u8::from(session.has_avatar);

    if session.has_avatar {
        let record = build_avatar_dialog_record(session);
        block_a[RECORD_OFFSET..RECORD_OFFSET + record.len()].copy_from_slice(&record);
        block_b[RECORD_OFFSET..RECORD_OFFSET + record.len()].copy_from_slice(&record);
    }

    payload
}
