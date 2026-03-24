use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::server::packet_logger::PacketDirection;
use crate::win32::{ApiHookResult, SocketState, Win32Context, callee_result};
use std::sync::atomic::Ordering;

/// `WS2_32.dll` 프록시 구현 모듈
///
/// Winsock 라이브러리를 가상화하여 소켓 생성, 바인딩, 네트워크 I/O 송수신 등을 패킷 단위로 추적 및 에뮬레이팅
pub struct DllWS2_32;

impl DllWS2_32 {
    // API: SOCKET accept(SOCKET s, struct sockaddr* addr, int* addrlen)
    // 역할: Ordinal_1 - 들어오는 연결 요청을 수락
    pub fn accept(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen_ptr = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] accept({}, {:#x}, {:#x}) -> SOCKET -1i32",
            sock,
            addr_ptr,
            addrlen_ptr
        );
        Some((3, Some(-1i32))) // INVALID_SOCKET
    }

    // API: int bind(SOCKET s, const struct sockaddr* name, int namelen)
    // 역할: Ordinal_2 - 로컬 주소를 소켓에 연결
    pub fn bind(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen_ptr = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] bind({}, {:#x}, {:#x}) -> int 0",
            sock,
            addr_ptr,
            addrlen_ptr
        );
        Some((3, Some(0)))
    }

    // API: int closesocket(SOCKET s)
    // 역할: Ordinal_3 - 소켓을 닫음
    pub fn closesocket(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let ctx = uc.get_data();
        if let Some(s) = ctx.sockets.lock().unwrap().get_mut(&sock) {
            *s = SocketState::Closed;
        }
        crate::emu_log!("[WS2_32] closesocket({}) -> int 0", sock);
        Some((1, Some(0)))
    }

