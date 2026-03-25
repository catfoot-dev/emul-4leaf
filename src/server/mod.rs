pub mod packet_logger;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use std::sync::OnceLock;

/// 서버의 브로드캐스트 송신단 (UI 등 다른 모듈에서 접근 가능하도록 전역 설정)
pub static SERVER_TX: OnceLock<broadcast::Sender<Vec<u8>>> = OnceLock::new();

#[tokio::main]
pub async fn server() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:33000";
    let listener = TcpListener::bind(addr).await?;
    crate::push_socket_log(format!("[*] Server running on {}", addr));
    crate::push_socket_log(format!("[*] Type HEX in Debug Window to send to clients."));

    // 1. 브로드캐스트 채널 생성
    // UI 입력 등을 연결된 모든 클라이언트에게 전달하기 위한 채널
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(10);
    SERVER_TX.set(tx.clone()).ok();

    // 2. 메인 루프: 클라이언트 연결 수락
    loop {
        let (socket, client_addr) = listener.accept().await?;
        crate::push_socket_log(format!("[*] New Client Connected: {}", client_addr));

        // 채널 구독 (Stdin에서 오는 데이터를 받기 위함)
        let mut rx = tx.subscribe();

        // 소켓을 Read 부분과 Write 부분으로 분리
        let (reader, mut writer) = socket.into_split();

        // 2-1. 수신 태스크 (Client -> Server Screen)
        let mut buf_reader = BufReader::new(reader);
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match buf_reader.read(&mut buf).await {
                    Ok(0) => {
                        crate::push_socket_log(format!("[*] Client Disconnected: {}", client_addr));
                        break;
                    }
                    Ok(n) => {
                        // 수신된 데이터 표시 (Hex + UTF-8 시도)
                        let data = &buf[0..n];
                        let hex_dump = hex::encode(data);
                        let text = String::from_utf8_lossy(data);
                        crate::push_socket_log(format!(
                            "[From {}]: {} (Hex: {})",
                            client_addr, text, hex_dump
                        ));
                    }
                    Err(e) => {
                        crate::push_socket_log(format!("[!] Error reading from socket: {}", e));
                        break;
                    }
                }
            }
        });

        // 2-2. 송신 태스크 (Server Stdin Channel -> Client Socket)
        tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                // 채널에서 데이터가 오면 소켓으로 전송
                if let Err(e) = writer.write_all(&msg).await {
                    crate::push_socket_log(format!("[!] Failed to write to client: {}", e));
                    break;
                }
            }
        });
    }
}
