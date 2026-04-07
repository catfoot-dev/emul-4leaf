//! # 전역 로깅 인프라
//!
//! 에뮬레이터 실행 중 발생하는 로그 메시지와 소켓 로그를 관리합니다.
//! 디버그 UI 버퍼, stderr 미러링, 캡처 파일 기록 기능을 제공합니다.

use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, OnceLock};

use crate::debug::LOG_SCROLL_MAX;

const CAPTURE_DIR: &str = "./docs/Capture";

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
    if !msg.is_empty() {
        append_capture_line("emu.log", &format!("[EMU] {}", msg));
    }

    if !crate::debug::should_send_debug_messages() {
        return;
    }
    if should_mirror_logs_to_stderr() {
        eprintln!("[EMU] {}", msg);
    }
    if let Some(buf) = LOG_BUFFER.get() {
        if let Ok(mut b) = buf.lock() {
            for line in msg.lines() {
                b.push_back(line.to_string());
                if b.len() > LOG_SCROLL_MAX {
                    b.pop_front();
                }
            }
            LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// 소켓 전용 로그 버퍼에 메시지를 추가합니다. 최대 200줄까지 유지됩니다.
///
/// # 인자
/// * `msg`: 추가할 소켓 로그 텍스트
pub fn push_socket_log(msg: String) {
    if !msg.is_empty() {
        append_capture_line("socket.log", &msg);
    }

    if !crate::debug::should_send_debug_messages() {
        return;
    }
    // eprintln!("[SOCK] {}", msg);
    if let Some(buf) = SOCKET_LOG_BUFFER.get() {
        if let Ok(mut b) = buf.lock() {
            for line in msg.lines() {
                b.push_back(line.to_string());
                if b.len() > 200 {
                    b.pop_front();
                }
            }
            SOCKET_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// 에뮬레이터 구동을 위한 전역 로거 및 버퍼들을 초기화합니다.
pub fn init_logger() {
    reset_capture_file("emu.log");
    reset_capture_file("socket.log");
    reset_capture_file("packets.log");
    reset_capture_file("frames.log");
    reset_capture_file("protocol_analysis.log");

    if crate::debug::should_send_debug_messages() {
        let _ = LOG_BUFFER.set(Mutex::new(VecDeque::new()));
        let _ = SOCKET_LOG_BUFFER.set(Mutex::new(VecDeque::new()));
    }
}

/// 현재 실행에서 캡처 파일을 기록할지 여부를 결정합니다.
pub(crate) fn should_write_capture_files() -> bool {
    if cfg!(test) {
        return false;
    }
    crate::debug::should_send_debug_messages()
        || env::var("EMUL_CAPTURE").ok().as_deref() == Some("1")
}

/// 캡처 파일 하나를 비우고 새 세션용으로 초기화합니다.
pub(crate) fn reset_capture_file(file_name: &str) {
    if !should_write_capture_files() {
        return;
    }

    let _ = fs::create_dir_all(CAPTURE_DIR);
    let path = Path::new(CAPTURE_DIR).join(file_name);
    let _ = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path);
}

/// 캡처 디렉터리의 지정된 파일 끝에 한 줄을 추가합니다.
static CAPTURE_FILES: OnceLock<Mutex<HashMap<String, std::io::BufWriter<fs::File>>>> =
    OnceLock::new();

/// 캡처 디렉터리의 지정된 파일 끝에 한 줄을 추가합니다.
pub(crate) fn append_capture_line(file_name: &str, line: &str) {
    if !should_write_capture_files() {
        return;
    }

    let map = CAPTURE_FILES.get_or_init(|| {
        let _ = fs::create_dir_all(CAPTURE_DIR);
        Mutex::new(HashMap::new())
    });
    if let Ok(mut files) = map.lock() {
        let writer = files.entry(file_name.to_string()).or_insert_with(|| {
            let path = Path::new(CAPTURE_DIR).join(file_name);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("Failed to open capture file");
            std::io::BufWriter::new(file)
        });
        let _ = writeln!(writer, "{}", line);
    }
}
