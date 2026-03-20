// =========================================================
// [데이터 구조] 통신용 프로토콜
// =========================================================
#[derive(Debug, Clone)]
pub struct CpuContext {
    pub regs: [u32; 9], // EAX, EBX, ECX, EDX, ESI, EDI, EBP, ESP, EIP
    pub stack: Vec<(u32, u32)>,
    pub next_instr: String,
}

pub enum DebugCommand {
    Step,
    Run,   // 자동 실행 모드 (F10 없이 계속 진행)
    Pause, // 스텝 모드로 복귀
    Stop,
}
