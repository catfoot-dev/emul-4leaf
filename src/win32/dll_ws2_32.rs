use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::server::packet_logger::PacketDirection;
use crate::win32::{ApiHookResult, TokioSocket, Win32Context, WsaEventEntry};

/// `WS2_32.dll` 프록시 구현 모듈
///
/// Tokio 기반의 실제 TCP 소켓 I/O를 에뮬레이션합니다.
/// 에뮬레이터 훅은 동기 컨텍�/// WSAEWOULDBLOCK - 논블로킹 소켓의 '지금 바로 처리 불가' 오류 코드입니다.
const WSAEWOULDBLOCK: u32 = 10035;
/// WSAETIMEDOUT - 연결 시간 초과 오류 코드입니다.
const WSAETIMEDOUT: u32 = 10060;
/// WSAECONNREFUSED - 연결 거부 오류 코드입니다.
const WSAECONNREFUSED: u32 = 10061;
/// Winsock 표준 소켓 오류 반환값입니다.
const SOCKET_ERROR: i32 = -1;
/// ioctlsocket 함수용 FIONBIO 명령어 코드입니다.
const FIONBIO: u32 = 0x8004667E;

/// 비동기 퓨처(Future)를 현재 스레드에서 동기적으로 실행합니다.
/// win32::get_tokio_runtime()의 공유 런타임을 사용합니다.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    crate::win32::get_tokio_runtime().block_on(f)
}

pub struct DllWS2_32 {}

impl DllWS2_32 {
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
        crate::emu_socket_log!("[ACCEPT] sock={}", sock);
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
    /// 실제 호스트의 `tokio::net::TcpStream::connect`를 호출하여 원격 주소와 연결을 수립합니다.
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
        crate::emu_socket_log!("[CONN] sock={} connecting to {}", sock, addr_str);