    /// API: int connect(SOCKET s, const struct sockaddr* name, int namelen)
    /// 역할: Ordinal_4 - 대상 서버에 연결을 시도
    pub fn connect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        let address = format!("{}:{}\0", ip, port);
        let ctx = uc.get_data();
        ctx.sockets.lock().unwrap().insert(
            sock,
            SocketState::Connected {
                remote_addr: ip,
                remote_port: port,
            },
        );
        crate::emu_log!(
            "[WS2_32] connect({}, \"{}\", {}) -> int 0",
            sock,
            address,
            address.len() + 1,
        );
        Some((3, Some(0))) // 성공
    }

    // API: int getpeername(SOCKET s, struct sockaddr* name, int* namelen)
    // 역할: Ordinal_5 - 연결된 상대방의 주소 정보를 가져옴
    pub fn getpeername(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen_ptr = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] getpeername({}, {}, {}) -> int 0",
            sock,
            addr_ptr,
            addrlen_ptr
        );
        Some((3, Some(0)))
    }

    // API: int getsockopt(SOCKET s, int level, int optname, char* optval, int* optlen)
    // 역할: Ordinal_7 - 소켓 옵션 값을 가져옴
    pub fn getsockopt(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let level = uc.read_arg(1);
        let optname = uc.read_arg(2);
        let optval = uc.read_arg(3);
        let optlen = uc.read_arg(4);
        crate::emu_log!(
            "[WS2_32] getsockopt({}, {}, {}, {}, {}) -> int 0",
            sock,
            level,
            optname,
            optval,
            optlen,
        );
        Some((5, Some(0)))
    }

    // API: u_long htonl(u_long hostlong)
    // 역할: Ordinal_8 - 32비트 호스트 바이트 순서를 네트워크 바이트 순서(Big-Endian)로 변환
    pub fn htonl(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let val = uc.read_arg(0);
        let result = val.to_be();
        crate::emu_log!("[WS2_32] htonl({}) -> u_long {}", val, result);
        Some((1, Some(result as i32)))
    }

    // API: u_short htons(u_short hostshort)
    // 역할: Ordinal_9 - 16비트 호스트 바이트 순서를 네트워크 바이트 순서(Big-Endian)로 변환
    pub fn htons(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let val = uc.read_arg(0) as u16;
        let result = val.to_be();
        crate::emu_log!("[WS2_32] htons({}) -> u_short {}", val, result);
        Some((1, Some(result as i32)))
    }

    // API: int ioctlsocket(SOCKET s, long cmd, u_long* argp)
    // 역할: Ordinal_10 - 소켓의 동작 방식을 변경
    pub fn ioctlsocket(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let cmd = uc.read_arg(1);
        let argp = uc.read_arg(2);
        crate::emu_log!("[WS2_32] ioctlsocket({}, {}, {}) -> int 0", sock, cmd, argp);
        Some((3, Some(0)))
    }

    // API: unsigned long inet_addr(const char* cp)
    // 역할: Ordinal_11 - IPv4 주소 문자열을 네트워크 바이트 순서의 정수로 변환
    pub fn inet_addr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr_str_ptr = uc.read_arg(0);
        let addr_str = uc.read_euc_kr(addr_str_ptr as u64);
        let parts: Vec<u8> = addr_str.split('.').filter_map(|p| p.parse().ok()).collect();
        let result = if parts.len() == 4 {
            u32::from_le_bytes([parts[0], parts[1], parts[2], parts[3]])
        } else {
            0xFFFFFFFF // INADDR_NONE
        };
        crate::emu_log!(
            "[WS2_32] inet_addr(\"{}\") -> u_long {:#x}",
            addr_str,
            result
        );
        Some((1, Some(result as i32)))
    }

    // API: char* inet_ntoa(struct in_addr in)
    // 역할: Ordinal_12 - 네트워크 바이트 순서의 IP 주소를 문자열로 변환
    pub fn inet_ntoa(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr = uc.read_arg(0);
        let bytes = addr.to_le_bytes();
        let ip_str = format!("{}.{}.{}.{}\0", bytes[0], bytes[1], bytes[2], bytes[3]);
        let ptr = uc.alloc_str(&ip_str[..ip_str.len() - 1]);
        crate::emu_log!(
            "[WS2_32] inet_ntoa({:#x}) -> char* {:#x}=\"{}\"",
            addr,
            ptr,
            ip_str
        );
        Some((1, Some(ptr as i32)))
    }

    // API: int listen(SOCKET s, int backlog)
    // 역할: Ordinal_13 - 소켓을 수신 모드로 설정
    pub fn listen(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let backlog = uc.read_arg(1);
        crate::emu_log!("[WS2_32] listen({}, {}) -> int 0", sock, backlog);
        Some((2, Some(0)))
    }

    // API: u_short ntohs(u_short netshort)
    // 역할: Ordinal_15 - 16비트 네트워크 바이트 순서를 호스트 바이트 순서로 변환
    pub fn ntohs(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let val = uc.read_arg(0) as u16;
        let result = u16::from_be(val);
        crate::emu_log!("[WS2_32] ntohs({}) -> u_short {}", val, result);
        Some((1, Some(result as i32)))
    }

    // API: int recv(SOCKET s, char* buf, int len, int flags)
    // 역할: Ordinal_16 - 소켓으로부터 데이터를 수신
    pub fn recv(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let _buf = uc.read_arg(1);
        let _len = uc.read_arg(2);
        // 현재는 WSAEWOULDBLOCK 반환 (비동기 모기 시뮬레이션)
        uc.get_data().last_error.store(10035, Ordering::SeqCst); // WSAEWOULDBLOCK
        crate::emu_log!(
            "[WS2_32] recv({}, {:#x}, {}, 0) -> int {}",
            sock,
            _buf,
            _len,
            -1i32
        );
        Some((4, Some(-1i32))) // SOCKET_ERROR
    }

    // API: int select(int nfds, fd_set* readfds, fd_set* writefds, fd_set* exceptfds, const struct timeval* timeout)
    // 역할: Ordinal_18 - 소켓의 읽기/쓰기/예외 상태를 확인
    pub fn select(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let nfds = uc.read_arg(0);
        let readfds = uc.read_arg(1);
        let writefds = uc.read_arg(2);
        let exceptfds = uc.read_arg(3);
        let timeout = uc.read_arg(4);
        crate::emu_log!(
            "[WS2_32] select({}, {:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
            nfds,
            readfds,
            writefds,
            exceptfds,
            timeout
        );
        Some((5, Some(0)))
    }

    // API: int send(SOCKET s, const char* buf, int len, int flags)
    // 역할: Ordinal_19 - 소켓을 통해 데이터를 전송
    pub fn send(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let len = uc.read_arg(2);
        let flags = uc.read_arg(3);
        if len > 0 {
            let data = uc
                .mem_read_as_vec(buf_addr as u64, len as usize)
                .unwrap_or_default();
            let ctx = uc.get_data();
            ctx.packet_logger
                .lock()
                .unwrap()
                .log(PacketDirection::Send, sock, &data);
        }
        crate::emu_log!(
            "[WS2_32] send({}, {:#x}, {}, {}) -> int {}",
            sock,
            buf_addr,
            len,
            flags,
            len
        );
        Some((4, Some(len as i32)))
    }

    // API: int setsockopt(SOCKET s, int level, int optname, const char* optval, int optlen)
    // 역할: Ordinal_21 - 소켓 옵션을 설정
    pub fn setsockopt(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let level = uc.read_arg(1);
        let optname = uc.read_arg(2);
        let optval = uc.read_arg(3);
        let optlen = uc.read_arg(4);
        crate::emu_log!(
            "[WS2_32] setsockopt({}, {}, {}, {:#x}, {}) -> int {}",
            sock,
            level,
            optname,
            optval,
            optlen,
            0
        );
        Some((5, Some(0)))
    }

    // API: int shutdown(SOCKET s, int how)
    // 역할: Ordinal_22 - 소켓의 송수신 기능을 중단
    pub fn shutdown(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let how = uc.read_arg(1);
        crate::emu_log!("[WS2_32] shutdown({}, {}) -> int {}", sock, how, 0);
        Some((2, Some(0)))
    }

    // API: SOCKET socket(int af, int type, int protocol)
    // 역할: Ordinal_23 - 새 소켓을 생성
    pub fn socket(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let af = uc.read_arg(0);
        let sock_type = uc.read_arg(1);
        let protocol = uc.read_arg(2);
        let ctx = uc.get_data();
        let sock = ctx.alloc_handle();
        ctx.sockets.lock().unwrap().insert(
            sock,
            SocketState::Created {
                af,
                sock_type,
                protocol,
            },
        );
        crate::emu_log!(
            "[WS2_32] socket({}, {}, {}) -> sock {}",
            af,
            sock_type,
            protocol,
            sock
        );
        Some((3, Some(sock as i32)))
    }

    // API: struct hostent* gethostbyname(const char* name)
    // 역할: Ordinal_52 - 호스트 이름에 해당하는 호스트 정보를 가져옴
    pub fn gethostbyname(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_euc_kr(name_addr as u64);
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

        crate::emu_log!(
            "[WS2_32] gethostbyname(\"{}\") -> struct hostent* {:#x}",
            name,
            hostent_addr
        );
        Some((1, Some(hostent_addr as i32)))
    }

    // API: int WSAGetLastError(void)
    // 역할: Ordinal_111 - 마지막으로 발생한 네트워크 오류 코드를 반환
    pub fn wsa_get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let err = ctx.last_error.load(Ordering::SeqCst);
        crate::emu_log!("[WS2_32] WSAGetLastError() -> {}", err);
        Some((0, Some(err as i32)))
    }

    // API: int WSAStartup(WORD wVersionRequested, LPWSADATA lpWSAData)
    // 역할: Ordinal_115 - Winsock 라이브러리를 초기화
    pub fn wsa_startup(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let version = uc.read_arg(0);
        let wsa_data_addr = uc.read_arg(1);

        // WSAData 구조체 394 bytes - 0으로 초기화 후 버전 세팅
        if wsa_data_addr != 0 {
            let zeros = vec![0u8; 394];
            uc.mem_write(wsa_data_addr as u64, &zeros).unwrap();
            // wVersion(2) + wHighVersion(2)
            uc.mem_write(wsa_data_addr as u64, &[2, 2]).unwrap();
            uc.mem_write(wsa_data_addr as u64 + 2, &[2, 2]).unwrap();
        }

        crate::emu_log!(
            "[WS2_32] WSAStartup({:#x}, {:#x}) -> int 0",
            version,
            wsa_data_addr
        );
        Some((2, Some(0)))
    }

    // API: int WSACleanup(void)
    // 역할: Ordinal_116 - Winsock 라이브러리 사용을 종료
    pub fn wsa_cleanup(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[WS2_32] WSACleanup() -> 0");
        Some((0, Some(0)))
    }

    // API: int WSAGetLastError(void)
    // 역할: Ordinal_111 - 마지막으로 발생한 네트워크 오류 코드를 반환
    pub fn wsa_fd_is_set(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let set = uc.read_arg(1);
        crate::emu_log!("[WS2_32] __WSAFDIsSet({:#x}, {:#x}) -> 0", sock, set);
        Some((2, Some(0)))
    }

    // API: int WSASend(SOCKET s, LPWSABUF lpBuffers, DWORD dwBufferCount, LPDWORD lpNumberOfBytesSent, DWORD dwFlags, LPWSAOVERLAPPED lpOverlapped, LPWSAOVERLAPPED_COMPLETION_ROUTINE lpCompletionRoutine)
    // 역할: 중첩된(Overlapped) 입출력을 사용하여 데이터를 전송
    pub fn wsa_send(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let bufs = uc.read_arg(1);
        let buf_count = uc.read_arg(2);
        let bytes_sent = uc.read_arg(3);
        let flags = uc.read_arg(4);
        let overlapped = uc.read_arg(5);
        let completion_routine = uc.read_arg(6);
        crate::emu_log!(
            "[WS2_32] WSASend({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> int {}",
            sock,
            bufs,
            buf_count,
            bytes_sent,
            flags,
            overlapped,
            completion_routine,
            0
        );
        Some((7, Some(0)))
    }

    // API: SOCKET WSASocketA(int af, int type, int protocol, LPWSAPROTOCOL_INFOA lpProtocolInfo, GROUP g, DWORD dwFlags)
    // 역할: 새 소켓을 생성 (확장 기능 포함)
    pub fn wsa_socket_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let af = uc.read_arg(0);
        let sock_type = uc.read_arg(1);
        let protocol = uc.read_arg(2);
        let protocol_info = uc.read_arg(3);
        let group = uc.read_arg(4);
        let flags = uc.read_arg(5);
        let ctx = uc.get_data();
        let sock = ctx.alloc_handle();
        ctx.sockets.lock().unwrap().insert(
            sock,
            SocketState::Created {
                af,
                sock_type,
                protocol,
            },
        );
        crate::emu_log!(
            "[WS2_32] WSASocketA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> sock {:#x}",
            af,
            sock_type,
            protocol,
            protocol_info,
            group,
            flags,
            sock
        );
        Some((6, Some(sock as i32)))
    }

    // API: WSAEVENT WSACreateEvent(void)
    // 역할: 새 이벤트 개체를 생성
    pub fn wsa_create_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        crate::emu_log!("[WS2_32] WSACreateEvent() -> {:#x}", handle);
        Some((0, Some(handle as i32)))
    }

    // API: int WSAEventSelect(SOCKET s, WSAEVENT hEventObject, long lNetworkEvents)
    // 역할: 소켓 이벤트를 이벤트 개체와 연결
    pub fn wsa_event_select(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let event = uc.read_arg(1);
        let network_events = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] WSAEventSelect({:#x}, {:#x}, {:#x}) -> 0",
            sock,
            event,
            network_events
        );
        Some((3, Some(0)))
    }

    // API: BOOL WSACloseEvent(WSAEVENT hEvent)
    // 역할: 이벤트 개체를 닫음
    pub fn wsa_close_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let event = uc.read_arg(0);
        crate::emu_log!("[WS2_32] WSACloseEvent({:#x}) -> 1", event);
        Some((1, Some(1))) // TRUE
    }

    // API: int WSAEnumNetworkEvents(SOCKET s, WSAEVENT hEventObject, LPWSANETWORKEVENTS lpNetworkEvents)
    // 역할: 특정 소켓에서 발생한 네트워크 이벤트를 확인
    pub fn wsa_enum_network_events(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let event = uc.read_arg(1);
        let net_events_addr = uc.read_arg(2);
        // WSANETWORKEVENTS: lNetworkEvents(4) + iErrorCode[10](40) = 44 bytes
        if net_events_addr != 0 {
            let zeros = [0u8; 44];
            uc.mem_write(net_events_addr as u64, &zeros).unwrap();
        }
        crate::emu_log!(
            "[WS2_32] WSAEnumNetworkEvents({:#x}, {:#x}, {:#x}) -> int 0",
            sock,
            event,
            net_events_addr
        );
        Some((3, Some(0)))
    }

    /// 함수명 기준 `WS2_32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // =========================================================
            // Ordinal → Real Winsock Function Mapping (WinXP ws2_32.dll)
            // =========================================================
            "Ordinal_1" | "accept" => Self::accept(uc),
            "Ordinal_2" | "bind" => Self::bind(uc),
            "Ordinal_3" | "closesocket" => Self::closesocket(uc),
            "Ordinal_4" | "connect" => Self::connect(uc),
            "Ordinal_5" | "getpeername" => Self::getpeername(uc),
            // 6: getsocketname
            "Ordinal_7" | "getsockopt" => Self::getsockopt(uc),
            "Ordinal_8" | "htonl" => Self::htonl(uc),
            "Ordinal_9" | "htons" => Self::htons(uc),
            "Ordinal_10" | "ioctlsocket" => Self::ioctlsocket(uc),
            "Ordinal_11" | "inet_addr" => Self::inet_addr(uc),
            "Ordinal_12" | "inet_ntoa" => Self::inet_ntoa(uc),
            "Ordinal_13" | "listen" => Self::listen(uc),
            // 14: ntohl
            "Ordinal_15" | "ntohs" => Self::ntohs(uc),
            "Ordinal_16" | "recv" => Self::recv(uc),
            // 17: recvfrom
            "Ordinal_18" | "select" => Self::select(uc),
            "Ordinal_19" | "send" => Self::send(uc),
            // 20: sendto
            "Ordinal_21" | "setsockopt" => Self::setsockopt(uc),
            "Ordinal_22" | "shutdown" => Self::shutdown(uc),
            "Ordinal_23" | "socket" => Self::socket(uc),
            // ...
            "Ordinal_52" | "gethostbyname" => Self::gethostbyname(uc),
            // ...
            "Ordinal_111" | "WSAGetLastError" => Self::wsa_get_last_error(uc),
            "Ordinal_115" | "WSAStartup" => Self::wsa_startup(uc),
            "Ordinal_116" | "WSACleanup" => Self::wsa_cleanup(uc),
            "Ordinal_151" | "__WSAFDIsSet" => Self::wsa_fd_is_set(uc),
            "WSASend" => Self::wsa_send(uc),
            "WSASocketA" => Self::wsa_socket_a(uc),
            "WSACreateEvent" => Self::wsa_create_event(uc),
            "WSAEventSelect" => Self::wsa_event_select(uc),
            "WSACloseEvent" => Self::wsa_close_event(uc),
            "WSAEnumNetworkEvents" => Self::wsa_enum_network_events(uc),
            _ => {
                crate::emu_log!("[!] WS2_32 Unhandled: {}", func_name);
                None
            }
        })
    }
}
