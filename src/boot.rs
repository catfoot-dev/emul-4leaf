//! # 리소스 디렉토리 탐색 및 부트스트래핑
//!
//! 에뮬레이터 실행에 필요한 리소스 디렉토리(DLL 파일들)를 탐색하고 확정합니다.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::emu_log;

/// 실행 시 결정된 리소스 디렉토리 경로를 전역 캐시합니다.
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub const EXECUTABLE_NAME: &str = "4Leaf.exe";
pub const LIBLARY_4LEAF: &str = "4Leaf.dll";
pub const LIBLARY_CORE: &str = "Core.dll";
pub const LIBLARY_WINCORE: &str = "WinCore.dll";
pub const LIBLARY_DNET: &str = "DNet.dll";
pub const LIBLARY_LIME: &str = "Lime.dll";
pub const LIBLARY_DICE: &str = "Dice.dll";

const RESOURCES_NAME: &str = "Resources";
const RESOURCES_DIR: &str = "./Resources";
const APPLICATION_DIR: &str = "../4Leaf";

/// 리소스 디렉토리로 인정하기 위해 반드시 존재해야 하는 파일 목록입니다.
const REQUIRED_RESOURCE_FILES: &[&str] = &[
    EXECUTABLE_NAME,
    LIBLARY_4LEAF,
    LIBLARY_CORE,
    LIBLARY_WINCORE,
    LIBLARY_DNET,
    LIBLARY_LIME,
    LIBLARY_DICE,
];

/// 리소스 디렉토리를 동적으로 탐색하여 확정합니다.
///
/// 후보 디렉토리에 필수 파일이 모두 존재하는지 검증합니다.
///
/// 탐색 순서:
/// - 실행 파일이 위치한 디렉토리
/// - 실행 파일 기준 디렉토리
/// - 현재 작업 디렉토리
/// - ./Resources, ../4Leaf
pub fn detect_resource_dir() {
    let candidates: Vec<PathBuf> = {
        let mut v = Vec::new();

        // 실행 파일 위치 기준 후보
        if let Ok(exe) = env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            v.push(exe_dir.to_path_buf());
            v.push(exe_dir.join(RESOURCES_NAME));
            v.push(exe_dir.join(APPLICATION_DIR));
        }

        // 현재 작업 디렉토리 기준 후보
        if let Ok(cwd) = env::current_dir() {
            v.push(cwd.clone());
            v.push(cwd.join(RESOURCES_NAME));
            v.push(cwd.join(APPLICATION_DIR));
        } else {
            v.push(PathBuf::from(RESOURCES_DIR));
            v.push(PathBuf::from(APPLICATION_DIR));
        }

        v
    };

    for candidate in &candidates {
        if !candidate.is_dir() {
            continue;
        }
        let all_present = REQUIRED_RESOURCE_FILES
            .iter()
            .all(|f| candidate.join(f).is_file());
        if all_present {
            let resolved = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            emu_log!("[BOOT] Resource directory: {}", resolved.display());
            let _ = RESOURCE_DIR.set(resolved);
            return;
        }
    }

    // fallback — 필수 파일이 없더라도 기존 동작을 유지
    let fallback = PathBuf::from(RESOURCES_DIR);
    let missing: Vec<&str> = REQUIRED_RESOURCE_FILES
        .iter()
        .filter(|f| !fallback.join(f).is_file())
        .copied()
        .collect();
    if missing.is_empty() {
        emu_log!("[BOOT] Resource directory (fallback): ./Resources");
    } else {
        emu_log!(
            "[BOOT] Resource directory not found! Missing files: {}",
            missing.join(", ")
        );
    }
    let _ = RESOURCE_DIR.set(fallback);
}

/// 확정된 리소스 디렉토리 경로를 반환합니다.
pub fn resource_dir() -> &'static Path {
    RESOURCE_DIR
        .get()
        .map(|p| p.as_path())
        .unwrap_or(Path::new(RESOURCES_DIR))
}
