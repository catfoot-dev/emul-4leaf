//! # 진단 추적 유틸리티
//!
//! 비결정적 멈춤 원인 분석을 위해 환경 변수로만 켜지는 경량 추적 기능을 제공합니다.

use crate::{
    dll::win32::{StackCleanup, Win32Context},
    helper::UnicornHelper,
};
use std::{
    collections::{HashSet, VecDeque},
    env,
    io::Seek,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};
use unicorn_engine::{RegisterX86, Unicorn};

const RECENT_LIMIT: usize = 16;
const LIME_GIF_RVA_START: u32 = 0x0000_b000;
const LIME_GIF_RVA_END: u32 = 0x0000_ca80;
const LIME_GIF_HOT_RVA_START: u32 = 0x0000_bd80;
const LIME_GIF_HOT_RVA_END: u32 = 0x0000_be20;
const LIME_GIF_OUTPUT_CURSOR_RVA: u64 = 0x0004_3450;
const LIME_GIF_OUTPUT_LIMIT_RVA: u64 = 0x0004_1450;
const GIF_STALL_REPORT_EVERY: usize = 16;
const GIF_STALL_ABORT_AFTER: usize = 64;
const DETERMINISTIC_TICK_STEP_MS: u32 = 16;

static TRACE_STATE: OnceLock<Mutex<TraceState>> = OnceLock::new();
static DETERMINISTIC_TICKS: AtomicU32 = AtomicU32::new(0);

#[derive(Default)]
struct TraceState {
    recent_hooks: VecDeque<String>,
    recent_files: VecDeque<String>,
    heap_guard_hits: HashSet<String>,
}

/// GIF 로딩 진단 모드가 켜져 있는지 반환합니다.
pub(crate) fn trace_gif_enabled() -> bool {
    env::var("EMUL_TRACE_GIF").ok().as_deref() == Some("1")
}

/// GIF stall 감지 시 에뮬레이터 루프를 중단할지 반환합니다.
pub(crate) fn gif_stall_break_enabled() -> bool {
    env::var("EMUL_GIF_STALL_BREAK").ok().as_deref() == Some("1")
}

/// 재현성 확인용 결정적 실행 모드가 켜져 있는지 반환합니다.
pub(crate) fn deterministic_enabled() -> bool {
    env::var("EMUL_DETERMINISTIC").ok().as_deref() == Some("1")
}

/// 결정적 모드에서 사용할 가상 밀리초 값을 반환합니다.
pub(crate) fn virtual_millis(start_time: Instant) -> u32 {
    if deterministic_enabled() {
        DETERMINISTIC_TICKS.fetch_add(DETERMINISTIC_TICK_STEP_MS, Ordering::SeqCst)
    } else {
        start_time.elapsed().as_millis() as u32
    }
}

/// 결정적 모드에서 사용할 실행 quantum을 반환합니다.
pub(crate) fn emulator_quantum(default_quantum: usize) -> usize {
    if deterministic_enabled() {
        20_000
    } else {
        default_quantum
    }
}

fn state() -> &'static Mutex<TraceState> {
    TRACE_STATE.get_or_init(|| Mutex::new(TraceState::default()))
}

fn trace_line(message: &str) {
    if trace_gif_enabled() {
        eprintln!("[GIF_TRACE] {message}");
    }
}

fn push_recent(queue: &mut VecDeque<String>, value: String) {
    queue.push_back(value);
    while queue.len() > RECENT_LIMIT {
        queue.pop_front();
    }
}

fn format_cleanup(cleanup: StackCleanup) -> String {
    match cleanup {
        StackCleanup::Caller => String::from("caller"),
        StackCleanup::Callee(count) => format!("callee(args={count},bytes={})", count * 4),
    }
}

fn format_hook_trace(
    import_func: &str,
    cleanup: StackCleanup,
    return_value: Option<i32>,
    esp_before: u64,
    esp_after: u64,
    eip_after: u64,
    ret_before: u32,
    retry: bool,
) -> String {
    format!(
        "{} cleanup={} ret={:?} retry={} esp={:#x}->{:#x} eip_after={:#x} guest_ret={:#x}",
        import_func,
        format_cleanup(cleanup),
        return_value,
        retry,
        esp_before,
        esp_after,
        eip_after,
        ret_before
    )
}

