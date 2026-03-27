pub mod packet_logger;
pub mod protocol;

use self::protocol::{ControlMessage, DNetPacket, ProtocolPacket};
use std::sync::OnceLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// 서버의 브로드캐스트 송신단
pub static SERVER_TX: OnceLock<broadcast::Sender<Vec<u8>>> = OnceLock::new();

#[tokio::main]
pub async fn server() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:33000";
    let listener = TcpListener::bind(addr).await?;
    crate::emu_socket_log!("[*] Protocol-aware Server running on {}", addr);

    let (tx, _rx) = broadcast::channel::<Vec<u8>>(10);
    SERVER_TX.set(tx.clone()).ok();

    loop {
        let (socket, client_addr) = listener.accept().await?;
        crate::emu_socket_log!("[*] New Client Connected: {}", client_addr);

        let mut rx = tx.subscribe();
        let (reader, mut writer) = socket.into_split();
        let (direct_tx, mut direct_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        // 서버는 초기 패킷을 먼저 보내지 않습니다.
        // 클라이언트가 CTRL_OPEN(1) 제어 메시지로 채널 개방을 먼저 요청합니다.

        // 1. 수신 및 프로토콜 처리 태스크
        // DNet 전송 계층
        //
        // 헤더: [channel_id: u16 LE][body_len: u16 LE]
        // ch=0: 제어 메시지 4B → [msg_type: u16 LE][target_channel_id: u16 LE]
        // ch=1-15: 데이터 패킷 → [main_type: u8][sub_type: u8][payload...]
        let mut buf_reader = BufReader::new(reader);
        tokio::spawn(async move {
            loop {
                // 4바이트 DNet 헤더 읽기
                let mut header = [0u8; 4];
                match buf_reader.read_exact(&mut header).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        crate::emu_socket_log!("[*] Client Disconnected: {}", client_addr);
                        break;
                    }
                    Err(e) => {
                        crate::emu_socket_log!("[!] Header read error: {}", e);
                        break;
                    }
                }

                // 유효성 검사 (channel_id 0-15, body_len 0-0x1FFC, ch0→len==4)
                let (channel_id, body_len) = match DNetPacket::parse_header(&header) {
                    Some(v) => v,
                    None => {
                        crate::emu_socket_log!(
                            "[!] Invalid DNet header from {}: {}",
                            client_addr,
                            hex::encode(&header)
                        );
                        break;
                    }
                };

                // body_len 바이트 본문 읽기
                let mut body = vec![0u8; body_len as usize];
                match buf_reader.read_exact(&mut body).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        crate::emu_socket_log!(
                            "[*] Client Disconnected (mid-packet): {}",
                            client_addr
                        );
                        break;
                    }
                    Err(e) => {
                        crate::emu_socket_log!("[!] Body read error: {}", e);
                        break;
                    }
                }

                // 채널별 라우팅
                if channel_id == 0 {
                    // 채널 0: 제어 메시지 [msg_type: u16 LE][대상 채널: u16 LE]
                    // SendControlMessage(this, msg, ch): HIWORD(a2)=채널번호, LOWORD(a2)=메시지타입
                    let ctrl = match ControlMessage::from_bytes(&body) {
                        Some(c) => c,
                        None => {
                            crate::emu_socket_log!(
                                "[!] Malformed control body: {}",
                                hex::encode(&body)
                            );
                            break;
                        }
                    };

                    crate::emu_socket_log!(
                        "[CTRL] msg={} target_ch={}",
                        ctrl.msg_type,
                        ctrl.channel_id
                    );

                    // 채널 상태 기계 (서버 측, TConnection::ProcessControlMessage 역공학)
                    let resp = match ctrl.msg_type {
                        protocol::CTRL_OPEN => {
                            // 클라이언트가 채널 N 개방 요청
                            // 서버는 LISTENING(1) 상태 → OPEN_ACK(2) 응답 + CONNECTED로 전환
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} open request → accepting",
                                ctrl.channel_id
                            );
                            Some(protocol::create_control_message(
                                protocol::CTRL_OPEN_ACK,
                                ctrl.channel_id,
                            ))
                        }
                        protocol::CTRL_OPEN_ACK => {
                            // 상대방이 채널 개방 확인 (서버 측 CONNECTING(0) 시나리오)
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} open acknowledged",
                                ctrl.channel_id
                            );
                            None
                        }
                        protocol::CTRL_CLOSE => {
                            // 클라이언트가 채널 N 종료 요청
                            // CONNECTED(2) 상태: OnClosed 처리 후 CLOSE_ACK(4) 전송
                            crate::emu_socket_log!(
                                "[CTRL] Ch={} close request → sending ack",
                                ctrl.channel_id
                            );
                            Some(protocol::create_control_message(
                                protocol::CTRL_CLOSE_ACK,
                                ctrl.channel_id,
                            ))
                        }
                        protocol::CTRL_CLOSE_ACK => {
                            // 종료 확인 수신 (CLOSING(3) → 완전 종료)
                            crate::emu_socket_log!("[CTRL] Ch={} fully closed", ctrl.channel_id);
                            None
                        }
                        unknown => {
                            crate::emu_socket_log!(
                                "[CTRL] Unknown msg_type={} ch={}",
                                unknown,
                                ctrl.channel_id
                            );
                            None
                        }
                    };

                    if let Some(pkt) = resp {
                        crate::emu_socket_log!("[SEND] {}", protocol::hex_dump("CTRL", &pkt));
                        if direct_tx.send(pkt).is_err() {
                            crate::emu_socket_log!("[!] Failed to queue control response");
                        }
                    }
                } else {
                    // 채널 1-15: 데이터 패킷 [main_type: u8][sub_type: u8][payload...]
                    // ProcessPacket: 채널이 CONNECTED(2) 상태여야 하며, 아닐 경우 CTRL_CLOSE(3) 전송
                    if let Some(pkt) = ProtocolPacket::from_bytes(&body) {
                        crate::emu_socket_log!(
                            "[RECV] ch={} main=0x{:02x} sub=0x{:02x} payload={}B",
                            channel_id,
                            pkt.main_type,
                            pkt.sub_type,
                            pkt.payload.len()
                        );
                        if pkt.main_type == 0x0b {
                            crate::emu_socket_log!("[*] ChatTown (sub=0x{:02x})", pkt.sub_type);
                        }
                    } else {
                        // 채널이 없거나 연결 안 됨 → CTRL_CLOSE(3) 전송 (ProcessPacket 동작)
                        crate::emu_socket_log!(
                            "[!] No handler for ch={} from {} → sending CLOSE",
                            channel_id,
                            client_addr
                        );
                        let close =
                            protocol::create_control_message(protocol::CTRL_CLOSE, channel_id);
                        direct_tx.send(close).ok();
                    }
                }
            }
        });

        // 2. 송신 태스크 (직접 응답 + 브로드캐스트)
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    Some(msg) = direct_rx.recv() => {
                        crate::emu_socket_log!("[SEND] {}", protocol::hex_dump("Pkt", &msg));
                        if let Err(e) = writer.write_all(&msg).await {
                            crate::emu_socket_log!("[!] Failed to write direct response: {}", e);
                            break;
                        }
                    }
                    res = rx.recv() => {
                        match res {
                            Ok(msg) => {
                                crate::emu_socket_log!("[SEND] {}", protocol::hex_dump("Bcast", &msg));
                                if let Err(e) = writer.write_all(&msg).await {
                                    crate::emu_socket_log!("[!] Failed to write broadcast message: {}", e);
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });
    }
}
