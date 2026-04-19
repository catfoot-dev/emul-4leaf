//! # 4Leaf Emulator Main Entry Point
//!
//! 이 모듈은 에뮬레이터의 생명주기를 관리하며, UI 스레드, 에뮬레이션 스레드,
//! 그리고 서버 스레드 간의 조율을 담당합니다.

#![windows_subsystem = "windows"]

mod boot;
mod debug;
mod dll;
mod server;
#[macro_use]
mod helper;
mod ui;

pub use debug::logging::{
    INDEX, LOG_BUFFER, LOG_COUNT, SOCKET_LOG_BUFFER, SOCKET_LOG_COUNT, init_logger, push_log,
    push_socket_log,
};

// 리소스 디렉토리 재수출
pub use boot::resource_dir;

use dll::win32::{LoadedDll, Win32Context};
use helper::{SHARED_MEM_BASE, UnicornHelper};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::{any::Any, env, thread};
use unicorn_engine::{
    Unicorn,
    unicorn_const::{Arch, Mode},
};

use crate::boot::{
    LIBLARY_4LEAF, LIBLARY_CORE, LIBLARY_DICE, LIBLARY_DNET, LIBLARY_LIME, LIBLARY_WINCORE,
};
use crate::debug::common::{CpuContext, DebugCommand};
use crate::ui::UiCommand;

/// 에뮬레이션 로그 출력을 위한 매크로입니다.
/// 호출 시 전역 인덱스를 부여하고 특수 문자(\r, \n)를 이스케이프 처리하여 기록합니다.
#[macro_export]
macro_rules! emu_log {
    () => {
        if $crate::debug::should_send_debug_messages() {
            $crate::push_log(String::new())
        }
    };
    ($($arg:tt)*) => {{
        if $crate::debug::should_send_debug_messages() {
            let index = $crate::INDEX.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let msg = std::format!("[{:#08x}] {}", index, std::format!($($arg)*)).replace("\r", "\\r").replace("\n", "\\n");
            $crate::push_log(msg)
        }
    }};
}

#[macro_export]
macro_rules! emu_socket_log {
    () => {
        if $crate::debug::should_send_debug_messages() {
            $crate::push_socket_log(String::new())
        }
    };
    ($($arg:tt)*) => {{
        if $crate::debug::should_send_debug_messages() {
            let msg = std::format!($($arg)*);
            $crate::push_socket_log(msg)
        }
    }};
}

/// 어플리케이션 메인 진입점입니다.
/// 하위 시스템(로그, 에뮬레이션, 서버, UI)들을 초기화하고 실행합니다.
fn main() {
    boot::detect_resource_dir();
    init_logger();

    let headless_mode = env::var("EMUL_HEADLESS").ok().as_deref() == Some("1");
    let debug_window_enabled = crate::debug::should_create_debug_window();

    // 스레드 간 통신을 위한 채널 설정
    let (cmd_tx, cmd_rx) = channel::<DebugCommand>();
    let (state_tx, state_rx) = channel::<CpuContext>();
    let (ui_tx, ui_rx) = channel::<UiCommand>();
    let (splash_tx, splash_rx) = channel::<()>();

    // 1. Win32 에뮬레이션 상태 컨텍스트 생성 (Arc로 내부 상태 공유 가능)
    let context = Win32Context::new(Some(ui_tx.clone()));

    if headless_mode {
        if let Err(e) = emu_4leaf(None, None, ui_tx, context, splash_tx) {
            println!("[4leaf Emulator Error] {:?}", e);
        }
        return;
    }

    let context_for_emu = context.clone();
    // 1. 에뮬레이션 코어 스레드 실행
    thread::spawn(move || {
        if let Err(e) = emu_4leaf(
            (!headless_mode && debug_window_enabled).then_some(state_tx),
            (!headless_mode && debug_window_enabled).then_some(cmd_rx),
            ui_tx,
            context_for_emu,
            splash_tx,
        ) {
            println!("[4leaf Emulator Error] {:?}", e);
        }
    });

    // 2. UI 렌더러 준비 (스플래시 화면 및 디버그 창)
    let mut initial_painters: Vec<Box<dyn ui::Painter>> = Vec::new();
    if let Some((pixels, width, height)) = ui::splash::load_splash_data(resource_dir()) {
        let splash_painter = ui::splash::SplashPainter {
            pixels,
            width,
            height,
            receiver: splash_rx,
            should_close: false,
        };
        initial_painters.push(Box::new(splash_painter));
    }

    if debug_window_enabled {
        let debug_painter = crate::debug::Debug::new(cmd_tx, state_rx);
        initial_painters.push(Box::new(debug_painter));
    }

    // 4. UI 이벤트 루프 실행 (메인 스레드 점유)
    ui::run_ui(ui_rx, initial_painters, context.clone());
}