/// 최근 Win32 hook 반환 상태를 기록합니다.
pub(crate) fn record_hook_trace(
    import_func: &str,
    cleanup: StackCleanup,
    return_value: Option<i32>,
    esp_before: u64,
    esp_after: u64,
    eip_after: u64,
    ret_before: u32,
    retry: bool,
) {
    if !trace_gif_enabled() {
        return;
    }
    let line = format_hook_trace(
        import_func,
        cleanup,
        return_value,
        esp_before,
        esp_after,
        eip_after,
        ret_before,
        retry,
    );
    if let Ok(mut trace) = state().lock() {
        push_recent(&mut trace.recent_hooks, line);
    }
}

/// 입력 바이트 검증용 체크섬을 계산합니다.
pub(crate) fn trace_checksum(data: &[u8]) -> u32 {
    data.iter().fold(0x811c_9dc5u32, |hash, byte| {
        hash ^ u32::from(*byte).wrapping_mul(0x0100_0193)
    })
}

fn should_trace_file(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".pak")
}

fn format_file_trace(
    op: &str,
    handle: u32,
    path: &str,
    offset_before: Option<u64>,
    offset_after: Option<u64>,
    requested: usize,
    actual: usize,
    data: &[u8],
) -> String {
    format!(
        "{} handle={:#x} path={} off={:?}->{:?} req={} actual={} checksum={:#010x}",
        op,
        handle,
        path,
        offset_before,
        offset_after,
        requested,
        actual,
        trace_checksum(data)
    )
}

/// `.pak` 파일 I/O 상태를 기록합니다.
pub(crate) fn record_file_io(
    op: &str,
    handle: u32,
    path: &str,
    offset_before: Option<u64>,
    offset_after: Option<u64>,
    requested: usize,
    actual: usize,
    data: &[u8],
) {
    if !trace_gif_enabled() || !should_trace_file(path) {
        return;
    }
    let line = format_file_trace(
        op,
        handle,
        path,
        offset_before,
        offset_after,
        requested,
        actual,
        data,
    );
    trace_line(&line);
    if let Ok(mut trace) = state().lock() {
        push_recent(&mut trace.recent_files, line);
    }
}

/// 힙 할당을 진단 로그로 기록합니다.
pub(crate) fn record_heap_alloc(addr: u64, size: usize) {
    if trace_gif_enabled() {
        trace_line(&format!("HEAP_ALLOC addr={:#x} size={}", addr, size));
    }
}

/// 힙 해제를 진단 로그로 기록합니다.
pub(crate) fn record_heap_free(addr: u32, size: Option<u32>, released: bool) {
    if trace_gif_enabled() {
        trace_line(&format!(
            "HEAP_FREE addr={:#x} size={:?} released={}",
            addr, size, released
        ));
    }
}

/// DIBSection 비트 버퍼 범위를 진단 로그로 기록합니다.
pub(crate) fn record_dib_section(
    hbmp: u32,
    bits_addr: u32,
    byte_len: u32,
    width: u32,
    height: u32,
    bit_count: u16,
    stride: u32,
) {
    if trace_gif_enabled() {
        trace_line(&format!(
            "DIB_SECTION hbmp={:#x} bits={:#x} bytes={} size={}x{} bpp={} stride={}",
            hbmp, bits_addr, byte_len, width, height, bit_count, stride
        ));
    }
}

fn heap_guard_message(addr: u64, size: usize, value: i64) -> String {
    format!(
        "HEAP_GUARD unallocated write addr={:#x} size={} value={:#x}",
        addr, size, value
    )
}

/// 할당되지 않은 힙 범위 쓰기를 중복을 줄여 기록합니다.
pub(crate) fn record_heap_guard_write(addr: u64, size: usize, value: i64) {
    if !trace_gif_enabled() {
        return;
    }
    let key = format!("{:#x}:{}", addr & !0xf, size);
    if let Ok(mut trace) = state().lock()
        && trace.heap_guard_hits.insert(key)
    {
        trace_line(&heap_guard_message(addr, size, value));
    }
}

fn lime_gif_location(uc: &Unicorn<Win32Context>, eip: u64) -> Option<(u64, u32)> {
    let ctx = uc.get_data();
    let modules = ctx.dll_modules.lock().ok()?;
    let lime = modules
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Lime.dll"))
        .map(|(_, dll)| dll)?;
    let base = lime.base_addr;
    let end = base.saturating_add(lime.size);
    if eip < base || eip >= end {
        return None;
    }
    let rva = eip.saturating_sub(base) as u32;
    (LIME_GIF_RVA_START..=LIME_GIF_RVA_END)
        .contains(&rva)
        .then_some((base, rva))
}

