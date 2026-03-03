use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllWS2_32 {}

impl DllWS2_32 {
    pub fn ordinal_115() -> Option<(usize, Option<i32>)>{
        println!("ordinal_115");
        Some((0, None))
    }

    pub fn ordinal_10() -> Option<(usize, Option<i32>)>{
        println!("ordinal_10");
        Some((0, None))
    }

    pub fn ordinal_116() -> Option<(usize, Option<i32>)>{
        println!("ordinal_116");
        Some((0, None))
    }

    pub fn ordinal_7() -> Option<(usize, Option<i32>)>{
        println!("ordinal_7");
        Some((0, None))
    }

    pub fn ordinal_111() -> Option<(usize, Option<i32>)>{
        println!("ordinal_111");
        Some((0, None))
    }

    pub fn ordinal_16() -> Option<(usize, Option<i32>)>{
        println!("ordinal_16");
        Some((0, None))
    }

    pub fn ordinal_22() -> Option<(usize, Option<i32>)>{
        println!("ordinal_22");
        Some((0, None))
    }

    pub fn ordinal_19() -> Option<(usize, Option<i32>)>{
        println!("ordinal_19");
        Some((0, None))
    }

    pub fn ordinal_5() -> Option<(usize, Option<i32>)>{
        println!("ordinal_5");
        Some((0, None))
    }

    pub fn ordinal_12() -> Option<(usize, Option<i32>)>{
        println!("ordinal_12");
        Some((0, None))
    }

    pub fn ordinal_15() -> Option<(usize, Option<i32>)>{
        println!("ordinal_15");
        Some((0, None))
    }

    pub fn ordinal_13() -> Option<(usize, Option<i32>)>{
        println!("ordinal_13");
        Some((0, None))
    }

    pub fn ordinal_2() -> Option<(usize, Option<i32>)>{
        println!("ordinal_2");
        Some((0, None))
    }

    pub fn ordinal_21() -> Option<(usize, Option<i32>)>{
        println!("ordinal_21");
        Some((0, None))
    }

    pub fn ordinal_3() -> Option<(usize, Option<i32>)>{
        println!("ordinal_3");
        Some((0, None))
    }

    pub fn ordinal_23() -> Option<(usize, Option<i32>)>{
        println!("ordinal_23");
        Some((0, None))
    }

    pub fn ordinal_4() -> Option<(usize, Option<i32>)>{
        println!("ordinal_4");
        Some((0, None))
    }

    pub fn ordinal_1() -> Option<(usize, Option<i32>)>{
        println!("ordinal_1");
        Some((0, None))
    }

    pub fn ordinal_151() -> Option<(usize, Option<i32>)>{
        println!("ordinal_151");
        Some((0, None))
    }

    pub fn wsa_send() -> Option<(usize, Option<i32>)>{
        println!("wsa_send");
        Some((0, None))
    }

    pub fn ordinal_18() -> Option<(usize, Option<i32>)>{
        println!("ordinal_18");
        Some((0, None))
    }

    pub fn ordinal_8() -> Option<(usize, Option<i32>)>{
        println!("ordinal_8");
        Some((0, None))
    }

    pub fn wsa_enum_network_events() -> Option<(usize, Option<i32>)>{
        println!("wsa_enum_network_events");
        Some((0, None))
    }

    pub fn wsa_socket_a() -> Option<(usize, Option<i32>)>{
        println!("wsa_socket_a");
        Some((0, None))
    }

    pub fn wsa_create_event() -> Option<(usize, Option<i32>)>{
        println!("wsa_create_event");
        Some((0, None))
    }

    pub fn wsa_event_select() -> Option<(usize, Option<i32>)>{
        println!("wsa_event_select");
        Some((0, None))
    }

    pub fn wsa_close_event() -> Option<(usize, Option<i32>)>{
        println!("wsa_close_event");
        Some((0, None))
    }

    pub fn ordinal_52() -> Option<(usize, Option<i32>)>{
        println!("ordinal_52");
        Some((0, None))
    }

    pub fn ordinal_9() -> Option<(usize, Option<i32>)>{
        println!("ordinal_9");
        Some((0, None))
    }

    pub fn ordinal_11() -> Option<(usize, Option<i32>)>{
        println!("ordinal_11");
        Some((0, None))
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "Ordinal_115" => DllWS2_32::ordinal_115(),
            "Ordinal_10" => DllWS2_32::ordinal_10(),
            "Ordinal_116" => DllWS2_32::ordinal_116(),
            "Ordinal_7" => DllWS2_32::ordinal_7(),
            "Ordinal_111" => DllWS2_32::ordinal_111(),
            "Ordinal_16" => DllWS2_32::ordinal_16(),
            "Ordinal_22" => DllWS2_32::ordinal_22(),
            "Ordinal_19" => DllWS2_32::ordinal_19(),
            "Ordinal_5" => DllWS2_32::ordinal_5(),
            "Ordinal_12" => DllWS2_32::ordinal_12(),
            "Ordinal_15" => DllWS2_32::ordinal_15(),
            "Ordinal_13" => DllWS2_32::ordinal_13(),
            "Ordinal_2" => DllWS2_32::ordinal_2(),
            "Ordinal_21" => DllWS2_32::ordinal_21(),
            "Ordinal_3" => DllWS2_32::ordinal_3(),
            "Ordinal_23" => DllWS2_32::ordinal_23(),
            "Ordinal_4" => DllWS2_32::ordinal_4(),
            "Ordinal_1" => DllWS2_32::ordinal_1(),
            "Ordinal_151" => DllWS2_32::ordinal_151(),
            "WSASend" => DllWS2_32::wsa_send(),
            "Ordinal_18" => DllWS2_32::ordinal_18(),
            "Ordinal_8" => DllWS2_32::ordinal_8(),
            "WSAEnumNetworkEvents" => DllWS2_32::wsa_enum_network_events(),
            "WSASocketA" => DllWS2_32::wsa_socket_a(),
            "WSACreateEvent" => DllWS2_32::wsa_create_event(),
            "WSAEventSelect" => DllWS2_32::wsa_event_select(),
            "WSACloseEvent" => DllWS2_32::wsa_close_event(),
            "Ordinal_52" => DllWS2_32::ordinal_52(),
            "Ordinal_9" => DllWS2_32::ordinal_9(),
            "Ordinal_11" => DllWS2_32::ordinal_11(),
            _ => None
        }
    }
}
