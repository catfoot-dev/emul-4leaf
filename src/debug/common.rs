// =========================================================
// [데이터 구조] 통신용 프로토콜
// =========================================================

/// UI와 에뮬레이터 간 주고받는 현재 CPU(레지스터, 스택 등)의 상태 정보
#[derive(Debug, Clone)]
pub struct CpuContext {
    /// 9개의 범용 및 제어 레지스터 배열 (EAX, EBX, ECX, EDX, ESI, EDI, EBP, ESP, EIP 순)
    pub regs: [u32; 9],
    /// 스택 상단부 값들의 리스트 (주소, 값) 형태
    pub stack: Vec<(u32, u32)>,
    /// 다음에 실행될 명령어의 디스어셈블 텍스트
    pub next_instr: String,
}

/// 에뮬레이터 UI 창에서 사용자의 입력으로 트리거되어 백그라운드 에뮬레이터를 제어하는 커맨드들
pub enum DebugCommand {
    /// F10: 한 줄(명령어 하나)만 실행하고 멈춤
    Step,
    /// F5: 자동 연속 실행 모드
    Run,
    /// F5: 자동 연속 실행 중 멈춤(Step 모드로 전환)
    Pause,
    /// 내부/외부적인 요인에 의한 강제 에뮬레이션 종료
    Stop,
}

/// 에뮬레이터 코어(Win32 API)가 UI 스레드에 요청하는 창 조작 커맨드
pub enum UiCommand {
    /// 새로운 윈도우 창 생성 요청
    CreateWindow {
        /// 가상 HWND 핸들
        hwnd: u32,
        /// 창 제목
        title: String,
        /// 너비
        width: u32,
        /// 높이
        height: u32,
    },
    /// 특정 윈도우 창 파괴 요청
    DestroyWindow {
        /// 가상 HWND 핸들
        hwnd: u32,
    },
}
