use std::collections::HashMap;

/// 채널이 서버 에뮬레이터 안에서 어느 해석 단계까지 진행됐는지 나타냅니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChannelPhase {
    OpenAccepted,
    BootstrapVersionSent,
    AwaitingMainFrameStageInfo,
    VersionNegotiated,
    LoginAccepted,
}

/// 채널별 프로토콜 분석 상태를 보관합니다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelAnalysisState {
    pub(crate) phase: ChannelPhase,
    pub(crate) post_bootstrap_client_packets: usize,
}

/// 도메인 처리기가 핸들러 루프에 돌려주는 공통 결과입니다.
#[derive(Debug)]
pub(crate) struct HandlerOutcome {
    pub(crate) responses: Vec<Vec<u8>>,
    pub(crate) phase_update: Option<ChannelPhase>,
}

/// Stage 채널에서 사용하는 raw 메시지 표현입니다.
#[derive(Debug, Clone)]
pub(crate) struct RawStagePacket {
    pub(crate) msg_id: u32,
    pub(crate) payload: Vec<u8>,
}

/// 분석 로그에 남길 단계 라벨을 사람이 읽기 쉬운 문자열로 반환합니다.
pub(crate) fn phase_label(phase: ChannelPhase) -> &'static str {
    match phase {
        ChannelPhase::OpenAccepted => "open-accepted",
        ChannelPhase::BootstrapVersionSent => "bootstrap-version-sent",
        ChannelPhase::AwaitingMainFrameStageInfo => "awaiting-mainframe-stage-info",
        ChannelPhase::VersionNegotiated => "version-negotiated",
        ChannelPhase::LoginAccepted => "login-accepted",
    }
}

/// 초기 오픈 직후보다 더 깊은 핸드셰이크 단계인지 판별합니다.
pub(crate) fn is_post_initial_handshake_phase(phase: ChannelPhase) -> bool {
    phase >= ChannelPhase::BootstrapVersionSent
}

/// 사람이 읽는 분석 로그와 캡처 파일에 동일한 라인을 남깁니다.
pub(crate) fn emit_protocol_analysis(line: &str) {
    crate::emu_socket_log!("[ANALYZE] {}", line);
    crate::append_capture_line("protocol_analysis.log", line);
}

/// raw stage 바디를 `msg_id + payload` 구조로 분해합니다.
pub(crate) fn raw_stage_packet_from_bytes(data: &[u8]) -> Option<RawStagePacket> {
    if data.len() < 4 {
        return None;
    }

    Some(RawStagePacket {
        msg_id: u32::from_le_bytes(data[..4].try_into().ok()?),
        payload: data[4..].to_vec(),
    })
}

/// 채널이 stage 전용 보조 채널인지 판별합니다.
fn is_stage_channel(channel_id: u16) -> bool {
    channel_id == 2
}

/// MainFrame이 다음 stage 정보를 기다리는 동안 열린 채널을 stage 채널로 승격할지 판별합니다.
pub(crate) fn should_promote_open_to_mainframe_stage(
    channel_id: u16,
    analysis_states: &HashMap<u16, ChannelAnalysisState>,
) -> bool {
    is_stage_channel(channel_id)
        && analysis_states.values().any(|state| {
            matches!(
                state.phase,
                ChannelPhase::BootstrapVersionSent | ChannelPhase::AwaitingMainFrameStageInfo
            )
        })
}

/// 현재 채널 바디를 일반 앱 패킷이 아니라 raw stage 패킷으로 해석해야 하는지 판별합니다.
pub(crate) fn should_parse_as_raw_stage_packet(
    channel_id: u16,
    analysis_states: &HashMap<u16, ChannelAnalysisState>,
    body: &[u8],
) -> bool {
    if body.len() < 4 {
        return false;
    }

    analysis_states
        .get(&channel_id)
        .map(|state| state.phase == ChannelPhase::AwaitingMainFrameStageInfo)
        .unwrap_or(false)
        && is_stage_channel(channel_id)
}
