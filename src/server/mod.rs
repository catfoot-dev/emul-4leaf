//! 4Leaf 서버 에뮬레이션 계층의 진입점입니다.
//!
//! 이 모듈은 읽는 사람이 흐름을 빠르게 파악할 수 있도록 책임을 네 층으로 나눕니다.
//! - `protocol`: 와이어 포맷과 패킷 직렬화/파싱
//! - `handler`: 수신 버퍼, 채널 상태, 도메인 디스패치
//! - `session`: 인메모리 세션/회원 상태
//! - `domain`: Auth, System, World, Chat 등 기능별 처리기

pub mod packet_logger;
pub mod protocol;

mod domain;
mod handler;
mod session;

pub use handler::run_dnet_handler;

#[cfg(test)]
mod tests;
