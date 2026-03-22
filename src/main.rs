mod debug;
mod server;
#[macro_use]
mod helper;
mod browser;
mod packet_logger;
mod win32;

use helper::{SHARED_MEM_BASE, UnicornHelper};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::{
    any::Any,
    sync::mpsc::{Receiver, Sender, channel},
    thread,
};

/// UI 스레드와 로그 출력 스레드 간 동기화를 위한 전역 로그 큐 형태의 버퍼
pub static LOG_BUFFER: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
/// 새로운 로그가 들어올 때 마다 증가하여 UI가 언제 렌더링 루프를 `Redraw` 할 지 결정하게 돕는 카운터
pub static LOG_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 전역 버퍼에 새로운 로그 메시지를 밀어넣고 100줄을 초과하면 가장 오래된 로그가 지워짐
///
/// # 인자
/// - `msg`: 추가할 로그의 텍스트
pub fn push_log(msg: String) {
    if let Some(buf) = LOG_BUFFER.get()
        && let Ok(mut b) = buf.try_lock() {
            // \n 이 포함되어 있으면 나눠서 push 함으로써 텍스트 겹침 방지
            for line in msg.lines() {
                b.push_back(line.to_string());
                if b.len() > 1000 {
                    b.pop_front();
                }
            }
            LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
}

/// 어플리케이션 시작 시 전역 로그 버퍼를 빈 `VecDeque`로 초기화함
pub fn init_logger() {
    LOG_BUFFER.set(Mutex::new(VecDeque::new())).unwrap();
}

#[macro_export]
macro_rules! emu_log {
    () => {
        $crate::push_log(String::new());
    };
    ($($arg:tt)*) => {
        let msg = std::format!($($arg)*);
        // std::println!("{}", msg); // 성능 저하 원인: 주석 처리
        $crate::push_log(msg);
    };
}

use unicorn_engine::{
    Unicorn,
    unicorn_const::{Arch, Mode},
};
use win32::Win32Context;

use crate::debug::common::{CpuContext, DebugCommand, UiCommand};
use crate::debug::create_debug_window;

fn main() {
    init_logger();

    // 1. 통신 채널 생성
    let (cmd_tx, cmd_rx) = channel::<DebugCommand>();
    let (state_tx, state_rx) = channel::<CpuContext>();
    let (ui_tx, ui_rx) = channel::<UiCommand>();

    thread::spawn(move || {
        if let Err(e) = emu_4leaf(state_tx, cmd_rx, ui_tx) {
            eprintln!("[4leaf Emulator Error] {:?}", e);
        }
    });

    thread::spawn(|| {
        if let Err(e) = server::server() {
            eprintln!("[Server Error] {:?}", e);
        }
    });

    create_debug_window(cmd_tx, state_rx, ui_rx);
}

/// Unicorn 엔진을 초기화하고, 여러 필수 DLL 코어 파일들을 메모리에 로드 및 링킹한 뒤 메인 엔트리 포인트를 실행함
///
/// 별개의 백그라운드 스레드에서 돌아감
///
/// # 인자
/// - `state_tx`: UI로 현재 CPU 상태를 전송할 채널의 송신단
/// - `cmd_rx`: UI로부터 조작 커맨드를 받아올 채널의 수신단
/// - `ui_tx`: UI 조작(창 생성 등)을 요청할 채널의 송신단
fn emu_4leaf(
    state_tx: Sender<CpuContext>,
    cmd_rx: Receiver<DebugCommand>,
    ui_tx: Sender<UiCommand>,
) -> Result<(), ()> {
    let context = Win32Context::new(Some(ui_tx));
    let mut unicorn = Unicorn::new_with_data(Arch::X86, Mode::MODE_32, context)
        .expect("Failed to create the Unicorn");

    unicorn.setup(state_tx, cmd_rx).unwrap();

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

        crate::emu_log!(
            "\n[*] Loading address {:#x} from {}...",
            target_base,
            filename
        );
        let loaded_dll = unicorn
            .load_dll_with_reloc(filename.as_str(), target_base)
            .unwrap();

        crate::emu_log!("[*] Resolving Imports for {}...", filename);
        unicorn.resolve_imports(&loaded_dll).unwrap();

        crate::emu_log!("\n[*] Initializing {}...", dll_name);
        unicorn.run_dll_entry(&loaded_dll).unwrap();
    }

    run_4leaf_main(&mut unicorn);

    Ok(())
}

/// 모든 기본 DLL 로딩 작업이 완료된 후, 진입점인 `4Leaf.dll`의 `Main` 함수를
/// 공유 메모리 주소 및 환경 변수들과 함께 실행함
fn run_4leaf_main(uc: &mut Unicorn<Win32Context>) {
    let dll_name = "4Leaf.dll";
    let func_name = "Main";
    let args: Vec<Box<dyn Any>> = vec![
        Box::new(0u32),
        Box::new(0u32),
        Box::new(SHARED_MEM_BASE as u32),
        Box::new("127.0.0.1"),
    ];
    uc.run_dll_func(dll_name, func_name, args);
}
