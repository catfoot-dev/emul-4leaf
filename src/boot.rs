//! # 리소스 디렉토리 탐색 및 부트스트래핑
//!
//! 에뮬레이터 실행에 필요한 리소스 디렉토리(DLL 파일들)를 탐색하고 확정합니다.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 실행 시 결정된 리소스 디렉토리 경로를 전역 캐시합니다.
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 리소스 디렉토리로 인정하기 위해 반드시 존재해야 하는 파일 목록입니다.
const REQUIRED_RESOURCE_FILES: &[&str] = &[
    "4Leaf.exe",
    "4Leaf.dll",
    "Core.dll",
    "WinCore.dll",
    "DNet.dll",
    "Lime.dll",
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
        if let Ok(exe) = env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                v.push(exe_dir.to_path_buf());
                v.push(exe_dir.join("Resources"));
                v.push(exe_dir.join("../4Leaf"));
            }
        }

        // 현재 작업 디렉토리 기준 후보
        if let Ok(cwd) = env::current_dir() {
            v.push(cwd.clone());
            v.push(cwd.join("Resources"));
            v.push(cwd.join("../4Leaf"));
        } else {
            v.push(PathBuf::from("./Resources"));
            v.push(PathBuf::from("../4Leaf"));
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
            eprintln!("[BOOT] Resource directory: {}", resolved.display());
            let _ = RESOURCE_DIR.set(resolved);
            return;
        }
    }

    // fallback — 필수 파일이 없더라도 기존 동작을 유지
    let fallback = PathBuf::from("./Resources");
    let missing: Vec<&str> = REQUIRED_RESOURCE_FILES
        .iter()
        .filter(|f| !fallback.join(f).is_file())
        .copied()
        .collect();
    if missing.is_empty() {
        eprintln!("[BOOT] Resource directory (fallback): ./Resources");
    } else {
        eprintln!(
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
        .unwrap_or(Path::new("./Resources"))
}
