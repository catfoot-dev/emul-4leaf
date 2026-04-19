use std::collections::HashMap;

/// 가입된 사용자 한 명의 최소 계정 정보를 나타냅니다.
#[derive(Debug, Clone)]
pub(crate) struct UserRecord {
    #[allow(dead_code)]
    pub(crate) password: Vec<u8>,
}

/// 로그인 이후 클라이언트가 참조하는 세션 상태를 나타냅니다.
#[derive(Debug, Clone)]
pub(crate) struct SessionInfo {
    #[allow(dead_code)]
    pub(crate) user_id: String,
    pub(crate) nickname: Vec<u8>,
    pub(crate) character: u8,
    pub(crate) gp: u32,
    pub(crate) fp: u32,
}

/// 서버 에뮬레이터가 유지하는 인메모리 게임 상태입니다.
#[derive(Debug)]
pub(crate) struct GameState {
    pub(crate) users: HashMap<String, UserRecord>,
    pub(crate) session: Option<SessionInfo>,
    pub(crate) client_version_code: u32,
}

impl GameState {
    /// 테스트용 기본 계정과 초기 버전 코드를 포함한 상태를 생성합니다.
    pub(crate) fn new() -> Self {
        let mut users = HashMap::new();
        users.insert(
            "test".to_string(),
            UserRecord {
                password: b"test".to_vec(),
            },
        );

        Self {
            users,
            session: None,
            client_version_code: 0x400d04e0,
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
    }
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
