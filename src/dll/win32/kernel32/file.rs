use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

// =========================================================
// File System
// =========================================================
// API: HANDLE CreateFileA(LPCSTR lpFileName, ...)
// 역할: 파일 또는 입출력 디바이스 개체를 생성하거나 오픈
pub(super) fn create_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    let access = uc.read_arg(1);
    let share_mode = uc.read_arg(2);
    let security_attributes = uc.read_arg(3);
    let creation_disposition = uc.read_arg(4);
    let template_file = uc.read_arg(5);
    let ctx = uc.get_data();
    let handle = ctx.alloc_handle();
    crate::emu_log!(
        "[KERNEL32] CreateFileA(\"{}\", {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HANDLE {:#x}",
        name,
        access,
        share_mode,
        security_attributes,
        creation_disposition,
        template_file,
        handle
    );
    Some(ApiHookResult::callee(7, Some(handle as i32)))
}

// API: HANDLE FindFirstFileA(LPCSTR lpFileName, LPWIN32_FIND_DATAA lpFindFileData)
// 역할: 지정된 이름과 일치하는 파일용 핸들을 검색/생성
pub(super) fn find_first_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    let find_file_data_addr = uc.read_arg(1);
    crate::emu_log!(
        "[KERNEL32] FindFirstFileA(\"{}\", {:#x}) -> INVALID_HANDLE_VALUE",
        name,
        find_file_data_addr
    );
    Some(ApiHookResult::callee(2, Some(-1i32))) // INVALID_HANDLE_VALUE
}

// API: BOOL FindNextFileA(HANDLE hFindFile, LPWIN32_FIND_DATAA lpFindFileData)
// 역할: FindFirstFileA의 추가 파일 찾기를 실행
pub(super) fn find_next_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hfindfile = uc.read_arg(0);
    let find_file_data_addr = uc.read_arg(1);
    crate::emu_log!(
        "[KERNEL32] FindNextFileA({:#x}, {:#x}) -> FALSE",
        hfindfile,
        find_file_data_addr
    );
    Some(ApiHookResult::callee(2, Some(0)))
}

// API: BOOL FindClose(HANDLE hFindFile)
// 역할: FindFirstFileA에 의해 띄워진 파일 탐색 핸들을 닫음
pub(super) fn find_close(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hfindfile = uc.read_arg(0);
    crate::emu_log!("[KERNEL32] FindClose({:#x}) -> BOOL 1", hfindfile);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: DWORD GetFileAttributesA(LPCSTR lpFileName)
// 역할: 지정된 파일 또는 디렉토리의 파일 시스템 속성을 검색
pub(super) fn get_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    crate::emu_log!(
        "[KERNEL32] GetFileAttributesA(\"{}\") -> INVALID_FILE_ATTRIBUTES",
        name
    );
    Some(ApiHookResult::callee(1, Some(-1i32))) // INVALID_FILE_ATTRIBUTES
}