        let result = block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::net::TcpStream::connect(&addr_str),
            )
            .await
        });

        let ctx = uc.get_data();
        match result {
            Ok(Ok(stream)) => {
                let non_blocking = ctx
                    .tcp_sockets
                    .lock()
                    .unwrap()
                    .get(&sock)
                    .map(|s| s.non_blocking)
                    .unwrap_or(false);

                ctx.tcp_sockets.lock().unwrap().insert(
                    sock,
                    TokioSocket {
                        af: 2,        // AF_INET
                        sock_type: 1, // SOCK_STREAM
                        protocol: 6,  // IPPROTO_TCP
                        stream: Some(stream),
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
                crate::emu_log!("[WS2_32] connect({}, \"{}\") -> OK", sock, addr_str);
                crate::emu_socket_log!("[CONN] sock={} -> {} OK", sock, addr_str);
                Some(ApiHookResult::callee(3, Some(0)))
            }
            Ok(Err(e)) => {
                let code = if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    WSAECONNREFUSED
                } else {
                    WSAEWOULDBLOCK
                };
                ctx.last_error.store(code, Ordering::SeqCst);
                crate::emu_log!(
                    "[WS2_32] connect({}, \"{}\") -> FAIL: {}",
                    sock,
                    addr_str,
                    e
                );
                crate::emu_socket_log!("[CONN] sock={} -> {} FAIL: {}", sock, addr_str, e);
                Some(ApiHookResult::callee(3, Some(SOCKET_ERROR)))
            }
            Err(_) => {
                ctx.last_error.store(WSAETIMEDOUT, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] connect({}, \"{}\") -> TIMEOUT", sock, addr_str);
                crate::emu_socket_log!("[CONN] sock={} -> {} FAIL: TIMEOUT", sock, addr_str);
                Some(ApiHookResult::callee(3, Some(SOCKET_ERROR)))
            }
        }
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

        if let Some(addr) = remote {
            if let Ok(sockaddr) = addr.parse::<std::net::SocketAddr>() {
                if let std::net::SocketAddr::V4(v4) = sockaddr {
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
                    crate::emu_socket_log!("[GETPEERNAME] sock={} -> {} OK", sock, addr);
                    return Some(ApiHookResult::callee(3, Some(0)));
                }
            }
        }
        crate::emu_log!(
            "[WS2_32] getpeername({}) -> SOCKET_ERROR (not connected)",
            sock
        );
        crate::emu_socket_log!("[GETPEERNAME] sock={}", sock);
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
                    TokioSocket {
                        af: 2,
                        sock_type: 1,
                        protocol: 6,
                        stream: None,
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
            crate::emu_socket_log!("[IOCTL] sock={} cmd={:#x} argp={:#x}", sock, cmd, argp);
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
        crate::emu_socket_log!("[INET_ADDR] addr_str={} -> {:#x}", addr_str, result);
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
        crate::emu_socket_log!("[INET_NTOA] addr={:#x} -> {:#x}=\"{}\"", addr, ptr, ip_str);
        Some(ApiHookResult::callee(1, Some(ptr as i32)))
    }

    // API: int listen(SOCKET s, int backlog)
    // 역할: Ordinal_13 - 소켓을 수신 모드로 설정
    pub fn listen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let backlog = uc.read_arg(1);
        crate::emu_log!("[WS2_32] listen({}, {}) -> int 0 (stub)", sock, backlog);
        crate::emu_socket_log!("[LISTEN] sock={} backlog={}", sock, backlog);
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: u_long ntohl(u_long netlong)
    // 역할: Ordinal_14 - 32비트 네트워크 바이트 순서를 호스트 바이트 순서로 변환
    pub fn ntohl(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let val = uc.read_arg(0);
        let result = u32::from_be(val);
        crate::emu_log!("[WS2_32] ntohl({:#x}) -> u_long {:#x}", val, result);
        crate::emu_socket_log!("[NTOHL] val={:#x} -> {:#x}", val, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: u_short ntohs(u_short netshort)
    // 역할: Ordinal_15 - 16비트 네트워크 바이트 순서를 호스트 바이트 순서로 변환
    pub fn ntohs(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let val = uc.read_arg(0) as u16;
        let result = u16::from_be(val);
        crate::emu_log!("[WS2_32] ntohs({}) -> u_short {}", val, result);
        crate::emu_socket_log!("[NTOHS] val={} -> {}", val, result);
        Some(ApiHookResult::callee(1, Some(result as i32)))
    }

    // API: int recv(SOCKET s, char* buf, int len, int flags)
    // 역할: Ordinal_16 - 실제 TcpStream에서 데이터를 수신
    pub fn recv(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let len = uc.read_arg(2) as usize;
        let flags = uc.read_arg(3);

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
                crate::emu_socket_log!("[RECV] sock={} FAIL: no socket", sock);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };

        // 1. 기존 recv_buf에서 먼저 소비
        if !socket.recv_buf.is_empty() {
            let take = len.min(socket.recv_buf.len());
            let data: Vec<u8> = socket.recv_buf.drain(..take).collect();
            drop(sockets);
            uc.mem_write(buf_addr as u64, &data).ok();
            packet_logger
                .lock()
                .unwrap()
                .log(PacketDirection::Recv, sock, &data);
            crate::emu_log!(
                "[WS2_32] recv({}, {:#x}, {}) -> {} (from buf)",
                sock,
                buf_addr,
                len,
                take
            );
            crate::emu_socket_log!("[RECV] sock={} -> {} (from buf)", sock, take);
            return Some(ApiHookResult::callee(4, Some(take as i32)));
        }

        let non_blocking = socket.non_blocking;
        let stream = match socket.stream.as_mut() {
            Some(s) => s,
            None => {
                drop(sockets);
                uc.get_data()
                    .last_error
                    .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };

        if non_blocking {
            // 논블로킹: try_read 시도
            let mut tmp = vec![0u8; len];
            match stream.try_read(&mut tmp) {
                Ok(0) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> 0 (connection closed)", sock);
                    crate::emu_socket_log!("[RECV] sock={} FAIL: connection closed", sock);
                    Some(ApiHookResult::callee(4, Some(0)))
                }
                Ok(n) => {
                    let data = tmp[..n].to_vec();
                    drop(sockets);
                    uc.mem_write(buf_addr as u64, &data).ok();
                    packet_logger
                        .lock()
                        .unwrap()
                        .log(PacketDirection::Recv, sock, &data);
                    crate::emu_log!(
                        "[WS2_32] recv({}, {:#x}, {}, {}) -> {} bytes (non-blocking)",
                        sock,
                        buf_addr,
                        len,
                        flags,
                        n
                    );
                    crate::emu_socket_log!("[RECV] sock={} -> {} (non-blocking)", sock, n);
                    Some(ApiHookResult::callee(4, Some(n as i32)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    drop(sockets);
                    let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
                    if tid != 0 {
                        return Some(ApiHookResult::retry());
                    }
                    uc.get_data()
                        .last_error
                        .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                    crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR (WSAEWOULDBLOCK)", sock);
                    crate::emu_socket_log!("[RECV] sock={} FAIL: WouldBlock", sock);
                    Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
                }
                Err(e) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR ({})", sock, e);
                    crate::emu_socket_log!("[RECV] sock={} FAIL: {}", sock, e);
                    Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
                }
            }
        } else {
            // 블로킹: try_read 시도 후 WouldBlock이면 retry
            let mut tmp = vec![0u8; len];
            match stream.try_read(&mut tmp) {
                Ok(0) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> 0 (connection closed)", sock);
                    crate::emu_socket_log!("[RECV] sock={} FAIL: connection closed", sock);
                    Some(ApiHookResult::callee(4, Some(0)))
                }
                Ok(n) => {
                    let data = tmp[..n].to_vec();
                    drop(sockets);
                    uc.mem_write(buf_addr as u64, &data).ok();
                    packet_logger
                        .lock()
                        .unwrap()
                        .log(PacketDirection::Recv, sock, &data);
                    crate::emu_log!(
                        "[WS2_32] recv({}, {:#x}, {}, {}) -> {} bytes (blocking-simulated)",
                        sock,
                        buf_addr,
                        len,
                        flags,
                        n
                    );
                    crate::emu_socket_log!("[RECV] sock={} -> {} (blocking-simulated)", sock, n);
                    Some(ApiHookResult::callee(4, Some(n as i32)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    drop(sockets);
                    let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
                    if tid != 0 {
                        // 스케줄러가 체크할 수 있게 retry 리턴
                        return Some(ApiHookResult::retry());
                    }
                    uc.get_data()
                        .last_error
                        .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                    Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
                }
                Err(e) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR ({})", sock, e);
                    crate::emu_socket_log!("[RECV] sock={} FAIL: {}", sock, e);
                    Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
                }
            }
        }
    }

    // API: int select(int nfds, fd_set* readfds, fd_set* writefds, fd_set* exceptfds, const struct timeval* timeout)
    // 역할: Ordinal_18 - 소켓 읽기/쓰기 가능 여부를 확인
    pub fn select(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let _nfds = uc.read_arg(0);
        let readfds_ptr = uc.read_arg(1);
        let writefds_ptr = uc.read_arg(2);
        let _exceptfds = uc.read_arg(3);
        let timeout_ptr = uc.read_arg(4);

        // timeval 구조체 파싱: tv_sec(4) + tv_usec(4)
        let timeout_us = if timeout_ptr != 0 {
            let sec = uc.read_u32(timeout_ptr as u64) as u64;
            let usec = uc.read_u32(timeout_ptr as u64 + 4) as u64;
            sec * 1_000_000 + usec
        } else {
            500_000 // 기본 500ms 타임아웃
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
                    // 읽기 가능 여부 확인: 실제 수신 시도 후 recv_buf에 선취 버퍼링
                    let mut tmp = vec![0u8; 256];
                    let result = s.stream.as_ref().map(|st| st.try_read(&mut tmp));
                    match result {
                        Some(Ok(0)) => { ready_socks.push(sock); }
                        Some(Ok(n)) => {
                            s.recv_buf.extend_from_slice(&tmp[..n]);
                            ready_socks.push(sock);
                        }
                        _ => {}
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
                if let Some(s) = sockets.get(&sock) {
                    if s.stream.is_some() {
                        ready_socks.push(sock);
                    }
                }
            }

            uc.write_u32(writefds_ptr as u64, ready_socks.len() as u32);
            for (i, s) in ready_socks.iter().enumerate() {
                uc.write_u32(writefds_ptr as u64 + 4 + (i * 4) as u64, *s);
            }
            total_ready += ready_socks.len() as i32;
        }

        let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        if tid != 0 {
            let ctx = uc.get_data();
            let mut threads = ctx.threads.lock().unwrap();
            if let Some(t) = threads.iter_mut().find(|th| th.thread_id == tid) {
                if total_ready == 0 {
                    // 타임아웃이 u64::MAX인 경우(무한 대기)는 resume_time을 설정하지 않아
                    // 매 루프마다 소켓 상태를 체크하고 terminate_requested를 확인할 수 있게 함
                    if timeout_us > 0 && timeout_us < u64::MAX {
                        if t.resume_time.is_none() {
                            t.resume_time =
                                Some(Instant::now() + Duration::from_micros(timeout_us));
                            return Some(ApiHookResult::retry());
                        } else if Instant::now() < t.resume_time.unwrap() {
                            return Some(ApiHookResult::retry());
                        } else {
                            t.resume_time = None;
                        }
                    } else if timeout_us == u64::MAX {
                        // 무한 대기 시에는 단순히 yield하여 다른 스레드에 기회를 줌
                        return Some(ApiHookResult::retry());
                    } else {
                        t.resume_time = None;
                    }
                } else {
                    t.resume_time = None;
                }
            }
        }

        if total_ready > 0 {
            crate::emu_socket_log!("[SELECT] total_ready={}", total_ready);
        }
        Some(ApiHookResult::callee(5, Some(total_ready)))
    }

    // API: int send(SOCKET s, const char* buf, int len, int flags)
    // 역할: Ordinal_19 - 실제 TcpStream에 데이터 전송
    pub fn send(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sock = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let len = uc.read_arg(2) as usize;
        let flags = uc.read_arg(3);

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
                crate::emu_socket_log!("[SEND] sock={} FAIL: no socket", sock);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };

        let stream = match socket.stream.as_mut() {
            Some(s) => s,
            None => {
                ctx.last_error.store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR (not connected)", sock);
                crate::emu_socket_log!("[SEND] sock={} FAIL: not connected", sock);
                return Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)));
            }
        };

        let result = stream.try_write(&data);
        drop(sockets);

        match result {
            Ok(n) => {
                ctx.packet_logger
                    .lock()
                    .unwrap()
                    .log(PacketDirection::Send, sock, &data[..n]);
                crate::emu_log!(
                    "[WS2_32] send({}, {:#x}, {}, {}) -> {} bytes",
                    sock,
                    buf_addr,
                    len,
                    flags,
                    n
                );
                crate::emu_socket_log!("[SEND] sock={} -> {} bytes", sock, n);
                Some(ApiHookResult::callee(4, Some(n as i32)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
                if tid != 0 {
                    return Some(ApiHookResult::retry());
                }
                ctx.last_error.store(WSAEWOULDBLOCK, Ordering::SeqCst);
                Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
            }
            Err(e) => {
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR ({})", sock, e);
                crate::emu_socket_log!("[SEND] sock={} FAIL: {}", sock, e);
                Some(ApiHookResult::callee(4, Some(SOCKET_ERROR)))
            }
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
        crate::emu_socket_log!(
            "[SETSOCKOPT] sock={} level={} optname={} optval={:#x} optlen={}",
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
        crate::emu_socket_log!("[SHUTDOWN] sock={} how={}", sock, how);
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
            TokioSocket {
                af,
                sock_type,
                protocol,
                stream: None,
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

        let resolved_ip = block_on(async {
            tokio::net::lookup_host(format!("{}:80", name))
                .await
                .ok()
                .and_then(|mut iter| iter.next())
                .map(|addr| match addr.ip() {
                    std::net::IpAddr::V4(v4) => v4.octets(),
                    std::net::IpAddr::V6(_) => [127, 0, 0, 1],
                })
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
        uc.write_u32(hostent_addr, name_str as u32); // h_name
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
        crate::emu_socket_log!("[WSA_GET_LAST_ERROR] err={}", err);
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
        crate::emu_socket_log!(
            "[WSA_STARTUP] version={:#x} wsa_data_addr={:#x}",
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
        let _flags = uc.read_arg(4);
        let _overlapped = uc.read_arg(5);
        let _completion_routine = uc.read_arg(6);

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
            if let Some(s) = sockets.get_mut(&sock) {
                if let Some(stream) = s.stream.as_mut() {
                    if block_on(async { stream.write_all(&data).await }).is_ok() {
                        total_sent += buf_len;
                    }
                }
            }
            drop(sockets);
        }

        if bytes_sent_addr != 0 {
            uc.write_u32(bytes_sent_addr as u64, total_sent as u32);
        }
        crate::emu_log!("[WS2_32] WSASend({:#x}) -> {} bytes sent", sock, total_sent);
        crate::emu_socket_log!("[SEND] sock={} -> {} bytes (WSA)", sock, total_sent);
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
                crate::emu_socket_log!("[RECV] sock={} FAIL: no socket", sock);
                return Some(ApiHookResult::callee(7, Some(SOCKET_ERROR)));
            }
        };

        let mut data_to_distribute = Vec::new();

        // 1. 기존 recv_buf에서 먼저 소비
        if !socket.recv_buf.is_empty() {
            let take = total_requested.min(socket.recv_buf.len());
            data_to_distribute = socket.recv_buf.drain(..take).collect();
        }

        // 2. 버퍼가 부족하고 스트림이 있으면 추가로 읽기
        if data_to_distribute.len() < total_requested {
            if let Some(stream) = socket.stream.as_mut() {
                let want = total_requested - data_to_distribute.len();
                let mut tmp = vec![0u8; want];
                let res = if socket.non_blocking {
                    match stream.try_read(&mut tmp) {
                        Ok(n) => Ok(n),
                        Err(e) => Err(e),
                    }
                } else {
                    block_on(async { stream.read(&mut tmp).await })
                };

                match res {
                    Ok(n) if n > 0 => {
                        data_to_distribute.extend_from_slice(&tmp[..n]);
                    }
                    _ => {} // EOF or Error (WouldBlock 등)
                }
            }
        }

        let total_n = data_to_distribute.len();
        drop(sockets);

        if total_n == 0 {
            let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
            if tid != 0 {
                return Some(ApiHookResult::retry());
            }
            uc.get_data()
                .last_error
                .store(WSAEWOULDBLOCK, Ordering::SeqCst);
            crate::emu_log!(
                "[WS2_32] WSARecv({}) -> SOCKET_ERROR (WSAEWOULDBLOCK)",
                sock
            );
            crate::emu_socket_log!("[RECV] sock={} FAIL: WouldBlock (WSA)", sock);
            return Some(ApiHookResult::callee(7, Some(SOCKET_ERROR)));
        }

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
            .log(PacketDirection::Recv, sock, &data_to_distribute);

        if bytes_recvd_addr != 0 {
            uc.write_u32(bytes_recvd_addr as u64, total_n as u32);
        }

        crate::emu_socket_log!("[RECV] sock={} -> {} bytes (WSA Multi)", sock, total_n);

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
        let _flags = uc.read_arg(5);
        let ctx = uc.get_data();
        let sock = ctx.alloc_handle();
        ctx.tcp_sockets.lock().unwrap().insert(
            sock,
            TokioSocket {
                af,
                sock_type,
                protocol,
                stream: None,
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
        crate::emu_socket_log!("[WSA_CREATE_EVENT] handle={:#x}", handle);
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
            .map(|s| s.stream.is_some())
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
        crate::emu_socket_log!(
            "[WSA_EVENT_SELECT] sock={} event={} network_events={} initial_pending={}",
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
            let sockets = tcp_sockets.lock().unwrap();
            if let Some(s) = sockets.get(&sock) {
                // FD_CONNECT(0x10): 스트림이 연결되어 있으면
                if interest & 0x10 != 0 && s.stream.is_some() {
                    pending |= 0x10;
                }
                // FD_READ(0x01): recv_buf에 데이터가 있으면
                if interest & 0x01 != 0 && !s.recv_buf.is_empty() {
                    pending |= 0x01;
                }
                // FD_WRITE(0x02): 연결된 소켓은 항상 쓰기 가능
                if interest & 0x02 != 0 && s.stream.is_some() {
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
        crate::emu_socket_log!(
            "[WSA_ENUM_NETWORK_EVENTS] sock={} event={} network_events={:#x}",
            sock,
            event,
            l_network_events
        );
        Some(ApiHookResult::callee(3, Some(0)))
    }

    /// 함수명 기준 `WS2_32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
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