fn is_lime_gif_hot_rva(rva: u32) -> bool {
    (LIME_GIF_HOT_RVA_START..=LIME_GIF_HOT_RVA_END).contains(&rva)
}

fn gif_lzw_output_overflow(uc: &Unicorn<Win32Context>, lime_base: u64) -> Option<(u32, u32, u32)> {
    let cursor = uc.read_u32(lime_base + LIME_GIF_OUTPUT_CURSOR_RVA);
    let limit_addr = lime_base.saturating_add(LIME_GIF_OUTPUT_LIMIT_RVA) as u32;
    gif_lzw_output_limit_exceeded(limit_addr, cursor).map(|overflow| (limit_addr, cursor, overflow))
}

fn gif_lzw_output_limit_exceeded(limit_addr: u32, cursor: u32) -> Option<u32> {
    (cursor > limit_addr).then(|| cursor - limit_addr)
}

fn format_lzw_overflow(overflow: Option<(u32, u32, u32)>) -> String {
    if let Some((limit_addr, cursor, bytes)) = overflow {
        format!(
            "limit_addr={:#x}, cursor={:#x}, overflow={}",
            limit_addr, cursor, bytes
        )
    } else {
        String::from("limit_addr=<none>, cursor=<none>, overflow=0")
    }
}

fn recent_summary(queue: &VecDeque<String>) -> String {
    if queue.is_empty() {
        String::from("<empty>")
    } else {
        queue.iter().cloned().collect::<Vec<_>>().join(" | ")
    }
}

