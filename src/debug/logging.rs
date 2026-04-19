//! # 전역 로깅 인프라
//!
//! 에뮬레이터 실행 중 발생하는 로그 메시지와 소켓 로그를 관리합니다.
//! 디버그 UI 버퍼, stderr 미러링, 캡처 파일 기록 기능을 제공합니다.

use std::collections::VecDeque;
use std::env;
use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, OnceLock};

use crate::debug::LOG_SCROLL_MAX;

/// 로그를 stderr에도 그대로 복제할지 여부를 반환합니다.
///
/// 디버그 창과 캡처 파일이 이미 있는 상태에서 stderr까지 동기 출력하면
/// 핫패스 I/O 비용이 커지므로, 명시적으로 요청한 경우에만 활성화합니다.
fn should_mirror_logs_to_stderr() -> bool {
    static STDERR_LOG_ENABLED: OnceLock<bool> = OnceLock::new();
    *STDERR_LOG_ENABLED.get_or_init(|| env::var("EMUL_STDERR_LOG").ok().as_deref() == Some("1"))
}

/// UI 스레드와 로그 출력 스레드 간 동기화를 위한 전역 로그 큐 형태의 버퍼입니다.
pub static LOG_BUFFER: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
/// 새로운 로그가 들어올 때 마다 증가하여 UI 리프레시 타이밍을 결정하는 카운터입니다.
pub static LOG_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 소켓/패킷 관련 로그 전용 버퍼입니다. 디버그 UI의 소켓 로그 패널에 표시됩니다.
pub static SOCKET_LOG_BUFFER: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
/// 소켓 로그 업데이트 여부를 알리는 카운터입니다.
pub static SOCKET_LOG_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// 로그 메시지 식별을 위한 전역 시퀀스 인덱스입니다.
pub static INDEX: AtomicUsize = AtomicUsize::new(0);

/// 전역 버퍼에 새로운 로그 메시지를 추가합니다.
/// 메시지 줄 수 제한(LOG_SCROLL_MAX)을 초과하면 가장 오래된 항목부터 삭제됩니다.
///
/// # 인자
/// * `msg`: 추가할 로그 텍스트
pub fn push_log(msg: String) {
    if !crate::debug::should_send_debug_messages() {
        return;
    }
    if should_mirror_logs_to_stderr() {
        println!("[EMU] {}", msg);
    }
    if let Some(buf) = LOG_BUFFER.get()
        && let Ok(mut b) = buf.lock()
    {
        for line in msg.lines() {
            b.push_back(line.to_string());
            if b.len() > LOG_SCROLL_MAX {
                b.pop_front();
            }
        }
        LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 소켓 전용 로그 버퍼에 메시지를 추가합니다. 최대 200줄까지 유지됩니다.
///
/// # 인자
/// * `msg`: 추가할 소켓 로그 텍스트
pub fn push_socket_log(msg: String) {
    if !crate::debug::should_send_debug_messages() {
        return;
    }
    // println!("[SOCK] {}", msg);
    if let Some(buf) = SOCKET_LOG_BUFFER.get()
        && let Ok(mut b) = buf.lock()
    {
        for line in msg.lines() {
            b.push_back(line.to_string());
            if b.len() > 200 {
                b.pop_front();
            }
        }
        SOCKET_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 에뮬레이터 구동을 위한 전역 로거 및 버퍼들을 초기화합니다.
pub fn init_logger() {
    if crate::debug::should_send_debug_messages() {
        let _ = LOG_BUFFER.set(Mutex::new(VecDeque::new()));
        let _ = SOCKET_LOG_BUFFER.set(Mutex::new(VecDeque::new()));
    }
}
