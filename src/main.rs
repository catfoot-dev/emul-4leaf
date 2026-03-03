mod server;
mod debug;
mod helper;
mod win32;

use helper::{SHARED_MEM_BASE, UnicornHelper};
use std::{any::Any, sync::mpsc::{Receiver, Sender, channel}, thread};
use unicorn_engine::{Unicorn, unicorn_const::{Arch, Mode}};
use win32::Win32Context;

use crate::debug::{Debug, create_debug_window};
use crate::debug::common::{CpuContext, DebugCommand};

fn main() {
    // 1. 통신 채널 생성
    let (cmd_tx, cmd_rx) = channel::<DebugCommand>();
    let (state_tx, state_rx) = channel::<CpuContext>();
    
    thread::spawn(move || {
        if let Err(e) = emu_4leaf(state_tx, cmd_rx) {
            eprintln!("[4leaf Emulator Error] {:?}", e);
        }
    });

    thread::spawn(|| {
        if let Err(e) = server::server() {
            eprintln!("[Server Error] {:?}", e);
        }
    });

    create_debug_window(cmd_tx, state_rx);
}

fn emu_4leaf(state_tx: Sender<CpuContext>, cmd_rx: Receiver<DebugCommand>) -> Result<(), ()> {
    let context = Win32Context::new();
    let mut unicorn = Unicorn::new_with_data(Arch::X86, Mode::MODE_32, context)
        .expect("Failed to create the Unicorn");

    unicorn.setup(state_tx, cmd_rx).unwrap();
    
    let dll_list = vec![
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

        println!("\n[*] Loading address {:#x} from {}...", target_base, filename);
        let loaded_dll = unicorn.load_dll_with_reloc(filename.as_str(), target_base).unwrap();

        println!("[*] Resolving Imports for {}...", filename);
        unicorn.resolve_imports(&loaded_dll).unwrap();

        println!("\n[*] Initializing {}...", dll_name);
        unicorn.run_dll_main(&loaded_dll).unwrap();
    }

    run_4leaf_main(&mut unicorn);

    Ok(())
}

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
