use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllCOMCTL32 {}

impl DllCOMCTL32 {
    pub fn __track_mouse_event() -> Option<(usize, Option<i32>)>{
        println!("__track_mouse_event");
        Some((0, None))
    }

    pub fn init_common_controls_ex() -> Option<(usize, Option<i32>)>{
        println!("init_common_controls_ex");
        Some((0, None))
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "_TrackMouseEvent" => DllCOMCTL32::__track_mouse_event(),
            "InitCommonControlsEx" => DllCOMCTL32::init_common_controls_ex(),
            _ => None
        }
    }
}