fn current_files_summary(uc: &mut Unicorn<Win32Context>) -> String {
    let mut files = match uc.get_data().files.lock() {
        Ok(files) => files,
        Err(_) => return String::from("<lock-failed>"),
    };
    if files.is_empty() {
        return String::from("<none>");
    }
    files
        .iter_mut()
        .map(|(handle, state)| {
            let pos = state.file.stream_position().ok();
            format!("{:#x}:{}@{:?}", handle, state.path, pos)
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn register_summary(uc: &Unicorn<Win32Context>) -> String {
    let regs = [
        ("EAX", RegisterX86::EAX),
        ("EBX", RegisterX86::EBX),
        ("ECX", RegisterX86::ECX),
        ("EDX", RegisterX86::EDX),
        ("ESI", RegisterX86::ESI),
        ("EDI", RegisterX86::EDI),
        ("EBP", RegisterX86::EBP),
        ("ESP", RegisterX86::ESP),
        ("EIP", RegisterX86::EIP),
    ];
    regs.iter()
        .map(|(name, reg)| format!("{name}={:#x}", uc.reg_read(*reg).unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn stack_summary(uc: &Unicorn<Win32Context>) -> String {
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    (0..8)
        .map(|i| {
            let addr = esp + i * 4;
            format!("{:#x}:{:#x}", addr, uc.read_u32(addr))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// GIF 루프 stall 진단 상태입니다.
pub(crate) struct GifStallDetector {
    recent_eips: VecDeque<u32>,
    hot_loop_count: usize,
    last_report_count: usize,
}

impl GifStallDetector {
    /// 새 GIF stall 감지기를 생성합니다.
    pub(crate) fn new() -> Self {
        Self {
            recent_eips: VecDeque::new(),
            hot_loop_count: 0,
            last_report_count: 0,
        }
    }

    /// 현재 EIP를 관찰하고 stall 중단 사유가 있으면 반환합니다.
    pub(crate) fn observe(
        &mut self,
        uc: &mut Unicorn<Win32Context>,
        eip: u64,
        quantum: usize,
    ) -> Option<String> {
        let Some((lime_base, rva)) = lime_gif_location(uc, eip) else {
            self.hot_loop_count = 0;
            return None;
        };

        self.recent_eips.push_back(eip as u32);
        while self.recent_eips.len() > RECENT_LIMIT {
            self.recent_eips.pop_front();
        }

        if is_lime_gif_hot_rva(rva) {
            self.hot_loop_count = self.hot_loop_count.saturating_add(1);
        } else {
            self.hot_loop_count = 0;
        }

        let lzw_overflow = is_lime_gif_hot_rva(rva)
            .then(|| gif_lzw_output_overflow(uc, lime_base))
            .flatten();

        if trace_gif_enabled()
            && self.hot_loop_count >= GIF_STALL_REPORT_EVERY
            && self.hot_loop_count - self.last_report_count >= GIF_STALL_REPORT_EVERY
        {
            self.last_report_count = self.hot_loop_count;
            emit_gif_stall_report(
                uc,
                eip,
                rva,
                self.hot_loop_count,
                quantum,
                &self.recent_eips,
            );
            if let Some((limit_addr, cursor, overflow)) = lzw_overflow {
                trace_line(&format!(
                    "GIF_LZW_STACK_OVERFLOW limit_addr={:#x} cursor={:#x} overflow={}",
                    limit_addr, cursor, overflow
                ));
            }
            if gif_stall_break_enabled() {
                return Some(format!(
                    "GIF stall break requested at {:#x} (rva={:#x}, repeat={})",
                    eip, rva, self.hot_loop_count
                ));
            }
        }

        if self.hot_loop_count >= GIF_STALL_ABORT_AFTER {
            if trace_gif_enabled() {
                emit_gif_stall_report(
                    uc,
                    eip,
                    rva,
                    self.hot_loop_count,
                    quantum,
                    &self.recent_eips,
                );
            }
            return Some(format!(
                "GIF LZW hot loop did not make progress at {:#x} (rva={:#x}, repeat={}, {})",
                eip,
                rva,
                self.hot_loop_count,
                format_lzw_overflow(lzw_overflow)
            ));
        }

        None
    }
}

fn emit_gif_stall_report(
    uc: &mut Unicorn<Win32Context>,
    eip: u64,
    rva: u32,
    repeat: usize,
    quantum: usize,
    recent_eips: &VecDeque<u32>,
) {
    let recent_eips = recent_eips
        .iter()
        .map(|eip| format!("{:#x}", eip))
        .collect::<Vec<_>>()
        .join(" ");
    let (recent_hooks, recent_files) = if let Ok(trace) = state().lock() {
        (
            recent_summary(&trace.recent_hooks),
            recent_summary(&trace.recent_files),
        )
    } else {
        (String::from("<lock-failed>"), String::from("<lock-failed>"))
    };
    trace_line(&format!(
        "GIF_STALL eip={:#x} lime_rva={:#x} repeat={} quantum={} recent_eips=[{}]",
        eip, rva, repeat, quantum, recent_eips
    ));
    trace_line(&format!("GIF_STALL_REGS {}", register_summary(uc)));
    trace_line(&format!("GIF_STALL_STACK {}", stack_summary(uc)));
    trace_line(&format!("GIF_STALL_FILES {}", current_files_summary(uc)));
    trace_line(&format!("GIF_STALL_RECENT_FILES {}", recent_files));
    trace_line(&format!("GIF_STALL_RECENT_HOOKS {}", recent_hooks));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_changes_when_input_bytes_change() {
        assert_eq!(trace_checksum(b"abc"), trace_checksum(b"abc"));
        assert_ne!(trace_checksum(b"abc"), trace_checksum(b"abd"));
    }

    #[test]
    fn hook_trace_formats_cleanup_and_stack_state() {
        let line = format_hook_trace(
            "KERNEL32.dll!Sleep",
            StackCleanup::Callee(1),
            Some(0),
            0x5000,
            0x5004,
            0xf000_0004,
            0x401000,
            false,
        );
        assert!(line.contains("cleanup=callee(args=1,bytes=4)"));
        assert!(line.contains("esp=0x5000->0x5004"));
        assert!(line.contains("guest_ret=0x401000"));
    }

    #[test]
    fn file_trace_formats_offsets_and_checksum() {
        let line = format_file_trace(
            "fread",
            0x1001,
            "Resources/MainFrame.pak",
            Some(4),
            Some(12),
            8,
            8,
            b"12345678",
        );
        assert!(line.contains("off=Some(4)->Some(12)"));
        assert!(line.contains("req=8 actual=8"));
        assert!(line.contains("checksum=0x"));
    }

    #[test]
    fn heap_guard_message_contains_target_range() {
        let line = heap_guard_message(0x2000_0010, 4, 0x12);
        assert!(line.contains("addr=0x20000010"));
        assert!(line.contains("size=4"));
        assert!(line.contains("value=0x12"));
    }

    #[test]
    fn gif_lzw_output_limit_detects_overflow() {
        assert_eq!(gif_lzw_output_limit_exceeded(0x3000, 0x3000), None);
        assert_eq!(gif_lzw_output_limit_exceeded(0x3000, 0x3001), Some(1));
        assert_eq!(gif_lzw_output_limit_exceeded(0x3000, 0x1000), None);
    }
}
