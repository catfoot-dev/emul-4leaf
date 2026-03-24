pub mod packet_logger;

use std::io::{self, Write};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[tokio::main]
pub async fn server() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:33000";
    let listener = TcpListener::bind(addr).await?;
    crate::emu_log!("[*] Server running on {}", addr);
    crate::emu_log!("[*] Enter HEX strings (e.g., '48656c6c6f') to send to clients.");

    // 1. 브로드캐스트 채널 생성
    // 콘솔 입력(Stdin)을 연결된 모든 클라이언트에게 전달하기 위한 채널
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(10);

    // 2. 별도 태스크: 서버 콘솔 입력 처리 (Stdin -> Channel)
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let stdin = io::stdin();
        let mut input = String::new();

        loop {
            input.clear();
            print!("Server> ");
            io::stdout().flush().unwrap();

            // 입력을 기다림 (블로킹이지만 별도 스레드/태스크처럼 동작하므로 네트워크에 영향 없음)
            if stdin.read_line(&mut input).is_ok() {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Hex 문자열을 바이너리로 변환
                match hex::decode(trimmed) {
                    Ok(bytes) => {
                        // 연결된 모든 클라이언트에게 브로드캐스트
                        if let Err(e) = tx_clone.send(bytes) {
                            eprintln!("Failed to broadcast: no active clients ({})", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error decoding hex: {}", e);
                    }
                }
            }
        }
    });

    // 3. 메인 루프: 클라이언트 연결 수락
    loop {
        let (socket, client_addr) = listener.accept().await?;
        crate::emu_log!("[*] New Client Connected: {}", client_addr);

        // 채널 구독 (Stdin에서 오는 데이터를 받기 위함)
        let mut rx = tx.subscribe();

        // 소켓을 Read 부분과 Write 부분으로 분리
        let (reader, mut writer) = socket.into_split();

        // 3-1. 수신 태스크 (Client -> Server Screen)
        let mut buf_reader = BufReader::new(reader);
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match buf_reader.read(&mut buf).await {
                    Ok(0) => {
                        crate::emu_log!("[*] Client Disconnected: {}", client_addr);
                        break;
                    }
                    Ok(n) => {
                        // 수신된 데이터 표시 (Hex + UTF-8 시도)
                        let data = &buf[0..n];
                        let hex_dump = hex::encode(data);
                        let text = String::from_utf8_lossy(data);
                        crate::emu_log!("[From {}]: {} (Hex: {})", client_addr, text, hex_dump);
                        print!("Server> "); // 프롬프트 복구
                        io::stdout().flush().unwrap();
                    }
                    Err(e) => {
                        eprintln!("Error reading from socket: {}", e);
                        break;
                    }
                }
            }
        });

        // 3-2. 송신 태스크 (Server Stdin Channel -> Client Socket)
        tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                // 채널에서 데이터가 오면 소켓으로 전송
                if let Err(e) = writer.write_all(&msg).await {
                    eprintln!("Failed to write to client: {}", e);
                    break;
                }
            }
        });
    }
}
