use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::server::packet_logger::PacketDirection;
use crate::win32::{ApiHookResult, TokioSocket, Win32Context, callee_result};

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

/// 소켓 I/O 전용 Tokio 런타임을 반환합니다.
/// 에뮬레이터 루프가 동기 스레드에서 작동하므로, 비동기 작업을 제어하기 위해 별도의 런타임을 유지합니다.
fn get_runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime for WS2_32")
    })
}

/// 비동기 퓨처(Future)를 현재 스레드에서 동기적으로 실행합니다.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    get_runtime().block_on(f)
}

pub struct DllWS2_32 {}

impl DllWS2_32 {
    /// **Ordinal 1: accept**
    ///
    /// 들어오는 연결 요청을 수락합니다.
    /// 현재 리스닝 소켓은 미구현 상태이므로 항상 `INVALID_SOCKET`을 반환합니다.
    pub fn accept(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen_ptr = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] accept({}, {:#x}, {:#x}) -> SOCKET -1 (not implemented)",
            sock,
            addr_ptr,
            addrlen_ptr
        );
        Some((3, Some(-1i32))) // INVALID_SOCKET
    }

    /// **Ordinal 2: bind**
    ///
    /// 로컬 주소를 소켓에 연결합니다. 에뮬레이션 환경에서는 항상 성공으로 처리합니다.
    pub fn bind(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let addr_ptr = uc.read_arg(1);
        let addrlen = uc.read_arg(2);
        crate::emu_log!(
            "[WS2_32] bind({}, {:#x}, {}) -> int 0",
            sock,
            addr_ptr,
            addrlen
        );
        crate::push_socket_log(format!("[BIND] sock={} addr_ptr={:#x}", sock, addr_ptr));
        Some((3, Some(0)))
    }

    /// **Ordinal 3: closesocket**
    ///
    /// 소켓을 닫고 관련 리소스를 해제합니다.
    pub fn closesocket(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let ctx = uc.get_data();
        ctx.tcp_sockets.lock().unwrap().remove(&sock);
        crate::emu_log!("[WS2_32] closesocket({}) -> int 0", sock);
        crate::push_socket_log(format!("[CLOSE] sock={}", sock));
        Some((1, Some(0)))
    }