// API: BOOL SetFileAttributesA(LPCSTR lpFileName, DWORD dwFileAttributes)
// 역할: 지정된 파일 또는 디렉토리의 속성을 설정
pub(super) fn set_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    let attributes = uc.read_arg(1);
    crate::emu_log!(
        "[KERNEL32] SetFileAttributesA(\"{}\", {:#x}) -> BOOL 1",
        name,
        attributes
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL RemoveDirectoryA(LPCSTR lpPathName)
// 역할: 기존의 빈 디렉터리를 삭제
pub(super) fn remove_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    crate::emu_log!("[KERNEL32] RemoveDirectoryA(\"{}\") -> BOOL 1", name);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL CreateDirectoryA(LPCSTR lpPathName, LPSECURITY_ATTRIBUTES lpSecurityAttributes)
// 역할: 새 디렉토리를 생성
pub(super) fn create_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    let security_attributes = uc.read_arg(1);
    crate::emu_log!(
        "[KERNEL32] CreateDirectoryA(\"{}\", {:#x}) -> BOOL 1",
        name,
        security_attributes
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL DeleteFileA(LPCSTR lpFileName)
// 역할: 기존 파일을 삭제
pub(super) fn delete_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    crate::emu_log!("[KERNEL32] DeleteFileA(\"{}\") -> BOOL 1", name);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL CopyFileA(LPCSTR lpExistingFileName, LPCSTR lpNewFileName, BOOL bFailIfExists)
// 역할: 기존 파일을 새 파일로 복사
pub(super) fn copy_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let src_addr = uc.read_arg(0);
    let src = if src_addr != 0 {
        uc.read_euc_kr(src_addr as u64)
    } else {
        String::new()
    };
    let dst_addr = uc.read_arg(1);
    let dst = if dst_addr != 0 {
        uc.read_euc_kr(dst_addr as u64)
    } else {
        String::new()
    };
    let fail_if_exists = uc.read_arg(2);
    crate::emu_log!(
        "[KERNEL32] CopyFileA(\"{}\", \"{}\", {}) -> BOOL 1",
        src,
        dst,
        fail_if_exists
    );
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: DWORD GetTempPathA(DWORD nBufferLength, LPSTR lpBuffer)
// 역할: 임시 파일용 디렉토리 경로를 지정
pub(super) fn get_temp_path_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf_size = uc.read_arg(0);
    let buf_addr = uc.read_arg(1);
    let path = ".\\Temp\\\0";
    uc.mem_write(buf_addr as u64, path.as_bytes()).unwrap();
    crate::emu_log!(
        "[KERNEL32] GetTempPathA({:#x}, {:#x}) -> \"{}\"",
        buf_size,
        buf_addr,
        path
    );
    Some(ApiHookResult::callee(2, Some((path.len() - 1) as i32)))
}

// API: DWORD GetShortPathNameA(LPCSTR lpszLongPath, LPSTR lpszShortPath, DWORD cchBuffer)
// 역할: 지정된 경로의 짧은 경로(8.3 폼) 형태를 가져옴
pub(super) fn get_short_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let long_addr = uc.read_arg(0);
    let long_name = if long_addr != 0 {
        uc.read_euc_kr(long_addr as u64)
    } else {
        String::new()
    };
    let short_addr = uc.read_arg(1);
    let buf_size = uc.read_arg(2);
    let mut bytes = long_name.as_bytes().to_vec();
    bytes.push(0);
    if short_addr != 0 {
        uc.mem_write(short_addr as u64, &bytes).unwrap();
    }
    crate::emu_log!(
        "[KERNEL32] GetShortPathNameA(\"{}\", {:#x}, {}) -> {:#x}",
        long_name,
        short_addr,
        buf_size,
        short_addr
    );
    Some(ApiHookResult::callee(3, Some((bytes.len() - 1) as i32)))
}

// API: DWORD GetFullPathNameA(LPCSTR lpFileName, DWORD nBufferLength, LPSTR lpBuffer, LPSTR *lpFilePart)
// 역할: 지정된 파일의 전체 경로와 파일 이름을 구함
pub(super) fn get_full_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    let buf_size = uc.read_arg(1);
    let buf_addr = uc.read_arg(2);
    let file_part_addr = uc.read_arg(3);
    let full = format!("C:\\4Leaf\\{}\0", name);
    uc.mem_write(buf_addr as u64, full.as_bytes()).unwrap();
    if file_part_addr != 0 {
        uc.write_u32(file_part_addr as u64, buf_addr);
    }
    crate::emu_log!(
        "[KERNEL32] GetFullPathNameA(\"{}\", {}, {:#x}, {:#x}) -> {:#x}",
        name,
        buf_size,
        buf_addr,
        file_part_addr,
        buf_addr
    );
    Some(ApiHookResult::callee(4, Some((full.len() - 1) as i32)))
}

// API: DWORD GetLongPathNameA(LPCSTR lpszShortPath, LPSTR lpszLongPath, DWORD cchBuffer)
// 역할: 지정된 경로의 원래 긴 경로 형태를 가져옴
pub(super) fn get_long_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let short_addr = uc.read_arg(0);
    let short_name = if short_addr != 0 {
        uc.read_euc_kr(short_addr as u64)
    } else {
        String::new()
    };
    let long_addr = uc.read_arg(1);
    let buf_size = uc.read_arg(2);
    let mut bytes = short_name.as_bytes().to_vec();
    bytes.push(0);
    if long_addr != 0 {
        uc.mem_write(long_addr as u64, &bytes).unwrap();
    }
    crate::emu_log!(
        "[KERNEL32] GetLongPathNameA(\"{}\", {:#x}, {}) -> {:#x}",
        short_name,
        long_addr,
        buf_size,
        long_addr
    );
    Some(ApiHookResult::callee(3, Some((bytes.len() - 1) as i32)))
}

// API: BOOL SetFileTime(HANDLE hFile, const FILETIME *lpCreationTime, const FILETIME *lpLastAccessTime, const FILETIME *lpLastWriteTime)
// 역할: 지정된 파일의 날짜 및 시간 정보를 지정
pub(super) fn set_file_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hfile = uc.read_arg(0);
    let creation_time = uc.read_arg(1);
    let last_access_time = uc.read_arg(2);
    let last_write_time = uc.read_arg(3);
    crate::emu_log!(
        "[KERNEL32] SetFileTime({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        hfile,
        creation_time,
        last_access_time,
        last_write_time
    );
    Some(ApiHookResult::callee(4, Some(1)))
}
