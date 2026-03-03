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
    Stop,
}
