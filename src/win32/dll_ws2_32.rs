use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::packet_logger::PacketDirection;
use crate::win32::{ApiHookResult, SocketState, Win32Context, callee_result};

pub struct DllWS2_32 {}

impl DllWS2_32 {
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // =========================================================
            // Ordinal → Real Winsock Function Mapping (WinXP ws2_32.dll)
            // =========================================================
            "Ordinal_1" => {
                // accept(SOCKET, sockaddr*, int*)
                println!("[WS2_32] accept(...)");
                Some((3, Some(-1i32))) // INVALID_SOCKET
            }
            "Ordinal_2" => {
                // bind(SOCKET, sockaddr*, int)
                println!("[WS2_32] bind(...)");
                Some((3, Some(0)))
            }
            "Ordinal_3" => {
                // closesocket(SOCKET)
                let sock = uc.read_arg(0);
                println!("[WS2_32] closesocket({})", sock);
                let ctx = uc.get_data_mut();
                if let Some(s) = ctx.sockets.get_mut(&sock) {
                    *s = SocketState::Closed;
                }
                Some((1, Some(0)))
            }
            "Ordinal_4" => {
                // connect(SOCKET, sockaddr*, int)
                let sock = uc.read_arg(0);
                let addr_ptr = uc.read_arg(1);
                // sockaddr_in: sin_family(2), sin_port(2), sin_addr(4)
                let port_bytes = uc.mem_read_as_vec(addr_ptr as u64 + 2, 2).unwrap();
                let port = u16::from_be_bytes([port_bytes[0], port_bytes[1]]);
                let ip_bytes = uc.mem_read_as_vec(addr_ptr as u64 + 4, 4).unwrap();
                let ip = format!(
                    "{}.{}.{}.{}",
                    ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]
                );
                println!("[WS2_32] connect(sock={}, {}:{}) [PACKET]", sock, ip, port);
                let ctx = uc.get_data_mut();
                ctx.sockets.insert(
                    sock,
                    SocketState::Connected {
                        remote_addr: ip,
                        remote_port: port,
                    },
                );
                Some((3, Some(0))) // 성공
            }
            "Ordinal_5" => {
                // getpeername(SOCKET, sockaddr*, int*)
                println!("[WS2_32] getpeername(...)");
                Some((3, Some(0)))
            }
            "Ordinal_7" => {
                // getprotobyname(const char*)
                println!("[WS2_32] getprotobyname(...)");
                Some((1, Some(0)))
            }
            "Ordinal_8" => {
                // gethostbyname(const char*)
                let name_addr = uc.read_arg(0);
                let name = uc.read_string(name_addr as u64);
                println!(
                    "[WS2_32] gethostbyname(\"{}\") -> returning localhost",
                    name
                );
                // hostent 구조체를 에뮬 메모리에 할당
                // 간략화: 127.0.0.1로 반환
                let hostent_addr = uc.malloc(32);
                let ip_data = uc.malloc(4);
                uc.mem_write(ip_data, &[127, 0, 0, 1]).unwrap();
                let ip_ptr = uc.malloc(8);
                uc.write_u32(ip_ptr, ip_data as u32);
                uc.write_u32(ip_ptr + 4, 0); // NULL 종료

                // hostent: h_name, h_aliases, h_addrtype, h_length, h_addr_list
                let name_str = uc.alloc_str("localhost");
                uc.write_u32(hostent_addr, name_str); // h_name
                uc.write_u32(hostent_addr + 4, 0); // h_aliases
                uc.write_u32(hostent_addr + 8, 2); // h_addrtype (AF_INET)
                uc.write_u32(hostent_addr + 12, 4); // h_length
                uc.write_u32(hostent_addr + 16, ip_ptr as u32); // h_addr_list
                Some((1, Some(hostent_addr as i32)))
            }
            "Ordinal_9" => {
                // getsockopt(SOCKET, int, int, char*, int*)
                println!("[WS2_32] getsockopt(...)");
                Some((5, Some(0)))
            }
            "Ordinal_10" => {
                // htonl(u32)
                let val = uc.read_arg(0);
                let result = val.to_be();
                Some((1, Some(result as i32)))
            }
            "Ordinal_11" => {
                // htons(u16)
                let val = uc.read_arg(0) as u16;
                let result = val.to_be();
                Some((1, Some(result as i32)))
            }
            "Ordinal_12" => {
                // inet_addr(const char*)
                let addr_str_ptr = uc.read_arg(0);
                let addr_str = uc.read_string(addr_str_ptr as u64);
                println!("[WS2_32] inet_addr(\"{}\") [PACKET]", addr_str);
                let parts: Vec<u8> = addr_str.split('.').filter_map(|p| p.parse().ok()).collect();
                let result = if parts.len() == 4 {
                    u32::from_le_bytes([parts[0], parts[1], parts[2], parts[3]])
                } else {
                    0xFFFFFFFF // INADDR_NONE
                };
                Some((1, Some(result as i32)))
            }
            "Ordinal_13" => {
                // inet_ntoa(in_addr)
                let addr = uc.read_arg(0);
                let bytes = addr.to_le_bytes();
                let ip_str = format!("{}.{}.{}.{}\0", bytes[0], bytes[1], bytes[2], bytes[3]);
                let ptr = uc.alloc_str(&ip_str[..ip_str.len() - 1]);
                Some((1, Some(ptr as i32)))
            }
            "Ordinal_15" => {
                // ntohs(u16)
                let val = uc.read_arg(0) as u16;
                let result = u16::from_be(val);
                Some((1, Some(result as i32)))
            }
            "Ordinal_16" => {
                // recv(SOCKET, char*, int, int)
                let sock = uc.read_arg(0);
                let _buf = uc.read_arg(1);
                let _len = uc.read_arg(2);
                println!(
                    "[WS2_32] recv(sock={}, buf={:#x}, len={})",
                    sock, _buf, _len
                );
                // 현재는 WSAEWOULDBLOCK 반환 (비동기 모드 시뮬레이션)
                uc.get_data_mut().last_error = 10035; // WSAEWOULDBLOCK
                Some((4, Some(-1i32))) // SOCKET_ERROR
            }
            "Ordinal_18" => {
                // send(SOCKET, const char*, int, int)
                let sock = uc.read_arg(0);
                let buf_addr = uc.read_arg(1);
                let len = uc.read_arg(2);
                if len > 0 {
                    let data = uc
                        .mem_read_as_vec(buf_addr as u64, len as usize)
                        .unwrap_or_default();
                    uc.get_data_mut()
                        .packet_logger
                        .log(PacketDirection::Send, sock, &data);
                }
                Some((4, Some(len as i32)))
            }
            "Ordinal_19" => {
                // setsockopt(SOCKET, int, int, const char*, int)
                println!("[WS2_32] setsockopt(...)");
                Some((5, Some(0)))
            }
            "Ordinal_21" => {
                // shutdown(SOCKET, int)
                let sock = uc.read_arg(0);
                println!("[WS2_32] shutdown(sock={})", sock);
                Some((2, Some(0)))
            }
            "Ordinal_22" => {
                // socket(int, int, int)
                let af = uc.read_arg(0);
                let sock_type = uc.read_arg(1);
                let protocol = uc.read_arg(2);
                let ctx = uc.get_data_mut();
                let sock = ctx.alloc_handle();
                ctx.sockets.insert(
                    sock,
                    SocketState::Created {
                        af,
                        sock_type,
                        protocol,
                    },
                );
                println!(
                    "[WS2_32] socket({}, {}, {}) -> sock {} [PACKET]",
                    af, sock_type, protocol, sock
                );
                Some((3, Some(sock as i32)))
            }
            "Ordinal_23" => {
                // WSAStartup(WORD, LPWSADATA)
                let _version = uc.read_arg(0);
                let wsa_data_addr = uc.read_arg(1);
                // WSAData 구조체 394 bytes - 0으로 초기화 후 버전 세팅
                if wsa_data_addr != 0 {
                    let zeros = vec![0u8; 394];
                    uc.mem_write(wsa_data_addr as u64, &zeros).unwrap();
                    // wVersion(2) + wHighVersion(2)
                    uc.mem_write(wsa_data_addr as u64, &[2, 2]).unwrap();
                    uc.mem_write(wsa_data_addr as u64 + 2, &[2, 2]).unwrap();
                }
                println!("[WS2_32] WSAStartup(...) -> 0");
                Some((2, Some(0)))
            }
            "Ordinal_52" => {
                // gethostname(char*, int)
                let buf_addr = uc.read_arg(0);
                let hostname = "4Leaf-EMU\0";
                uc.mem_write(buf_addr as u64, hostname.as_bytes()).unwrap();
                println!("[WS2_32] gethostname(...) -> \"4Leaf-EMU\"");
                Some((2, Some(0)))
            }
            "Ordinal_111" => {
                // WSAGetLastError()
                let ctx = uc.get_data_mut();
                let err = ctx.last_error;
                println!("[WS2_32] WSAGetLastError() -> {}", err);
                Some((0, Some(err as i32)))
            }
            "Ordinal_115" => {
                // WSAStartup (또 다른 ordinal mapping)
                println!("[WS2_32] WSAStartup(ordinal 115) -> 0");
                Some((2, Some(0)))
            }
            "Ordinal_116" => {
                // WSACleanup()
                println!("[WS2_32] WSACleanup() -> 0");
                Some((0, Some(0)))
            }
            "Ordinal_151" => {
                // __WSAFDIsSet(SOCKET, fd_set*)
                println!("[WS2_32] __WSAFDIsSet(...)");
                Some((2, Some(0)))
            }
            "WSASend" => {
                let sock = uc.read_arg(0);
                let _bufs = uc.read_arg(1);
                let buf_count = uc.read_arg(2);
                println!(
                    "[WS2_32] WSASend(sock={}, bufs={}) [PACKET]",
                    sock, buf_count
                );
                Some((7, Some(0)))
            }
            "WSASocketA" => {
                let af = uc.read_arg(0);
                let sock_type = uc.read_arg(1);
                let protocol = uc.read_arg(2);
                let ctx = uc.get_data_mut();
                let sock = ctx.alloc_handle();
                ctx.sockets.insert(
                    sock,
                    SocketState::Created {
                        af,
                        sock_type,
                        protocol,
                    },
                );
                println!(
                    "[WS2_32] WSASocketA({}, {}, {}) -> sock {}",
                    af, sock_type, protocol, sock
                );
                Some((6, Some(sock as i32)))
            }
            "WSACreateEvent" => {
                let ctx = uc.get_data_mut();
                let handle = ctx.alloc_handle();
                println!("[WS2_32] WSACreateEvent() -> {:#x}", handle);
                Some((0, Some(handle as i32)))
            }
            "WSAEventSelect" => {
                println!("[WS2_32] WSAEventSelect(...)");
                Some((3, Some(0)))
            }
            "WSACloseEvent" => {
                println!("[WS2_32] WSACloseEvent(...)");
                Some((1, Some(1))) // TRUE
            }
            "WSAEnumNetworkEvents" => {
                let _sock = uc.read_arg(0);
                let _event = uc.read_arg(1);
                let net_events_addr = uc.read_arg(2);
                // WSANETWORKEVENTS: lNetworkEvents(4) + iErrorCode[10](40) = 44 bytes
                if net_events_addr != 0 {
                    let zeros = [0u8; 44];
                    uc.mem_write(net_events_addr as u64, &zeros).unwrap();
                }
                Some((3, Some(0)))
            }
            _ => {
                println!("[WS2_32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
