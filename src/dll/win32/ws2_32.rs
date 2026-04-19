use crate::{
    dll::win32::{ApiHookResult, VirtualSocket, Win32Context, WsaEventEntry, kernel32::KERNEL32},
    helper::UnicornHelper,
    server::packet_logger::PacketDirection,
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use unicorn_engine::Unicorn;

/// `WS2_32.dll` 프록시 구현 모듈
///
/// `std::sync::mpsc` 채널 기반 인-프로세스 가상 소켓 I/O를 에뮬레이션합니다.
/// connect() 호출 시 DNet 핸들러 스레드를 생성하며, 실제 TCP 연결은 사용하지 않습니다.
/// WSAEWOULDBLOCK - 논블로킹 소켓의 '지금 바로 처리 불가' 오류 코드입니다.
const WSAEWOULDBLOCK: u32 = 10035;
/// WSAECONNREFUSED - 연결 거부 오류 코드입니다.
#[allow(dead_code)]
const WSAECONNREFUSED: u32 = 10061;
/// WSAEOPNOTSUPP - 소켓이 해당 플래그/동작을 지원하지 않는다는 오류 코드입니다.
const WSAEOPNOTSUPP: u32 = 10045;
/// Winsock 표준 소켓 오류 반환값입니다.
const SOCKET_ERROR: i32 = -1;
/// ioctlsocket 함수용 FIONBIO 명령어 코드입니다.
const FIONBIO: u32 = 0x8004667E;
/// recv 계열에서 버퍼를 소비하지 않고 미리보기만 하는 플래그입니다.
const MSG_PEEK: u32 = 0x0002;
/// send/recv 계열의 OOB 플래그입니다.
// const MSG_OOB: u32 = 0x0001;
/// WSASocketA에서 일반적으로 사용되는 overlapped 플래그입니다.
const WSA_FLAG_OVERLAPPED: u32 = 0x01;
/// WSASocketA의 handle 상속 금지 플래그입니다.
const WSA_FLAG_NO_HANDLE_INHERIT: u32 = 0x80;

/// recv 계열 플래그에 따라 버퍼를 유지할지 계산합니다.
fn should_peek(flags: u32) -> bool {
    flags & MSG_PEEK != 0
}

/// 지원하지 않는 플래그 비트를 계산합니다.
fn unsupported_flag_bits(flags: u32, supported_mask: u32) -> u32 {
    flags & !supported_mask
}

/// 지원하지 않는 플래그를 받은 경우 Winsock 스타일 오류를 설정합니다.
fn return_unsupported_flags(
    uc: &mut Unicorn<Win32Context>,
    api_name: &str,
    flags: u32,
    allowed_mask: u32,
    argc: usize,
) -> Option<ApiHookResult> {
    let unsupported = unsupported_flag_bits(flags, allowed_mask);
    if unsupported == 0 {
        return None;
    }

    uc.get_data()
        .last_error
        .store(WSAEOPNOTSUPP, Ordering::SeqCst);
    crate::emu_log!(
        "[WS2_32] {} flags={:#x} unsupported_bits={:#x} -> SOCKET_ERROR",
        api_name,
        flags,
        unsupported
    );
    crate::emu_socket_log!(
        "[{}] unsupported flags={:#x} unsupported_bits={:#x}",
        api_name,
        flags,
        unsupported
    );
    Some(ApiHookResult::callee(argc, Some(SOCKET_ERROR)))
}

/// `select`의 `timeval` 값을 Rust `Duration`으로 변환합니다.
///
/// `None`은 Winsock의 `timeout == NULL`과 동일하며 무한 대기를 의미합니다.
fn select_timeout_duration(timeval: Option<(u32, u32)>) -> Option<Duration> {
    timeval.map(|(sec, usec)| Duration::from_secs(sec as u64) + Duration::from_micros(usec as u64))
}

/// 내부 수신 버퍼에서 필요한 만큼 복사하고, peek가 아니면 소비합니다.
fn take_from_recv_buf(recv_buf: &mut Vec<u8>, requested: usize, peek: bool) -> Vec<u8> {
    let take = requested.min(recv_buf.len());
    if peek {
        recv_buf[..take].to_vec()
    } else {
        recv_buf.drain(..take).collect()
    }
}

/// recv/WSARecv 공통으로 가상 소켓의 대기 데이터를 가져옵니다.
///
/// peek일 때는 수신 데이터를 `recv_buf`에 유지하고 복사만 반환합니다.
fn recv_pending_data(
    socket: &mut VirtualSocket,
    requested: usize,
    peek: bool,
) -> Result<Vec<u8>, std::sync::mpsc::TryRecvError> {
    let mut data = if socket.recv_buf.is_empty() {
        Vec::new()
    } else {
        take_from_recv_buf(&mut socket.recv_buf, requested, peek)
    };

    if data.len() >= requested {
        return Ok(data);
    }

    let chan_rx = match socket.chan_rx.as_mut() {
        Some(rx) => rx,
        None => return Err(std::sync::mpsc::TryRecvError::Empty),
    };

    match chan_rx.try_recv() {
        Ok(msg) => {
            if peek {
                // peek는 큐에서 제거한 데이터를 내부 버퍼에 되돌려 놓고 복사만 반환합니다.
                socket.recv_buf.extend_from_slice(&msg);
                Ok(take_from_recv_buf(&mut socket.recv_buf, requested, true))
            } else {
                let want = requested.saturating_sub(data.len());
                let take = want.min(msg.len());
                data.extend_from_slice(&msg[..take]);
                if msg.len() > take {
                    let mut leftover = msg[take..].to_vec();
                    leftover.append(&mut socket.recv_buf);
                    socket.recv_buf = leftover;
                }
                Ok(data)
            }
        }
        Err(err) => {
            if data.is_empty() {
                Err(err)
            } else {
                Ok(data)
            }
        }
    }
}

pub struct WS2_32 {}

impl WS2_32 {
    /// **Ordinal 1: accept**
    ///
    /// 들어오는 연결 요청을 수락합니다.
    /// 현재 리스닝 소켓은 미구현 상태이므로 항상 `INVALID_SOCKET`을 반환합니다.
    pub fn accept(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen_ptr = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] accept({}, {:#x}, {:#x}) -> SOCKET -1 (not implemented)",
            sock,
            addr_ptr,
            addrlen_ptr
        );
        Some(ApiHookResult::callee(3, Some(-1i32))) // INVALID_SOCKET
    }

    /// **Ordinal 2: bind**
    ///
    /// 로컬 주소를 소켓에 연결합니다. 에뮬레이션 환경에서는 항상 성공으로 처리합니다.
    pub fn bind(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] bind({}, {:#x}, {}) -> int 0",
            sock,
            addr_ptr,
            addrlen
        );
        crate::emu_socket_log!("[BIND] sock={} addr_ptr={:#x}", sock, addr_ptr);
        Some(ApiHookResult::callee(3, Some(0)))
    }

    /// **Ordinal 3: closesocket**
    ///
    /// 소켓을 닫고 관련 리소스를 해제합니다.
    pub fn closesocket(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let ctx = uc.get_data();
        ctx.tcp_sockets.lock().unwrap().remove(&sock);
        crate::emu_log!("[WS2_32] closesocket({}) -> int 0", sock);
        crate::emu_socket_log!("[CLOSE] sock={}", sock);
        Some(ApiHookResult::callee(1, Some(0)))
    }

    /// **Ordinal 4: connect**
    ///
    /// 채널 쌍을 생성하고 DNet 핸들러 스레드를 인-프로세스로 실행합니다.
    /// 실제 TCP 연결 없이 `std::sync::mpsc` 채널로 소켓 통신을 에뮬레이션합니다.
    pub fn connect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);

        // sockaddr_in 구조체 파싱: family(2), port(2 BE), addr(4)
        let port_bytes = uc
            .mem_read_as_vec(addr_ptr as u64 + 2, 2)
            .unwrap_or_default();
        let port = u16::from_be_bytes([port_bytes[0], port_bytes[1]]);
        let ip_bytes = uc
            .mem_read_as_vec(addr_ptr as u64 + 4, 4)
            .unwrap_or_default();
        let ip = format!(
            "{}.{}.{}.{}",
            ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]
        );
        let addr_str = format!("{}:{}", ip, port);

        crate::emu_log!("[WS2_32] connect({}, \"{}\") ...", sock, addr_str);
        crate::emu_socket_log!("[CONN] sock={} connecting to {} (channel)", sock, addr_str);

        // 채널 쌍 생성:
        //   guest_tx / handler_rx : 게스트 → DNet 핸들러 (send 경로)
        //   handler_tx / guest_rx : DNet 핸들러 → 게스트 (recv 경로)
        let (guest_tx, handler_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (handler_tx, guest_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        // DNet 프로토콜 핸들러를 별도 스레드로 실행
        std::thread::spawn(move || {
            crate::server::run_dnet_handler(handler_rx, handler_tx);
        });

        let ctx = uc.get_data();
        let non_blocking = ctx
            .tcp_sockets
            .lock()
            .unwrap()
            .get(&sock)
            .map(|s| s.non_blocking)
            .unwrap_or(false);

        ctx.tcp_sockets.lock().unwrap().insert(
            sock,
            VirtualSocket {
                af: 2,        // AF_INET
                sock_type: 1, // SOCK_STREAM
                protocol: 6,  // IPPROTO_TCP
                chan_tx: Some(guest_tx),
                chan_rx: Some(guest_rx),
                connected: true,
                recv_buf: Vec::new(),
                non_blocking,
                remote_addr: Some(addr_str.clone()),
            },
        );

        // 연결 성공 시 FD_CONNECT(0x10) 이벤트를 이 소켓을 보고 있는 WSA 이벤트에 반영
        {
            let mut wsa_map = ctx.wsa_event_map.lock().unwrap();
            for entry in wsa_map.values_mut() {
                if entry.socket == sock && entry.interest & 0x10 != 0 {
                    entry.pending |= 0x10; // FD_CONNECT
                }
            }
        }

        crate::emu_log!(
            "[WS2_32] connect({}, \"{}\") -> OK (channel)",
            sock,
            addr_str
        );
        crate::emu_socket_log!("[CONN] sock={} -> {} OK (channel)", sock, addr_str);
        Some(ApiHookResult::callee(3, Some(0)))
    }

    // API: int getpeername(SOCKET s, struct sockaddr* name, int* namelen)
    // 역할: Ordinal_5 - 연결된 상대방의 주소 정보를 가져옴
    pub fn getpeername(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen_ptr = uc.read_arg(2);

        let remote = uc
            .get_data()
            .tcp_sockets
            .lock()
            .unwrap()
            .get(&sock)
            .and_then(|s| s.remote_addr.clone());

        if let Some(addr) = remote
            && let Ok(sockaddr) = addr.parse::<std::net::SocketAddr>()
            && let std::net::SocketAddr::V4(v4) = sockaddr
        {
            let ip = v4.ip().octets();
            let port = v4.port().to_be();
            if addr_ptr != 0 {
                uc.write_u32(addr_ptr as u64, 0x0002u32); // sin_family = AF_INET(2)
                uc.mem_write(addr_ptr as u64 + 2, &port.to_be_bytes()).ok();
                uc.mem_write(addr_ptr as u64 + 4, &ip).ok();
            }
            if addrlen_ptr != 0 {
                uc.write_u32(addrlen_ptr as u64, 16);
            }
            crate::emu_log!("[WS2_32] getpeername({}) -> \"{}\" (OK)", sock, addr);
            return Some(ApiHookResult::callee(3, Some(0)));
        }
        crate::emu_log!(
            "[WS2_32] getpeername({}) -> SOCKET_ERROR (not connected)",
            sock
        );
        Some(ApiHookResult::callee(3, Some(SOCKET_ERROR)))
    }

    // API: int getsockopt(SOCKET s, int level, int optname, char* optval, int* optlen)
    // 역할: Ordinal_7 - 소켓 옵션 값을 가져옴
    pub fn getsockopt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let level = uc.read_arg(1);
        let optname = uc.read_arg(2);
        let optval = uc.read_arg(3);
        let optlen = uc.read_arg(4);
        // SO_ERROR (0xFFFF 레벨, optname 4103) - 연결 오류 없음으로 0 반환
        if optval != 0 {
            uc.write_u32(optval as u64, 0);
        }
        crate::emu_log!(
            "[WS2_32] getsockopt({}, {}, {}, {:#x}, {}) -> int 0",
            sock,
            level,
            optname,
            optval,
            optlen
        );
        Some(ApiHookResult::callee(5, Some(0)))
    }

    // API: u_long htonl(u_long hostlong)
    // 역할: Ordinal_8 - 32비트 호스트 바이트 순서를 네트워크 바이트 순서(Big-Endian)로 변환
    pub fn htonl(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let val = uc.read_arg(0);
        let result = val.swap_bytes();
        crate::emu_log!("[WS2_32] htonl({:#x}) -> u_long {:#x}", val, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: u_short htons(u_short hostshort)
    // 역할: Ordinal_9 - 16비트 호스트 바이트 순서를 네트워크 바이트 순서(Big-Endian)로 변환
    pub fn htons(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let val = uc.read_arg(0) as u16;
        let result = val.to_be();
        crate::emu_log!("[WS2_32] htons({}) -> u_short {}", val, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: int ioctlsocket(SOCKET s, long cmd, u_long* argp)
    // 역할: Ordinal_10 - FIONBIO로 논블로킹 모드 설정
    pub fn ioctlsocket(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let cmd = uc.read_arg(1);
        let argp = uc.read_arg(2);
        let ctx = uc.get_data();

        if cmd == FIONBIO {
            let val = if argp != 0 {
                uc.read_u32(argp as u64)
            } else {
                0
            };
            let non_blocking = val != 0;
            if let Some(s) = ctx.tcp_sockets.lock().unwrap().get_mut(&sock) {
                s.non_blocking = non_blocking;
                crate::emu_log!(
                    "[WS2_32] ioctlsocket({}, FIONBIO, {}) -> non_blocking={}",
                    sock,
                    val,
                    non_blocking
                );
                crate::emu_socket_log!(
                    "[IOCTL] sock={} FIONBIO non_blocking={}",
                    sock,
                    non_blocking
                );
            } else {
                // 소켓이 아직 연결되지 않은 경우 - 나중에 반영하기 위해 생성
                ctx.tcp_sockets.lock().unwrap().insert(
                    sock,
                    VirtualSocket {
                        af: 2,
                        sock_type: 1,
                        protocol: 6,
                        chan_tx: None,
                        chan_rx: None,
                        connected: false,
                        recv_buf: Vec::new(),
                        non_blocking,
                        remote_addr: None,
                    },
                );
            }
        } else {
            crate::emu_log!(
                "[WS2_32] ioctlsocket({}, cmd={:#x}, argp={:#x}) -> 0",
                sock,
                cmd,
                argp
            );
        }
        Some(ApiHookResult::callee(3, Some(0)))
    }

    // API: unsigned long inet_addr(const char* cp)
    // 역할: Ordinal_11 - IPv4 주소 문자열을 네트워크 바이트 순서의 정수로 변환
    pub fn inet_addr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let addr_str_ptr = uc.read_arg(0);
        let addr_str = uc.read_euc_kr(addr_str_ptr as u64);
        let parts: Vec<u8> = addr_str.split('.').filter_map(|p| p.parse().ok()).collect();
        let result = if parts.len() == 4 {
            u32::from_le_bytes([parts[0], parts[1], parts[2], parts[3]])
        } else {
            0xFFFFFFFF // INADDR_NONE
        };
        crate::emu_log!("[WS2_32] inet_addr(\"{}\") -> {:#x}", addr_str, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: char* inet_ntoa(struct in_addr in)
    // 역할: Ordinal_12 - 네트워크 바이트 순서의 IP 주소를 문자열로 변환
    pub fn inet_ntoa(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let addr = uc.read_arg(0);
        let bytes = addr.to_le_bytes();
        let ip_str = format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3]);
        let ptr = uc.alloc_str(&ip_str);
        crate::emu_log!(
            "[WS2_32] inet_ntoa({:#x}) -> char* {:#x}=\"{}\"",
            addr,
            ptr,
            ip_str
        );
        Some(ApiHookResult::callee(1, Some(ptr as i32)))
    }

    // API: int listen(SOCKET s, int backlog)
    // 역할: Ordinal_13 - 소켓을 수신 모드로 설정
    pub fn listen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let backlog = uc.read_arg(1);
        crate::emu_log!("[WS2_32] listen({}, {}) -> int 0 (stub)", sock, backlog);
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: u_long ntohl(u_long netlong)
    // 역할: Ordinal_14 - 32비트 네트워크 바이트 순서를 호스트 바이트 순서로 변환
    pub fn ntohl(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let val = uc.read_arg(0);
        let result = u32::from_be(val);
        crate::emu_log!("[WS2_32] ntohl({:#x}) -> u_long {:#x}", val, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: u_short ntohs(u_short netshort)
    // 역할: Ordinal_15 - 16비트 네트워크 바이트 순서를 호스트 바이트 순서로 변환
    pub fn ntohs(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let val = uc.read_arg(0) as u16;
        let result = u16::from_be(val);
        crate::emu_log!("[WS2_32] ntohs({}) -> u_short {}", val, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: int recv(SOCKET s, char* buf, int len, int flags)
    // 역할: Ordinal_16 - 실제 TcpStream에서 데이터를 수신
    pub fn recv(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let len = uc.read_arg(2) as usize;
        let flags = uc.read_arg(3);

        if let Some(result) = return_unsupported_flags(uc, "recv", flags, MSG_PEEK, 4) {
            return Some(result);
        }
        let peek = should_peek(flags);

        // 나중에 uc.mem_write()와 동시에 ctx를 빌릴 수 없으므로 Arc 먼저 클론
        // AtomicU32는 Clone이 없으므로 last_error는 별도로 처리
        let (tcp_sockets, packet_logger) = {
            let ctx = uc.get_data();
            (ctx.tcp_sockets.clone(), ctx.packet_logger.clone())
        };

        let mut sockets = tcp_sockets.lock().unwrap();
        let socket = match sockets.get_mut(&sock) {
            Some(s) => s,
            None => {
                drop(sockets);
                uc.get_data()
                    .last_error
                    .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR (no socket)", sock);
                crate::emu_socket_log!("[SERVER] sock={} FAIL: no socket", sock);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };

        let non_blocking = socket.non_blocking;
        if socket.chan_rx.is_none() {
            drop(sockets);
            uc.get_data()
                .last_error
                .store(WSAEWOULDBLOCK, Ordering::SeqCst);
            return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
        }

        match recv_pending_data(socket, len, peek) {
            Ok(data) => {
                let take = data.len();
                drop(sockets);
                let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
                KERNEL32::clear_retry_wait(uc.get_data(), tid);
                uc.mem_write(buf_addr as u64, &data).ok();
                packet_logger
                    .lock()
                    .unwrap()
                    .log(PacketDirection::Recv, sock, &data, !peek);
                crate::emu_log!(
                    "[WS2_32] recv({}, {:#x}, {}, {}) -> {} bytes",
                    sock,
                    buf_addr,
                    len,
                    flags,
                    take
                );
                // crate::emu_socket_log!(
                //     "[SERVER] sock={} -> {}{}",
                //     sock,
                //     take,
                //     if peek { " (peek)" } else { "" }
                // );
                Some(ApiHookResult::callee(4, Some(take as i32)))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                drop(sockets);
                let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
                if tid != 0 || !non_blocking {
                    KERNEL32::schedule_retry_wait(uc.get_data(), tid, None);
                    return Some(ApiHookResult::retry());
                }
                uc.get_data()
                    .last_error
                    .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                KERNEL32::clear_retry_wait(uc.get_data(), tid);
                crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR (WSAEWOULDBLOCK)", sock);
                crate::emu_socket_log!("[CLIENT] sock={} FAIL: WouldBlock", sock);
                Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                drop(sockets);
                let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
                KERNEL32::clear_retry_wait(uc.get_data(), tid);
                crate::emu_log!("[WS2_32] recv({}) -> 0 (channel closed)", sock);
                crate::emu_socket_log!("[CLIENT] sock={} channel closed (EOF)", sock);
                Some(ApiHookResult::callee(4, Some(0)))
            }
        }
    }

    // API: int select(int nfds, fd_set* readfds, fd_set* writefds, fd_set* exceptfds, const struct timeval* timeout)
    // 역할: Ordinal_18 - 소켓 읽기/쓰기 가능 여부를 확인
    pub fn select(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let _nfds = uc.read_arg(0);
        let readfds_ptr = uc.read_arg(1);
        let writefds_ptr = uc.read_arg(2);
        let exceptfds_ptr = uc.read_arg(3);
        let timeout_ptr = uc.read_arg(4);

        // Winsock에서 timeout == NULL은 기본 폴링이 아니라 무한 대기입니다.
        let timeout = if timeout_ptr != 0 {
            let sec = uc.read_u32(timeout_ptr as u64);
            let usec = uc.read_u32(timeout_ptr as u64 + 4);
            select_timeout_duration(Some((sec, usec)))
        } else {
            None
        };

        let mut total_ready = 0i32;

        if readfds_ptr != 0 {
            let count = uc.read_u32(readfds_ptr as u64) as usize;
            let count = count.min(64);
            let mut ready_socks = Vec::new();

            let ctx = uc.get_data();
            for i in 0..count {
                let sock = uc.read_u32(readfds_ptr as u64 + 4 + (i * 4) as u64);
                let mut sockets = ctx.tcp_sockets.lock().unwrap();
                if let Some(s) = sockets.get_mut(&sock) {
                    if !s.recv_buf.is_empty() {
                        ready_socks.push(sock);
                        continue;
                    }
                    if let Some(chan_rx) = s.chan_rx.as_mut() {
                        match chan_rx.try_recv() {
                            Ok(data) => {
                                // 데이터를 recv_buf에 보관 후 readable로 표시
                                s.recv_buf.extend(data);
                                ready_socks.push(sock);
                            }
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                ready_socks.push(sock); // EOF도 readable
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        }
                    }
                }
            }

            // 결과 반영: fd_set 업데이트
            uc.write_u32(readfds_ptr as u64, ready_socks.len() as u32);
            for (i, s) in ready_socks.iter().enumerate() {
                uc.write_u32(readfds_ptr as u64 + 4 + (i * 4) as u64, *s);
            }
            total_ready += ready_socks.len() as i32;
        }

        if writefds_ptr != 0 {
            let count = uc.read_u32(writefds_ptr as u64) as usize;
            let count = count.min(64);
            let mut ready_socks = Vec::new();

            let ctx = uc.get_data();
            for i in 0..count {
                let sock = uc.read_u32(writefds_ptr as u64 + 4 + (i * 4) as u64);
                let sockets = ctx.tcp_sockets.lock().unwrap();
                if let Some(s) = sockets.get(&sock)
                    && s.connected
                {
                    ready_socks.push(sock);
                }
            }

            uc.write_u32(writefds_ptr as u64, ready_socks.len() as u32);
            for (i, s) in ready_socks.iter().enumerate() {
                uc.write_u32(writefds_ptr as u64 + 4 + (i * 4) as u64, *s);
            }
            total_ready += ready_socks.len() as i32;
        }

        if exceptfds_ptr != 0 {
            // 현재 에뮬레이션 환경에서는 OOB 데이터나 non-blocking connect 실패 시점을
            // 정밀하게 추적/지원하지 않으므로 일단 예외(exception) 상태는 발생하지 않는 것으로 취급합니다.
            // fd_set 구조체의 count 필드를 0으로 클리어하여 애플리케이션에서 오작동(WSAFDIsSet 1 반환)을 방지합니다.
            uc.write_u32(exceptfds_ptr as u64, 0);
        }

        let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        if total_ready == 0 {
            if timeout.is_some_and(|duration| duration.is_zero()) {
                KERNEL32::clear_retry_wait(uc.get_data(), tid);
                return Some(ApiHookResult::callee(5, Some(0)));
            }

            let now = Instant::now();
            let deadline = timeout.and_then(|duration| {
                KERNEL32::current_wait_deadline(uc.get_data(), tid).or(Some(now + duration))
            });

            if let Some(limit) = deadline
                && now >= limit
            {
                KERNEL32::clear_retry_wait(uc.get_data(), tid);
                return Some(ApiHookResult::callee(5, Some(0)));
            }

            KERNEL32::schedule_retry_wait(uc.get_data(), tid, deadline);
            return Some(ApiHookResult::retry());
        }

        KERNEL32::clear_retry_wait(uc.get_data(), tid);

        // if total_ready > 0 {
        //     crate::emu_socket_log!("[SELECT] total_ready={}", total_ready);
        // }
        Some(ApiHookResult::callee(5, Some(total_ready)))
    }

    // API: int send(SOCKET s, const char* buf, int len, int flags)
    // 역할: Ordinal_19 - 실제 TcpStream에 데이터 전송
    pub fn send(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let len = uc.read_arg(2) as usize;
        let flags = uc.read_arg(3);

        if let Some(result) = return_unsupported_flags(uc, "send", flags, 0, 4) {
            return Some(result);
        }

        if len == 0 {
            return Some(ApiHookResult::callee(4, Some(0)));
        }

        let data = uc.mem_read_as_vec(buf_addr as u64, len).unwrap_or_default();
        let ctx = uc.get_data();

        let mut sockets = ctx.tcp_sockets.lock().unwrap();
        let socket = match sockets.get_mut(&sock) {
            Some(s) => s,
            None => {
                ctx.last_error.store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR (no socket)", sock);
                crate::emu_socket_log!("[CLIENT] sock={} FAIL: no socket", sock);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };

        let chan_tx = match socket.chan_tx.as_ref() {
            Some(tx) => tx.clone(),
            None => {
                drop(sockets);
                ctx.last_error.store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR (not connected)", sock);
                crate::emu_socket_log!("[CLIENT] sock={} FAIL: not connected", sock);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };
        drop(sockets);

        if chan_tx.send(data.clone()).is_ok() {
            ctx.packet_logger
                .lock()
                .unwrap()
                .log(PacketDirection::Send, sock, &data, true);
            crate::emu_log!(
                "[WS2_32] send({}, {:#x}, {}, {}) -> {} bytes",
                sock,
                buf_addr,
                len,
                flags,
                len
            );
            crate::emu_socket_log!("[CLIENT] sock={} -> {} bytes", sock, len);
            Some(ApiHookResult::callee(4, Some(len as i32)))
        } else {
            crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR (channel closed)", sock);
            crate::emu_socket_log!("[CLIENT] sock={} FAIL: channel closed", sock);
            Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
        }
    }

    // API: int setsockopt(SOCKET s, int level, int optname, const char* optval, int optlen)
    // 역할: Ordinal_21 - 소켓 옵션 설정 (주요 옵션만 처리)
    pub fn setsockopt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let level = uc.read_arg(1);
        let optname = uc.read_arg(2);
        let optval = uc.read_arg(3);
        let optlen = uc.read_arg(4);
        crate::emu_log!(
            "[WS2_32] setsockopt({}, level={}, optname={}, {:#x}, {}) -> 0",
            sock,
            level,
            optname,
            optval,
            optlen
        );
        Some(ApiHookResult::callee(5, Some(0)))
    }

    // API: int shutdown(SOCKET s, int how)
    // 역할: Ordinal_22 - 소켓의 송수신 기능을 중단
    pub fn shutdown(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let how = uc.read_arg(1);
        // 실제로 TcpStream을 끊지는 않고 closesocket에서 처리
        crate::emu_log!("[WS2_32] shutdown({}, how={}) -> 0", sock, how);
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: SOCKET socket(int af, int type, int protocol)
    // 역할: Ordinal_23 - 새 TokioSocket 생성
    pub fn socket(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let af = uc.read_arg(0);
        let sock_type = uc.read_arg(1);
        let protocol = uc.read_arg(2);
        let ctx = uc.get_data();
        let sock = ctx.alloc_handle();
        ctx.tcp_sockets.lock().unwrap().insert(
            sock,
            VirtualSocket {
                af,
                sock_type,
                protocol,
                chan_tx: None,
                chan_rx: None,
                connected: false,
                recv_buf: Vec::new(),
                non_blocking: false,
                remote_addr: None,
            },
        );
        crate::emu_log!(
            "[WS2_32] socket(af={}, type={}, proto={}) -> SOCKET {:#x}",
            af,
            sock_type,
            protocol,
            sock
        );
        crate::emu_socket_log!(
            "[SOCK] created sock={} af={} type={} proto={}",
            sock,
            af,
            sock_type,
            protocol
        );
        Some(ApiHookResult::callee(3, Some(sock as i32)))
    }

    // API: struct hostent* gethostbyname(const char* name)
    // 역할: Ordinal_52 - 실제 DNS 조회로 호스트 이름 해석
    pub fn gethostbyname(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_euc_kr(name_addr as u64);

        use std::net::ToSocketAddrs;
        let resolved_ip = format!("{}:80", name)
            .to_socket_addrs()
            .ok()
            .and_then(|mut iter| iter.next())
            .map(|addr| match addr.ip() {
                std::net::IpAddr::V4(v4) => v4.octets(),
                std::net::IpAddr::V6(_) => [127, 0, 0, 1],
            })
            .unwrap_or([127, 0, 0, 1]);

        crate::emu_log!(
            "[WS2_32] gethostbyname(\"{}\") -> {}.{}.{}.{}",
            name,
            resolved_ip[0],
            resolved_ip[1],
            resolved_ip[2],
            resolved_ip[3]
        );
        crate::emu_socket_log!(
            "[DNS] \"{}\" -> {}.{}.{}.{}",
            name,
            resolved_ip[0],
            resolved_ip[1],
            resolved_ip[2],
            resolved_ip[3]
        );

        // hostent 구조체를 에뮬 메모리에 할당 (16 bytes)
        let hostent_addr = uc.malloc(16);
        let ip_data = uc.malloc(4);
        uc.mem_write(ip_data, &resolved_ip).unwrap();
        let ip_ptr = uc.malloc(8);
        uc.write_u32(ip_ptr, ip_data as u32);
        uc.write_u32(ip_ptr + 4, 0); // NULL 종료

        let name_str = uc.alloc_str(&name);
        uc.write_u32(hostent_addr, name_str); // h_name
        uc.write_u32(hostent_addr + 4, 0); // h_aliases
        uc.write_u16(hostent_addr + 8, 2); // h_addrtype (AF_INET)
        uc.write_u16(hostent_addr + 10, 4); // h_length (IPv4)
        uc.write_u32(hostent_addr + 12, ip_ptr as u32); // h_addr_list

        Some(ApiHookResult::callee(1, Some(hostent_addr as i32)))
    }

    // API: int WSAGetLastError(void)
    // 역할: Ordinal_111 - 마지막으로 발생한 네트워크 오류 코드를 반환
    pub fn wsa_get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let err = uc.get_data().last_error.load(Ordering::SeqCst);
        crate::emu_log!("[WS2_32] WSAGetLastError() -> {}", err);
        Some(ApiHookResult::callee(0, Some(err as i32)))
    }

    // API: int WSAStartup(WORD wVersionRequested, LPWSADATA lpWSAData)
    // 역할: Ordinal_115 - Winsock 라이브러리를 초기화
    pub fn wsa_startup(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let version = uc.read_arg(0);
        let wsa_data_addr = uc.read_arg(1);

        if wsa_data_addr != 0 {
            let zeros = vec![0u8; 394];
            uc.mem_write(wsa_data_addr as u64, &zeros).unwrap();
            uc.mem_write(wsa_data_addr as u64, &[2, 2]).unwrap(); // wVersion
            uc.mem_write(wsa_data_addr as u64 + 2, &[2, 2]).unwrap(); // wHighVersion
        }

        crate::emu_log!(
            "[WS2_32] WSAStartup({:#x}, {:#x}) -> 0",
            version,
            wsa_data_addr
        );
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: int WSACleanup(void)
    // 역할: Ordinal_116 - Winsock 라이브러리 사용을 종료
    pub fn wsa_cleanup(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        crate::emu_log!("[WS2_32] WSACleanup() -> 0");
        Some(ApiHookResult::callee(0, Some(0)))
    }

    // API: int __WSAFDIsSet(SOCKET fd, fd_set* set)
    // 역할: Ordinal_151 - 소켓이 fd_set에 포함되어 있는지 확인
    pub fn wsa_fd_is_set(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let set_ptr = uc.read_arg(1);
        if set_ptr == 0 {
            return Some(ApiHookResult::callee(2, Some(0)));
        }

        let count = uc.read_u32(set_ptr as u64) as usize;
        let count = count.min(64); // FD_SETSIZE
        for i in 0..count {
            let s = uc.read_u32(set_ptr as u64 + 4 + (i * 4) as u64);
            if s == sock {
                return Some(ApiHookResult::callee(2, Some(1)));
            }
        }
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: int WSASend(SOCKET s, LPWSABUF lpBuffers, DWORD dwBufferCount, LPDWORD lpNumberOfBytesSent, DWORD dwFlags, LPWSAOVERLAPPED lpOverlapped, LPWSAOVERLAPPED_COMPLETION_ROUTINE lpCompletionRoutine)
    // 역할: WSABuf 배열에서 데이터를 읽어 실제로 전송
    pub fn wsa_send(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let bufs_addr = uc.read_arg(1);
        let buf_count = uc.read_arg(2);
        let bytes_sent_addr = uc.read_arg(3);
        let flags = uc.read_arg(4);
        let _overlapped = uc.read_arg(5);
        let _completion_routine = uc.read_arg(6);

        if let Some(result) = return_unsupported_flags(uc, "WSASend", flags, 0, 7) {
            return Some(result);
        }

        let mut total_sent = 0usize;
        for i in 0..buf_count {
            // WSABUF: len(4) + buf(4 ptr)
            let offset = (i * 8) as u64;
            let buf_len = uc.read_u32(bufs_addr as u64 + offset) as usize;
            let buf_ptr = uc.read_u32(bufs_addr as u64 + offset + 4);

            if buf_len == 0 || buf_ptr == 0 {
                continue;
            }
            let data = uc
                .mem_read_as_vec(buf_ptr as u64, buf_len)
                .unwrap_or_default();
            let ctx = uc.get_data();

            let mut sockets = ctx.tcp_sockets.lock().unwrap();
            if let Some(s) = sockets.get_mut(&sock)
                && let Some(chan_tx) = s.chan_tx.as_ref()
                && chan_tx.send(data.clone()).is_ok()
            {
                total_sent += buf_len;
            }
            drop(sockets);
        }

        if bytes_sent_addr != 0 {
            uc.write_u32(bytes_sent_addr as u64, total_sent as u32);
        }
        crate::emu_log!("[WS2_32] WSASend({:#x}) -> {} bytes sent", sock, total_sent);
        crate::emu_socket_log!("[CLIENT] sock={} -> {} bytes (WSA)", sock, total_sent);
        Some(ApiHookResult::callee(
            7,
            Some(if total_sent > 0 { 0 } else { SOCKET_ERROR }),
        ))
    }

    // API: int WSARecv(SOCKET s, LPWSABUF lpBuffers, DWORD dwBufferCount, LPDWORD lpNumberOfBytesRecvd, LPDWORD lpFlags, LPWSAOVERLAPPED lpOverlapped, LPWSAOVERLAPPED_COMPLETION_ROUTINE lpCompletionRoutine)
    // 역할: WSABuf 배열에 데이터를 수신하여 기록
    pub fn wsa_recv(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let bufs_addr = uc.read_arg(1);
        let buf_count = uc.read_arg(2);
        let bytes_recvd_addr = uc.read_arg(3);
        let flags_ptr = uc.read_arg(4);
        let flags = if flags_ptr != 0 {
            uc.read_u32(flags_ptr as u64)
        } else {
            0
        };

        if let Some(result) = return_unsupported_flags(uc, "WSARecv", flags, MSG_PEEK, 7) {
            return Some(result);
        }
        let peek = should_peek(flags);

        crate::emu_socket_log!(
            "[WS2_32] WSARecv({:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
            sock,
            bufs_addr,
            buf_count,
            bytes_recvd_addr,
            flags_ptr
        );

        if buf_count == 0 {
            return Some(ApiHookResult::callee(7, Some(0)));
        }

        let mut bufs = Vec::new();
        let mut total_requested = 0usize;
        for i in 0..buf_count {
            let offset = (i * 8) as u64;
            let blen = uc.read_u32(bufs_addr as u64 + offset) as usize;
            let bptr = uc.read_u32(bufs_addr as u64 + offset + 4);
            if blen > 0 && bptr != 0 {
                bufs.push((bptr, blen));
                total_requested += blen;
            }
        }

        if total_requested == 0 {
            if bytes_recvd_addr != 0 {
                uc.write_u32(bytes_recvd_addr as u64, 0);
            }
            return Some(ApiHookResult::callee(7, Some(0)));
        }

        let (tcp_sockets, packet_logger) = {
            let ctx = uc.get_data();
            (ctx.tcp_sockets.clone(), ctx.packet_logger.clone())
        };

        let mut sockets = tcp_sockets.lock().unwrap();
        let socket = match sockets.get_mut(&sock) {
            Some(s) => s,
            None => {
                drop(sockets);
                uc.get_data()
                    .last_error
                    .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] WSARecv({}) -> SOCKET_ERROR (no socket)", sock);
                crate::emu_socket_log!("[SERVER] sock={} FAIL: no socket", sock);
                return Some(ApiHookResult::callee(7, Some(SOCKET_ERROR)));
            }
        };

        let data_to_distribute = match recv_pending_data(socket, total_requested, peek) {
            Ok(data) => data,
            Err(std::sync::mpsc::TryRecvError::Empty) => Vec::new(),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        let total_n = data_to_distribute.len();
        drop(sockets);

        if total_n == 0 {
            let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
            if tid != 0 {
                KERNEL32::schedule_retry_wait(uc.get_data(), tid, None);
                return Some(ApiHookResult::retry());
            }

            KERNEL32::schedule_retry_wait(uc.get_data(), tid, None);
            return Some(ApiHookResult::retry());
        }

        KERNEL32::clear_retry_wait(
            uc.get_data(),
            uc.get_data().current_thread_idx.load(Ordering::SeqCst),
        );

        // 데이터 분배
        let mut curr = 0usize;
        for (bptr, blen) in bufs {
            if curr >= total_n {
                break;
            }
            let take = (total_n - curr).min(blen);
            uc.mem_write(bptr as u64, &data_to_distribute[curr..curr + take])
                .ok();
            curr += take;
        }

        packet_logger
            .lock()
            .unwrap()
            .log(PacketDirection::Recv, sock, &data_to_distribute, !peek);

        if bytes_recvd_addr != 0 {
            uc.write_u32(bytes_recvd_addr as u64, total_n as u32);
        }
        if flags_ptr != 0 {
            // 현재 에뮬레이터는 MSG_PARTIAL 같은 출력 플래그를 생성하지 않으므로 0으로 돌려줍니다.
            uc.write_u32(flags_ptr as u64, 0);
        }

        // crate::emu_socket_log!(
        //     "[SERVER] sock={} -> {} bytes (WSA Multi{})",
        //     sock,
        //     total_n,
        //     if peek { ", peek" } else { "" }
        // );

        Some(ApiHookResult::callee(7, Some(0)))
    }

    // API: SOCKET WSASocketA(int af, int type, int protocol, LPWSAPROTOCOL_INFOA lpProtocolInfo, GROUP g, DWORD dwFlags)
    // 역할: 새 소켓을 생성 (확장 기능 포함)
    pub fn wsa_socket_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let af = uc.read_arg(0);
        let sock_type = uc.read_arg(1);
        let protocol = uc.read_arg(2);
        let _protocol_info = uc.read_arg(3);
        let _group = uc.read_arg(4);
        let flags = uc.read_arg(5);

        if let Some(result) = return_unsupported_flags(
            uc,
            "WSASocketA",
            flags,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT,
            6,
        ) {
            return Some(result);
        }
        let ctx = uc.get_data();
        let sock = ctx.alloc_handle();
        ctx.tcp_sockets.lock().unwrap().insert(
            sock,
            VirtualSocket {
                af,
                sock_type,
                protocol,
                chan_tx: None,
                chan_rx: None,
                connected: false,
                recv_buf: Vec::new(),
                non_blocking: false,
                remote_addr: None,
            },
        );
        crate::emu_log!(
            "[WS2_32] WSASocketA(af={}, type={}, proto={}) -> SOCKET {:#x}",
            af,
            sock_type,
            protocol,
            sock
        );
        crate::emu_socket_log!(
            "[SOCK] created(WSA) sock={} af={} type={} proto={}",
            sock,
            af,
            sock_type,
            protocol
        );
        Some(ApiHookResult::callee(6, Some(sock as i32)))
    }

    // API: WSAEVENT WSACreateEvent(void)
    // 역할: 새 이벤트 개체를 생성
    pub fn wsa_create_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!("[WS2_32] WSACreateEvent() -> {:#x}", handle);
        Some(ApiHookResult::callee(0, Some(handle as i32)))
    }

    // API: int WSAEventSelect(SOCKET s, WSAEVENT hEventObject, long lNetworkEvents)
    // 역할: 소켓 이벤트를 이벤트 개체와 연결
    pub fn wsa_event_select(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let event = uc.read_arg(1);
        let network_events = uc.read_arg(2);

        let ctx = uc.get_data();

        // 이미 연결된 소켓이면 FD_CONNECT(0x10)를 즉시 pending으로 설정
        let already_connected = ctx
            .tcp_sockets
            .lock()
            .unwrap()
            .get(&sock)
            .map(|s| s.connected)
            .unwrap_or(false);

        let initial_pending = if already_connected && network_events & 0x10 != 0 {
            0x10 // FD_CONNECT
        } else {
            0
        };

        ctx.wsa_event_map.lock().unwrap().insert(
            event,
            WsaEventEntry {
                socket: sock,
                interest: network_events,
                pending: initial_pending,
            },
        );

        crate::emu_log!(
            "[WS2_32] WSAEventSelect(sock={:#x}, event={:#x}, mask={:#x}) -> 0 (pending={:#x})",
            sock,
            event,
            network_events,
            initial_pending
        );
        Some(ApiHookResult::callee(3, Some(0)))
    }

    // API: BOOL WSACloseEvent(WSAEVENT hEvent)
    // 역할: 이벤트 개체를 닫음
    pub fn wsa_close_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let event = uc.read_arg(0);
        crate::emu_log!("[WS2_32] WSACloseEvent({:#x}) -> TRUE", event);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: int WSAEnumNetworkEvents(SOCKET s, WSAEVENT hEventObject, LPWSANETWORKEVENTS lpNetworkEvents)
    // 역할: 특정 소켓에서 발생한 네트워크 이벤트를 확인하고 WSANETWORKEVENTS 구조체에 기록
    pub fn wsa_enum_network_events(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let _sock = uc.read_arg(0);
        let event = uc.read_arg(1);
        let net_events_addr = uc.read_arg(2);

        // Arc를 먼저 클론하여 ctx 빌림 해제 후 uc 메모리 조작 가능하게 함
        let (wsa_event_map, tcp_sockets) = {
            let ctx = uc.get_data();
            (ctx.wsa_event_map.clone(), ctx.tcp_sockets.clone())
        };

        // wsa_event_map에서 이 이벤트 핸들의 소켓과 pending 이벤트를 가져옴
        let (sock, interest, mut pending) = {
            let map = wsa_event_map.lock().unwrap();
            if let Some(e) = map.get(&event) {
                (e.socket, e.interest, e.pending)
            } else {
                // 알 수 없는 이벤트 → 빈 결과
                if net_events_addr != 0 {
                    uc.mem_write(net_events_addr as u64, &[0u8; 44]).ok();
                }
                return Some(ApiHookResult::callee(3, Some(0)));
            }
        };

        // 소켓 현재 상태 기반으로 추가 이벤트 감지 (pending에 누적)
        {
            let mut sockets = tcp_sockets.lock().unwrap();
            if let Some(s) = sockets.get_mut(&sock) {
                // FD_READ 체크 전에 채널에 대기 중인 데이터를 recv_buf로 드레인
                if let Some(chan_rx) = s.chan_rx.as_mut() {
                    while let Ok(data) = chan_rx.try_recv() {
                        s.recv_buf.extend(data);
                    }
                }
                // FD_CONNECT(0x10): 채널이 연결되어 있으면
                if interest & 0x10 != 0 && s.connected {
                    pending |= 0x10;
                }
                // FD_READ(0x01): recv_buf에 데이터가 있으면
                if interest & 0x01 != 0 && !s.recv_buf.is_empty() {
                    pending |= 0x01;
                }
                // FD_WRITE(0x02): 연결된 소켓은 항상 쓰기 가능
                if interest & 0x02 != 0 && s.connected {
                    pending |= 0x02;
                }
            }
        }

        // pending 이벤트를 관심 마스크로 필터링
        let l_network_events = pending & interest;

        // WSANETWORKEVENTS 구조체 기록:
        // [0..4]  lNetworkEvents (u32)
        // [4..44] iErrorCode[10] (모두 0)
        if net_events_addr != 0 {
            uc.mem_write(net_events_addr as u64, &[0u8; 44]).ok();
            uc.write_u32(net_events_addr as u64, l_network_events);
        }

        // pending 클리어 (소비됨)
        {
            let mut map = wsa_event_map.lock().unwrap();
            if let Some(e) = map.get_mut(&event) {
                e.pending = 0;
            }
        }

        crate::emu_log!(
            "[WS2_32] WSAEnumNetworkEvents(sock={:#x}, event={:#x}) -> lNetworkEvents={:#x}",
            sock,
            event,
            l_network_events
        );
        Some(ApiHookResult::callee(3, Some(0)))
    }

    /// 함수명 기준 `WS2_32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        // 각 API 구현체가 자체 로그를 남기므로 디스패치 레벨 중복 로그는 생략합니다.
        match func_name {
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
            "Ordinal_14" | "ntohl" => Self::ntohl(uc),
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
            "WSARecv" => Self::wsa_recv(uc),
            "WSASocketA" => Self::wsa_socket_a(uc),
            "WSACreateEvent" => Self::wsa_create_event(uc),
            "WSAEventSelect" => Self::wsa_event_select(uc),
            "WSACloseEvent" => Self::wsa_close_event(uc),
            "WSAEnumNetworkEvents" => Self::wsa_enum_network_events(uc),
            _ => {
                crate::emu_log!("[!] WS2_32 Unhandled: {}", func_name);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;

    fn make_socket_with_message(msg: &[u8]) -> VirtualSocket {
        let (tx, rx) = mpsc::channel();
        tx.send(msg.to_vec()).unwrap();
        drop(tx);

        VirtualSocket {
            af: 2,
            sock_type: 1,
            protocol: 6,
            chan_tx: None,
            chan_rx: Some(rx),
            connected: true,
            recv_buf: Vec::new(),
            non_blocking: false,
            remote_addr: None,
        }
    }

    #[test]
    fn recv_pending_data_peek_keeps_message_in_queue_buffer() {
        let mut socket = make_socket_with_message(b"hello");

        let first = recv_pending_data(&mut socket, 2, true).unwrap();
        assert_eq!(first, b"he");
        assert_eq!(socket.recv_buf, b"hello");

        let second = recv_pending_data(&mut socket, 5, false).unwrap();
        assert_eq!(second, b"hello");
        assert!(socket.recv_buf.is_empty());
    }

    #[test]
    fn recv_pending_data_without_peek_consumes_and_keeps_leftover() {
        let mut socket = make_socket_with_message(b"hello");

        let data = recv_pending_data(&mut socket, 2, false).unwrap();
        assert_eq!(data, b"he");
        assert_eq!(socket.recv_buf, b"llo");
    }

    #[test]
    fn unsupported_flag_bits_reports_only_unknown_bits() {
        assert_eq!(unsupported_flag_bits(MSG_PEEK, MSG_PEEK), 0);
        assert_eq!(
            unsupported_flag_bits(
                WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT,
                WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT
            ),
            0
        );
    }

    #[test]
    fn select_timeout_duration_preserves_infinite_wait_for_null_timeout() {
        assert_eq!(select_timeout_duration(None), None);
    }

    #[test]
    fn select_timeout_duration_parses_timeval_values() {
        assert_eq!(
            select_timeout_duration(Some((1, 500_000))),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(select_timeout_duration(Some((0, 0))), Some(Duration::ZERO));
    }
}