/// Unicorn 엔진을 초기화하고 필수 DLL들을 로드한 뒤 메인 시뮬레이션을 시작합니다.
///
/// # 인자
/// * `state_tx`: UI로의 CPU 상태 전달 채널. 비디버그 모드에서는 `None`
/// * `cmd_rx`: UI로부터의 제어 명령 수신 채널. 비디버그 모드에서는 `None`
/// * `_ui_tx`: UI 조작 요청 채널
/// * `context`: Win32 상태 컨텍스트
/// * `splash_tx`: 스플래시 종료 알림 채널
fn emu_4leaf(
    state_tx: Option<Sender<CpuContext>>,
    cmd_rx: Option<Receiver<DebugCommand>>,
    _ui_tx: Sender<UiCommand>,
    context: Win32Context,
    splash_tx: Sender<()>,
) -> Result<(), ()> {
    let mut unicorn = Unicorn::new_with_data(Arch::X86, Mode::MODE_32, context)
        .expect("Failed to create the Unicorn instance");

    // 기본 훅 및 상태 전달 설정
    unicorn.setup(None, None).map_err(|e| {
        crate::emu_log!("[!] Infrastructure setup failed: {:?}", e);
    })?;

    // Rare.dll은 호스트 프록시로 처리하므로 모듈 메타데이터만 선등록합니다.
    unicorn.get_data().dll_modules.lock().unwrap().insert(
        LIBLARY_CORE.to_string(),
        LoadedDll {
            name: resource_dir()
                .join(LIBLARY_CORE)
                .to_string_lossy()
                .to_string(),
            base_addr: 0x3000_0000,
            size: 0,
            entry_point: 0,
            exports: HashMap::new(),
        },
    );

    // 어플리케이션 구동에 필요한 핵심 DLL 목록
    let dll_list = [
        LIBLARY_CORE,
        LIBLARY_WINCORE,
        LIBLARY_DNET,
        LIBLARY_LIME,
        // LIBLARY_DICE,
        LIBLARY_4LEAF,
    ];

    let address_begin = 0x3200_0000_u64;
    for (i, dll_name) in dll_list.iter().enumerate() {
        let target_base = address_begin + (i as u64 * 0x0200_0000);
        let filename = resource_dir().join(dll_name).to_string_lossy().to_string();

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
    crate::ui::win_event::WinEvent::notify_wakeup();

    // 4Leaf 메인 루틴 실행
    run_4leaf_main(&mut unicorn, state_tx, cmd_rx);

    Ok(())
}

/// 에뮬레이터가 런타임 준비를 마치면 최종적으로 4Leaf 어플리케이션의 엔트리 함수를 호출합니다.
///
/// # 인자
/// * `uc`: 초기화된 Unicorn 엔진 인스턴스
fn run_4leaf_main(
    uc: &mut Unicorn<Win32Context>,
    state_tx: Option<Sender<CpuContext>>,
    cmd_rx: Option<Receiver<DebugCommand>>,
) {
    // Main(NULL, NULL, SHARED_MEM_BASE, "127.0.0.1") 형식으로 호출
    let args: Vec<Box<dyn Any>> = vec![
        Box::new(0u32),
        Box::new(0u32),
        Box::new(SHARED_MEM_BASE as u32),
        Box::new("127.0.0.1"),
    ];

    uc.run_emulator(LIBLARY_4LEAF, "Main", args, state_tx, cmd_rx);
}