    /// **Ordinal 4: connect**
    ///
    /// 실제 호스트의 `tokio::net::TcpStream::connect`를 호출하여 원격 주소와 연결을 수립합니다.
    pub fn connect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                crate::emu_log!("[WS2_32] connect({}, \"{}\") -> OK", sock, addr_str);
                crate::push_socket_log(format!("[CONN] sock={} -> {} OK", sock, addr_str));
                Some((3, Some(0)))
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
                crate::push_socket_log(format!("[CONN] sock={} -> {} FAIL: {}", sock, addr_str, e));
                Some((3, Some(SOCKET_ERROR)))
            }
            Err(_) => {
                ctx.last_error.store(WSAETIMEDOUT, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] connect({}, \"{}\") -> TIMEOUT", sock, addr_str);
                Some((3, Some(SOCKET_ERROR)))
            }
        }
    }

    // API: int getpeername(SOCKET s, struct sockaddr* name, int* namelen)
    // 역할: Ordinal_5 - 연결된 상대방의 주소 정보를 가져옴
    pub fn getpeername(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                    return Some((3, Some(0)));
                }
            }
        }
        crate::emu_log!(
            "[WS2_32] getpeername({}) -> SOCKET_ERROR (not connected)",
            sock
        );
        Some((3, Some(SOCKET_ERROR)))
    }

    // API: int getsockopt(SOCKET s, int level, int optname, char* optval, int* optlen)
    // 역할: Ordinal_7 - 소켓 옵션 값을 가져옴
    pub fn getsockopt(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((5, Some(0)))
    }

    // API: u_long htonl(u_long hostlong)
    // 역할: Ordinal_8 - 32비트 호스트 바이트 순서를 네트워크 바이트 순서(Big-Endian)로 변환
    pub fn htonl(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let val = uc.read_arg(0);
        let result = val.swap_bytes();
        crate::emu_log!("[WS2_32] htonl({:#x}) -> u_long {:#x}", val, result);
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
    // 역할: Ordinal_10 - FIONBIO로 논블로킹 모드 설정
    pub fn ioctlsocket(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                crate::push_socket_log(format!(
                    "[IOCTL] sock={} FIONBIO non_blocking={}",
                    sock, non_blocking
                ));
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
        }
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
        crate::emu_log!("[WS2_32] inet_addr(\"{}\") -> {:#x}", addr_str, result);
        Some((1, Some(result as i32)))
    }

    // API: char* inet_ntoa(struct in_addr in)
    // 역할: Ordinal_12 - 네트워크 바이트 순서의 IP 주소를 문자열로 변환
    pub fn inet_ntoa(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(ptr as i32)))
    }

    // API: int listen(SOCKET s, int backlog)
    // 역할: Ordinal_13 - 소켓을 수신 모드로 설정
    pub fn listen(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let backlog = uc.read_arg(1);
        crate::emu_log!("[WS2_32] listen({}, {}) -> int 0 (stub)", sock, backlog);
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
    // 역할: Ordinal_16 - 실제 TcpStream에서 데이터를 수신
    pub fn recv(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                return Some((4, Some(SOCKET_ERROR)));
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
            return Some((4, Some(take as i32)));
        }

        let non_blocking = socket.non_blocking;
        let stream = match socket.stream.as_mut() {
            Some(s) => s,
            None => {
                drop(sockets);
                uc.get_data()
                    .last_error
                    .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                return Some((4, Some(SOCKET_ERROR)));
            }
        };

        if non_blocking {
            // 논블로킹: try_read 시도
            let mut tmp = vec![0u8; len];
            match stream.try_read(&mut tmp) {
                Ok(0) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> 0 (connection closed)", sock);
                    Some((4, Some(0)))
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
                    Some((4, Some(n as i32)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    drop(sockets);
                    uc.get_data()
                        .last_error
                        .store(WSAEWOULDBLOCK, Ordering::SeqCst);
                    crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR (WSAEWOULDBLOCK)", sock);
                    Some((4, Some(SOCKET_ERROR)))
                }
                Err(e) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR ({})", sock, e);
                    Some((4, Some(SOCKET_ERROR)))
                }
            }
        } else {
            // 블로킹: async read
            let mut tmp = vec![0u8; len];
            let read_result = block_on(async { stream.read(&mut tmp).await });
            match read_result {
                Ok(0) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> 0 (connection closed)", sock);
                    Some((4, Some(0)))
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
                        "[WS2_32] recv({}, {:#x}, {}, {}) -> {} bytes (blocking)",
                        sock,
                        buf_addr,
                        len,
                        flags,
                        n
                    );
                    Some((4, Some(n as i32)))
                }
                Err(e) => {
                    drop(sockets);
                    crate::emu_log!("[WS2_32] recv({}) -> SOCKET_ERROR ({})", sock, e);
                    Some((4, Some(SOCKET_ERROR)))
                }
            }
        }
    }

    // API: int select(int nfds, fd_set* readfds, fd_set* writefds, fd_set* exceptfds, const struct timeval* timeout)
    // 역할: Ordinal_18 - 소켓 읽기/쓰기 가능 여부를 확인
    pub fn select(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let nfds = uc.read_arg(0);
        let readfds = uc.read_arg(1);
        let writefds = uc.read_arg(2);
        let exceptfds = uc.read_arg(3);
        let timeout_ptr = uc.read_arg(4);

        // timeval 구조체 파싱: tv_sec(4) + tv_usec(4)
        let timeout_us = if timeout_ptr != 0 {
            let sec = uc.read_u32(timeout_ptr as u64) as u64;
            let usec = uc.read_u32(timeout_ptr as u64 + 4) as u64;
            sec * 1_000_000 + usec
        } else {
            500_000 // 기본 500ms 타임아웃
        };

        // fd_set: count(4) + fd[64](4 each) 구조체
        // readfds 에서 소켓 핸들 목록 추출
        let mut readable_count = 0i32;
        let mut writable_count = 0i32;

        if readfds != 0 {
            let count = uc.read_u32(readfds as u64) as usize;
            let count = count.min(64);
            let ctx = uc.get_data();
            for i in 0..count {
                let sock = uc.read_u32(readfds as u64 + 4 + (i * 4) as u64);
                let sockets = ctx.tcp_sockets.lock().unwrap();
                if let Some(s) = sockets.get(&sock) {
                    if !s.recv_buf.is_empty() {
                        readable_count += 1;
                        continue;
                    }
                    if let Some(stream) = &s.stream {
                        // 소켓이 읽기 가능한지 짧은 타임아웃으로 확인
                        let readable = block_on(async {
                            tokio::time::timeout(
                                std::time::Duration::from_micros(timeout_us / count.max(1) as u64),
                                stream.readable(),
                            )
                            .await
                        });
                        if readable.is_ok() {
                            readable_count += 1;
                        }
                    }
                }
            }
        }

        if writefds != 0 {
            let count = uc.read_u32(writefds as u64) as usize;
            let count = count.min(64);
            let ctx = uc.get_data();
            for i in 0..count {
                let sock = uc.read_u32(writefds as u64 + 4 + (i * 4) as u64);
                let sockets = ctx.tcp_sockets.lock().unwrap();
                if let Some(s) = sockets.get(&sock) {
                    // 연결된 소켓은 기본적으로 쓰기 가능
                    if s.stream.is_some() {
                        writable_count += 1;
                    }
                }
            }
        }

        let total = readable_count + writable_count;
        crate::emu_log!(
            "[WS2_32] select({}, {:#x}, {:#x}, {:#x}, {:#x}) -> {} (r={}, w={})",
            nfds,
            readfds,
            writefds,
            exceptfds,
            timeout_ptr,
            total,
            readable_count,
            writable_count
        );
        Some((5, Some(total)))
    }

    // API: int send(SOCKET s, const char* buf, int len, int flags)
    // 역할: Ordinal_19 - 실제 TcpStream에 데이터 전송
    pub fn send(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let len = uc.read_arg(2) as usize;
        let flags = uc.read_arg(3);

        if len == 0 {
            return Some((4, Some(0)));
        }

        let data = uc.mem_read_as_vec(buf_addr as u64, len).unwrap_or_default();
        let ctx = uc.get_data();

        let mut sockets = ctx.tcp_sockets.lock().unwrap();
        let socket = match sockets.get_mut(&sock) {
            Some(s) => s,
            None => {
                ctx.last_error.store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR (no socket)", sock);
                return Some((4, Some(SOCKET_ERROR)));
            }
        };

        let stream = match socket.stream.as_mut() {
            Some(s) => s,
            None => {
                ctx.last_error.store(WSAEWOULDBLOCK, Ordering::SeqCst);
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR (not connected)", sock);
                return Some((4, Some(SOCKET_ERROR)));
            }
        };

        let result = block_on(async { stream.write_all(&data).await });
        drop(sockets);

        match result {
            Ok(()) => {
                ctx.packet_logger
                    .lock()
                    .unwrap()
                    .log(PacketDirection::Send, sock, &data);
                crate::emu_log!(
                    "[WS2_32] send({}, {:#x}, {}, {}) -> {} bytes",
                    sock,
                    buf_addr,
                    len,
                    flags,
                    len
                );
                Some((4, Some(len as i32)))
            }
            Err(e) => {
                crate::emu_log!("[WS2_32] send({}) -> SOCKET_ERROR ({})", sock, e);
                Some((4, Some(SOCKET_ERROR)))
            }
        }
    }

    // API: int setsockopt(SOCKET s, int level, int optname, const char* optval, int optlen)
    // 역할: Ordinal_21 - 소켓 옵션 설정 (주요 옵션만 처리)
    pub fn setsockopt(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((5, Some(0)))
    }

    // API: int shutdown(SOCKET s, int how)
    // 역할: Ordinal_22 - 소켓의 송수신 기능을 중단
    pub fn shutdown(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let how = uc.read_arg(1);
        // 실제로 TcpStream을 끊지는 않고 closesocket에서 처리
        crate::emu_log!("[WS2_32] shutdown({}, how={}) -> 0", sock, how);
        Some((2, Some(0)))
    }

    // API: SOCKET socket(int af, int type, int protocol)
    // 역할: Ordinal_23 - 새 TokioSocket 생성
    pub fn socket(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        crate::push_socket_log(format!(
            "[SOCK] created sock={} af={} type={} proto={}",
            sock, af, sock_type, protocol
        ));
        Some((3, Some(sock as i32)))
    }

    // API: struct hostent* gethostbyname(const char* name)
    // 역할: Ordinal_52 - 실제 DNS 조회로 호스트 이름 해석
    pub fn gethostbyname(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        crate::push_socket_log(format!(
            "[DNS] \"{}\" -> {}.{}.{}.{}",
            name, resolved_ip[0], resolved_ip[1], resolved_ip[2], resolved_ip[3]
        ));

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

        Some((1, Some(hostent_addr as i32)))
    }

    // API: int WSAGetLastError(void)
    // 역할: Ordinal_111 - 마지막으로 발생한 네트워크 오류 코드를 반환
    pub fn wsa_get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let err = uc.get_data().last_error.load(Ordering::SeqCst);
        crate::emu_log!("[WS2_32] WSAGetLastError() -> {}", err);
        Some((0, Some(err as i32)))
    }

    // API: int WSAStartup(WORD wVersionRequested, LPWSADATA lpWSAData)
    // 역할: Ordinal_115 - Winsock 라이브러리를 초기화
    pub fn wsa_startup(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(0)))
    }

    // API: int WSACleanup(void)
    // 역할: Ordinal_116 - Winsock 라이브러리 사용을 종료
    pub fn wsa_cleanup(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[WS2_32] WSACleanup() -> 0");
        Some((0, Some(0)))
    }

    // API: int __WSAFDIsSet(SOCKET fd, fd_set* set)
    pub fn wsa_fd_is_set(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let set = uc.read_arg(1);
        crate::emu_log!("[WS2_32] __WSAFDIsSet({:#x}, {:#x}) -> 0", sock, set);
        Some((2, Some(0)))
    }

    // API: int WSASend(SOCKET s, LPWSABUF lpBuffers, DWORD dwBufferCount, LPDWORD lpNumberOfBytesSent, DWORD dwFlags, LPWSAOVERLAPPED lpOverlapped, LPWSAOVERLAPPED_COMPLETION_ROUTINE lpCompletionRoutine)
    // 역할: WSABuf 배열에서 데이터를 읽어 실제로 전송
    pub fn wsa_send(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((7, Some(if total_sent > 0 { 0 } else { SOCKET_ERROR })))
    }

    // API: SOCKET WSASocketA(int af, int type, int protocol, LPWSAPROTOCOL_INFOA lpProtocolInfo, GROUP g, DWORD dwFlags)
    // 역할: 새 소켓을 생성 (확장 기능 포함)
    pub fn wsa_socket_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        crate::push_socket_log(format!(
            "[SOCK] created(WSA) sock={} af={} type={} proto={}",
            sock, af, sock_type, protocol
        ));
        Some((6, Some(sock as i32)))
    }

    // API: WSAEVENT WSACreateEvent(void)
    // 역할: 새 이벤트 개체를 생성
    pub fn wsa_create_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.get_data().alloc_handle();
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
        crate::emu_log!("[WS2_32] WSACloseEvent({:#x}) -> TRUE", event);
        Some((1, Some(1)))
    }

    // API: int WSAEnumNetworkEvents(SOCKET s, WSAEVENT hEventObject, LPWSANETWORKEVENTS lpNetworkEvents)
    // 역할: 특정 소켓에서 발생한 네트워크 이벤트를 확인
    pub fn wsa_enum_network_events(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let sock = uc.read_arg(0);
        let event = uc.read_arg(1);
        let net_events_addr = uc.read_arg(2);
        if net_events_addr != 0 {
            let zeros = [0u8; 44];
            uc.mem_write(net_events_addr as u64, &zeros).unwrap();
        }
        crate::emu_log!(
            "[WS2_32] WSAEnumNetworkEvents({:#x}, {:#x}, {:#x}) -> 0",
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
