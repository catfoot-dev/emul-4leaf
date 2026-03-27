//! # 4Leaf Emulator Main Entry Point
//!
//! 이 모듈은 에뮬레이터의 생명주기를 관리하며, UI 스레드, 에뮬레이션 스레드,
//! 그리고 서버 스레드 간의 조율을 담당합니다. 전역 로그 버퍼와 실행 흐름 제어를 포함합니다.

mod debug;
mod server;
#[macro_use]
mod helper;
mod ui;
mod win32;

use helper::{SHARED_MEM_BASE, UnicornHelper};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, atomic::AtomicUsize};
use std::{
    any::Any,
    sync::mpsc::{Receiver, Sender, channel},
    thread,
};

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
    if let Some(buf) = LOG_BUFFER.get() {
        let mut b = buf.lock().unwrap();
        for line in msg.lines() {
            b.push_back(line.to_string());
            if b.len() > LOG_SCROLL_MAX {
                b.pop_front();
            }
        }
        LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 에뮬레이터 구동을 위한 전역 로거 및 버퍼들을 초기화합니다.
pub fn init_logger() {
    let _ = LOG_BUFFER.set(Mutex::new(VecDeque::new()));
    let _ = SOCKET_LOG_BUFFER.set(Mutex::new(VecDeque::new()));
}

/// 소켓 전용 로그 버퍼에 메시지를 추가합니다. 최대 200줄까지 유지됩니다.
///
/// # 인자
/// * `msg`: 추가할 소켓 로그 텍스트
pub fn push_socket_log(msg: String) {
    if let Some(buf) = SOCKET_LOG_BUFFER.get() {
        let mut b = buf.lock().unwrap();
        for line in msg.lines() {
            b.push_back(line.to_string());
            if b.len() > 200 {
                b.pop_front();
            }
        }
        SOCKET_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 에뮬레이션 로그 출력을 위한 매크로입니다.
/// 호출 시 전역 인덱스를 부여하고 특수 문자(\r, \n)를 이스케이프 처리하여 기록합니다.
#[macro_export]
macro_rules! emu_log {
    () => {
        $crate::push_log(String::new())
    };
    ($($arg:tt)*) => {{
        let index = $crate::INDEX.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let msg = std::format!("[{:#08x}] {}", index, std::format!($($arg)*)).replace("\r", "\\r").replace("\n", "\\n");
        $crate::push_log(msg)
    }};
}

#[macro_export]
macro_rules! emu_socket_log {
    () => {
        $crate::push_socket_log(String::new())
    };
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        $crate::push_socket_log(msg)
    }};
}

use unicorn_engine::{
    Unicorn,
    unicorn_const::{Arch, Mode},
};
use win32::Win32Context;

use crate::debug::LOG_SCROLL_MAX;
use crate::debug::common::{CpuContext, DebugCommand};
use crate::ui::UiCommand;

/// 어플리케이션 메인 진입점입니다.
/// 하위 시스템(로그, 에뮬레이션, 서버, UI)들을 초기화하고 실행합니다.
fn main() {
    init_logger();

    // 스레드 간 통신을 위한 채널 설정
    let (cmd_tx, cmd_rx) = channel::<DebugCommand>();
    let (state_tx, state_rx) = channel::<CpuContext>();
    let (ui_tx, ui_rx) = channel::<UiCommand>();
    let (splash_tx, splash_rx) = channel::<()>();

    // 1. Win32 에뮬레이션 상태 컨텍스트 생성 (Arc로 내부 상태 공유 가능)
    let context = Win32Context::new(Some(ui_tx.clone()));

    let context_for_emu = context.clone();
    // 1. 에뮬레이션 코어 스레드 실행
    thread::spawn(move || {
        if let Err(e) = emu_4leaf(state_tx, cmd_rx, ui_tx, context_for_emu, splash_tx) {
            eprintln!("[4leaf Emulator Error] {:?}", e);
        }
    });

    // 2. 가상 서버(백그운드) 스레드 실행
    thread::spawn(|| {
        if let Err(e) = server::server() {
            eprintln!("[Server Error] {:?}", e);
        }
    });

    // 3. UI 렌더러 준비 (스플래시 화면 및 디버그 창)
    let mut initial_painters: Vec<Box<dyn ui::Painter>> = Vec::new();
    if let Some((pixels, width, height)) = ui::splash::load_splash_data("./Resources") {
        let splash_painter = ui::splash::SplashPainter {
            pixels,
            width,
            height,
            receiver: splash_rx,
            should_close: false,
        };
        initial_painters.push(Box::new(splash_painter));
    }

    let debug_painter = crate::debug::Debug::new(cmd_tx, state_rx);
    initial_painters.push(Box::new(debug_painter));

    // 4. UI 이벤트 루프 실행 (메인 스레드 점유)
    ui::run_ui(ui_rx, initial_painters, context.clone());
}

/// Unicorn 엔진을 초기화하고 필수 DLL들을 로드한 뒤 메인 시뮬레이션을 시작합니다.
///
/// # 인자
/// * `state_tx`: UI로의 CPU 상태 전달 채널
/// * `cmd_rx`: UI로부터의 제어 명령 수신 채널
/// * `_ui_tx`: UI 조작 요청 채널
/// * `context`: Win32 상태 컨텍스트
/// * `splash_tx`: 스플래시 종료 알림 채널
fn emu_4leaf(
    state_tx: Sender<CpuContext>,
    cmd_rx: Receiver<DebugCommand>,
    _ui_tx: Sender<UiCommand>,
    context: Win32Context,
    splash_tx: Sender<()>,
) -> Result<(), ()> {
    let mut unicorn = Unicorn::new_with_data(Arch::X86, Mode::MODE_32, context)
        .expect("Failed to create the Unicorn instance");

    // 기본 훅 및 상태 전달 설정
    unicorn.setup(state_tx, cmd_rx).map_err(|e| {
        crate::emu_log!("[!] Infrastructure setup failed: {:?}", e);
    })?;

    // 어플리케이션 구동에 필요한 핵심 DLL 목록
    let dll_list = [
        "Core.dll",
        "WinCore.dll",
        "DNet.dll",
        "Lime.dll",
        "Rare.dll",
        "4Leaf.dll",
    ];

    for (i, dll_name) in dll_list.iter().enumerate() {
        let filename = format!("Resources/{}", dll_name);
        let target_base = (0x3000_0000 + i * 0x100_0000) as u64;

        crate::emu_log!("[*] Loading {} at {:#x}...", dll_name, target_base);

        // 1. DLL 로드 및 재배치(Relocation) 수행
        let loaded_dll = unicorn
            .load_dll_with_reloc(&filename, target_base)
            .map_err(|_| {
                crate::emu_log!("[!] Critical: Failed to load {}", dll_name);
            })?;

        // 2. IAT(Import Address Table) 해결
        unicorn.resolve_imports(&loaded_dll).map_err(|_| {
            crate::emu_log!("[!] Critical: Failed to resolve imports for {}", dll_name);
        })?;

        // 3. DllMain 실행
        unicorn.run_dll_entry(&loaded_dll).map_err(|_| {
            crate::emu_log!("[!] Critical: DllMain failed for {}", dll_name);
        })?;
    }

    // 모든 자격 증명이 로드되었음을 알리고 스플래시 창 종료 유도
    let _ = splash_tx.send(());

    // 4Leaf 메인 루틴 실행
    run_4leaf_main(&mut unicorn);

    Ok(())
}

/// 에뮬레이터가 런타임 준비를 마치면 최종적으로 4Leaf 어플리케이션의 엔트리 함수를 호출합니다.
///
/// # 인자
/// * `uc`: 초기화된 Unicorn 엔진 인스턴스
fn run_4leaf_main(uc: &mut Unicorn<Win32Context>) {
    let dll_name = "4Leaf.dll";
    let func_name = "Main";

    // Main(NULL, NULL, SHARED_MEM_BASE, "127.0.0.1") 형식으로 호출
    let args: Vec<Box<dyn Any>> = vec![
        Box::new(0u32),
        Box::new(0u32),
        Box::new(SHARED_MEM_BASE as u32),
        Box::new("127.0.0.1"),
    ];

    uc.run_emulator(dll_name, func_name, args);
}
