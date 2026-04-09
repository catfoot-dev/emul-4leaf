use crate::{
    dll::win32::{ApiHookResult, FileState, Win32Context},
    helper::UnicornHelper,
};
use std::io::{Read, Seek, SeekFrom, Write};
use unicorn_engine::Unicorn;

// =========================================================
// File I/O
// =========================================================

/// 파일 읽기/쓰기 오류를 CRT 상태에 반영합니다.
fn mark_file_error(state: &mut FileState) {
    state.error = true;
}

/// 파일 위치가 바뀌거나 상태를 초기화해야 할 때 EOF/에러 플래그를 지웁니다.
fn clear_file_status(state: &mut FileState) {
    state.eof = false;
    state.error = false;
}

// API: FILE* fopen(const char* filename, const char* mode)
// 역할: 파일을 오픈
pub(super) fn fopen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let filename_addr = uc.read_arg(0);
    let mode_addr = uc.read_arg(1);
    let filename = uc.read_euc_kr(filename_addr as u64);
    let mode = uc.read_euc_kr(mode_addr as u64);

    let filename = crate::resource_dir()
        .join(&filename)
        .to_string_lossy()
        .to_string();
    let mut options = std::fs::OpenOptions::new();
    // Parse mode: r, w, a, +, b, t

    for c in mode.chars() {
        match c {
            'r' => {
                options.read(true);
            }
            'w' => {
                options.write(true).create(true).truncate(true);
            }
            'a' => {
                options.append(true).create(true);
            }
            '+' => {
                options.read(true).write(true);
            }
            _ => {}
        }
    }

    let mut file_result = options.open(&filename);
    if file_result.is_err() && !filename.contains('/') && !filename.contains('\\') {
        let alt_path = crate::resource_dir()
            .join(&filename)
            .to_string_lossy()
            .to_string();
        file_result = options.open(&alt_path);
    }

    match file_result {
        Ok(file) => {
            let context = uc.get_data();
            let handle = context.alloc_handle();
            context.files.lock().unwrap().insert(
                handle,
                FileState {
                    file,
                    path: filename.clone(),
                    eof: false,
                    error: false,
                },
            );
            crate::emu_log!(
                "[MSVCRT] fopen(\"{}\", \"{}\") -> FILE* {:#x}",
                filename,
                mode,
                handle
            );
            Some(ApiHookResult::callee(2, Some(handle as i32)))
        }
        Err(e) => {
            crate::emu_log!(
                "[MSVCRT] fopen(\"{}\", \"{}\") -> FILE* 0 {:?}",
                filename,
                mode,
                e
            );
            Some(ApiHookResult::callee(2, Some(0)))
        }
    }
}

// API: int fclose(FILE* stream)
// 역할: 파일을 닫음
pub(super) fn fclose(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let context = uc.get_data();
    let mut files = context.files.lock().unwrap();
    if files.remove(&{ stream_handle }).is_some() {
        crate::emu_log!("[MSVCRT] fclose({:#x}) -> int 0", stream_handle);
        Some(ApiHookResult::callee(1, Some(0)))
    } else {
        crate::emu_log!("[MSVCRT] fclose({:#x}) -> int -1", stream_handle);
        Some(ApiHookResult::callee(1, Some(-1))) // EOF
    }
}

// API: size_t fread(void* buffer, size_t size, size_t count, FILE* stream)
// 역할: 스트림에서 데이터를 읽음
pub(super) fn fread(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buffer_addr = uc.read_arg(0);
    let size = uc.read_arg(1);
    let count = uc.read_arg(2);
    let stream_handle = uc.read_arg(3);
    let total_size = (size * count) as usize;

    if total_size == 0 {
        return Some(ApiHookResult::callee(4, Some(0)));
    }

    let mut data = vec![0u8; total_size];
    let bytes_read = {
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(state) = files.get_mut(&{ stream_handle }) {
            match state.file.read(&mut data) {
                Ok(bytes_read) => {
                    state.eof = bytes_read < total_size;
                    if bytes_read > 0 {
                        state.error = false;
                    }
                    bytes_read
                }
                Err(_) => {
                    mark_file_error(state);
                    0
                }
            }
        } else {
            0
        }
    };

    if bytes_read > 0 {
        uc.mem_write(buffer_addr as u64, &data[..bytes_read])
            .unwrap();
    }

    let actual_count = (bytes_read as u32 / size) as i32;
    crate::emu_log!(
        "[MSVCRT] fread({:#x}, {:#x}, {:#x}, {:#x}) -> size_t {:#x}",
        stream_handle,
        size,
        count,
        buffer_addr,
        actual_count
    );
    Some(ApiHookResult::callee(4, Some(actual_count)))
}

// API: size_t fwrite(const void* buffer, size_t size, size_t count, FILE* stream)
// 역할: 스트림에 데이터를 씀
pub(super) fn fwrite(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buffer_addr = uc.read_arg(0);
    let size = uc.read_arg(1);
    let count = uc.read_arg(2);
    let stream_handle = uc.read_arg(3);
    let total_size = (size * count) as usize;

    if total_size == 0 {
        return Some(ApiHookResult::callee(4, Some(0)));
    }

    let data = uc.mem_read_as_vec(buffer_addr as u64, total_size).unwrap();
    let bytes_written = {
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(state) = files.get_mut(&{ stream_handle }) {
            match state.file.write(&data) {
                Ok(bytes_written) => {
                    state.error = false;
                    state.eof = false;
                    bytes_written
                }
                Err(_) => {
                    mark_file_error(state);
                    0
                }
            }
        } else {
            0
        }
    };

    let actual_count = (bytes_written as u32 / size) as i32;
    crate::emu_log!(
        "[MSVCRT] fwrite({:#x}, {:#x}, {:#x}, {:#x}) -> size_t {:#x}",
        stream_handle,
        size,
        count,
        buffer_addr,
        actual_count
    );
    Some(ApiHookResult::callee(4, Some(actual_count)))
}

// API: int fseek(FILE* stream, long offset, int origin)
// 역할: 파일 포인터를 특정 위치로 이동
pub(super) fn fseek(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let offset = uc.read_arg(1) as i32 as i64; // Sign-extend long
    let origin = uc.read_arg(2); // 0=SEEK_SET, 1=SEEK_CUR, 2=SEEK_END

    let pos = match origin {
        0 => SeekFrom::Start(offset as u64),
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return Some(ApiHookResult::callee(3, Some(-1))),
    };

    let context = uc.get_data();
    let mut files = context.files.lock().unwrap();
    if let Some(state) = files.get_mut(&{ stream_handle }) {
        match state.file.seek(pos) {
            Ok(new_pos) => {
                clear_file_status(state);
                crate::emu_log!(
                    "[MSVCRT] fseek({:#x}, {:#x}, {:#x}) -> int {:#x}",
                    stream_handle,
                    offset,
                    origin,
                    new_pos
                );
                Some(ApiHookResult::callee(3, Some(0)))
            }
            Err(e) => {
                mark_file_error(state);
                crate::emu_log!(
                    "[MSVCRT] fseek({:#x}, {:#x}, {:#x}) -> int -1 {:?}",
                    stream_handle,
                    offset,
                    origin,
                    e
                );
                Some(ApiHookResult::callee(3, Some(-1)))
            }
        }
    } else {
        crate::emu_log!(
            "[MSVCRT] fseek(handle {:#x}) - handle not found",
            stream_handle
        );
        Some(ApiHookResult::callee(3, Some(-1)))
    }
}

// API: long ftell(FILE* stream)
// 역할: 현재 파일 포인터의 위치를 가져옴
pub(super) fn ftell(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let context = uc.get_data();
    let mut files = context.files.lock().unwrap();
    if let Some(state) = files.get_mut(&{ stream_handle }) {
        match state.file.stream_position() {
            Ok(pos) => {
                crate::emu_log!("[MSVCRT] ftell({:#x}) -> long {:#x}", stream_handle, pos);
                Some(ApiHookResult::callee(1, Some(pos as i32)))
            }
            Err(_) => {
                mark_file_error(state);
                Some(ApiHookResult::callee(1, Some(-1)))
            }
        }
    } else {
        crate::emu_log!(
            "[MSVCRT] ftell({:#x}) -> long -1 (handle not found)",
            stream_handle
        );
        Some(ApiHookResult::callee(1, Some(-1)))
    }
}

// API: int fflush(FILE* stream)
// 역할: 스트림의 버퍼를 플러시(비움)
pub(super) fn fflush(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let context = uc.get_data();
    let mut files = context.files.lock().unwrap();
    if let Some(state) = files.get_mut(&{ stream_handle }) {
        match state.file.flush() {
            Ok(_) => {
                state.error = false;
                crate::emu_log!("[MSVCRT] fflush({:#x}) -> int 0", stream_handle);
                Some(ApiHookResult::callee(1, Some(0)))
            }
            Err(_) => {
                mark_file_error(state);
                crate::emu_log!("[MSVCRT] fflush({:#x}) -> int -1", stream_handle);
                Some(ApiHookResult::callee(1, Some(-1)))
            }
        }
    } else {
        crate::emu_log!("[MSVCRT] fflush({:#x}) -> int -1", stream_handle);
        Some(ApiHookResult::callee(1, Some(-1)))
    }
}

// Low-level I/O
// API: int _open(const char* filename, int oflag, ...)
// 역할: 저수준 파일 기술자를 오픈
pub(super) fn _open(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let filename_addr = uc.read_arg(0);
    let oflag = uc.read_arg(1);
    let filename = uc.read_euc_kr(filename_addr as u64);

    let mut options = std::fs::OpenOptions::new();
    // oflag (from fcntl.h/io.h): O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_APPEND=8, O_CREAT=0x100, O_TRUNC=0x200
    if oflag & 0x1 != 0 {
        options.write(true);
    } else if oflag & 0x2 != 0 {
        options.read(true).write(true);
    } else {
        options.read(true);
    }

    if oflag & 0x0008 != 0 {
        options.append(true);
    }
    if oflag & 0x0100 != 0 {
        options.create(true);
    }
    if oflag & 0x0200 != 0 {
        options.truncate(true);
    }

    let mut file_result = options.open(&filename);
    if file_result.is_err() && !filename.contains('/') && !filename.contains('\\') {
        let alt_path = crate::resource_dir()
            .join(&filename)
            .to_string_lossy()
            .to_string();
        file_result = options.open(&alt_path);
    }

    match file_result {
        Ok(file) => {
            let context = uc.get_data();
            let handle = context.alloc_handle();
            context.files.lock().unwrap().insert(
                handle,
                FileState {
                    file,
                    path: filename.clone(),
                    eof: false,
                    error: false,
                },
            );
            crate::emu_log!(
                "[MSVCRT] _open(\"{}\", {:#x}) -> int {:#x}",
                filename,
                oflag,
                handle
            );
            Some(ApiHookResult::callee(3, Some(handle as i32))) // cdecl, may have pmode
        }
        Err(e) => {
            crate::emu_log!(
                "[MSVCRT] _open(\"{}\", {:#x}) -> int -1: {:?}",
                filename,
                oflag,
                e
            );
            Some(ApiHookResult::callee(3, Some(-1)))
        }
    }
}

// API: int _close(int fd)
// 역할: 파일 기술자를 닫음
pub(super) fn _close(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let fd = uc.read_arg(0);
    let context = uc.get_data();
    if context.files.lock().unwrap().remove(&fd).is_some() {
        crate::emu_log!("[MSVCRT] _close(fd {:#x}) -> int 0", fd);
        Some(ApiHookResult::callee(1, Some(0)))
    } else {
        crate::emu_log!("[MSVCRT] _close(fd {:#x}) -> int -1", fd);
        Some(ApiHookResult::callee(1, Some(-1)))
    }
}

// API: int _read(int fd, void* buffer, unsigned int count)
// 역할: 파일 기술자에서 데이터를 읽음
pub(super) fn _read(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let fd = uc.read_arg(0);
    let buffer_addr = uc.read_arg(1);
    let count = uc.read_arg(2);

    let mut data = vec![0u8; count as usize];
    let bytes_read = {
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(state) = files.get_mut(&fd) {
            match state.file.read(&mut data) {
                Ok(bytes_read) => {
                    state.eof = bytes_read < count as usize;
                    if bytes_read > 0 {
                        state.error = false;
                    }
                    bytes_read
                }
                Err(_) => {
                    mark_file_error(state);
                    0
                }
            }
        } else {
            0
        }
    };

    if bytes_read > 0 {
        uc.mem_write(buffer_addr as u64, &data[..bytes_read])
            .unwrap();
    }

    crate::emu_log!(
        "[MSVCRT] _read({:#x}, {:#x}, {}) -> int {}",
        fd,
        buffer_addr,
        count,
        bytes_read
    );
    Some(ApiHookResult::callee(3, Some(bytes_read as i32)))
}

// API: int _write(int fd, const void* buffer, unsigned int count)
// 역할: 파일 기술자에 데이터를 씀
pub(super) fn _write(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let fd = uc.read_arg(0);
    let buffer_addr = uc.read_arg(1);
    let count = uc.read_arg(2);

    let data = uc
        .mem_read_as_vec(buffer_addr as u64, count as usize)
        .unwrap();
    let bytes_written = {
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(state) = files.get_mut(&fd) {
            match state.file.write(&data) {
                Ok(bytes_written) => {
                    state.error = false;
                    state.eof = false;
                    bytes_written
                }
                Err(_) => {
                    mark_file_error(state);
                    0
                }
            }
        } else {
            0
        }
    };

    crate::emu_log!(
        "[MSVCRT] _write({:#x}, {:#x}, {}) -> int {}",
        fd,
        buffer_addr,
        count,
        bytes_written
    );
    Some(ApiHookResult::callee(3, Some(bytes_written as i32)))
}

// API: long _lseek(int fd, long offset, int origin)
// 역할: 파일 기술자의 읽기/쓰기 위치를 이동
pub(super) fn _lseek(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let fd = uc.read_arg(0);
    let offset = uc.read_arg(1) as i32 as i64;
    let origin = uc.read_arg(2);

    let pos = match origin {
        0 => SeekFrom::Start(offset as u64),
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return Some(ApiHookResult::callee(3, Some(-1))),
    };

    let context = uc.get_data();
    let mut files = context.files.lock().unwrap();
    if let Some(state) = files.get_mut(&fd) {
        match state.file.seek(pos) {
            Ok(new_pos) => {
                clear_file_status(state);
                crate::emu_log!(
                    "[MSVCRT] _lseek({:#x}, {}, {}) -> int {:#x}",
                    fd,
                    offset,
                    origin,
                    new_pos
                );
                Some(ApiHookResult::callee(3, Some(new_pos as i32)))
            }
            Err(_) => {
                mark_file_error(state);
                Some(ApiHookResult::callee(3, Some(-1)))
            }
        }
    } else {
        crate::emu_log!(
            "[MSVCRT] _lseek({:#x}, {}, {}) -> int -1 (fd not found)",
            fd,
            offset,
            origin
        );
        Some(ApiHookResult::callee(3, Some(-1)))
    }
}

// API: int feof(FILE* stream)
// 역할: 스트림이 EOF 상태인지 확인
pub(super) fn feof(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let eof = uc
        .get_data()
        .files
        .lock()
        .unwrap()
        .get(&stream_handle)
        .map(|state| state.eof)
        .unwrap_or(false);
    crate::emu_log!("[MSVCRT] feof({:#x}) -> int {}", stream_handle, eof as i32);
    Some(ApiHookResult::callee(1, Some(eof as i32)))
}

// API: int ferror(FILE* stream)
// 역할: 스트림이 에러 상태인지 확인
pub(super) fn ferror(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let error = uc
        .get_data()
        .files
        .lock()
        .unwrap()
        .get(&stream_handle)
        .map(|state| state.error)
        .unwrap_or(false);
    crate::emu_log!(
        "[MSVCRT] ferror({:#x}) -> int {}",
        stream_handle,
        error as i32
    );
    Some(ApiHookResult::callee(1, Some(error as i32)))
}

// API: void clearerr(FILE* stream)
// 역할: 스트림의 EOF/에러 상태를 지움
pub(super) fn clearerr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    if let Some(state) = uc.get_data().files.lock().unwrap().get_mut(&stream_handle) {
        clear_file_status(state);
    }
    crate::emu_log!("[MSVCRT] clearerr({:#x}) -> void", stream_handle);
    Some(ApiHookResult::callee(1, Some(0)))
}

// API: void rewind(FILE* stream)
// 역할: 파일 포인터를 처음으로 되돌리고 상태를 초기화
pub(super) fn rewind(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    if let Some(state) = uc.get_data().files.lock().unwrap().get_mut(&stream_handle) {
        let _ = state.file.seek(SeekFrom::Start(0));
        clear_file_status(state);
    }
    crate::emu_log!("[MSVCRT] rewind({:#x}) -> void", stream_handle);
    Some(ApiHookResult::callee(1, Some(0)))
}

// API: int _pipe(int* phandles, unsigned int size, int oflag)
// 역할: 익명 파이프를 생성
pub(super) fn _pipe(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let phandles = uc.read_arg(0);
    let size = uc.read_arg(1);
    let oflag = uc.read_arg(2);
    crate::emu_log!(
        "[MSVCRT] _pipe({:#x}, {}, {}) -> int -1",
        phandles,
        size,
        oflag
    );
    Some(ApiHookResult::callee(3, Some(-1))) // cdecl
}

// API: int _stat(const char* filename, struct _stat* buffer)
// 역할: 파일의 상태 정보를 가져옴
pub(super) fn _stat(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let filename_addr = uc.read_arg(0);
    let buffer_addr = uc.read_arg(1);
    let filename = uc.read_euc_kr(filename_addr as u64);

    if let Ok(metadata) = std::fs::metadata(&filename) {
        let mut stat_buf = vec![0u8; 64];
        let size = metadata.len() as u32;
        let mode = if metadata.is_dir() { 0x4000 } else { 0x8000 } | 0o666;

        // Simplified VC6 _stat layout
        stat_buf[4..6].copy_from_slice(&(mode as u16).to_le_bytes());
        stat_buf[14..18].copy_from_slice(&size.to_le_bytes());

        uc.mem_write(buffer_addr as u64, &stat_buf).unwrap();
        crate::emu_log!(
            "[MSVCRT] _stat(\"{}\", {:#x}) -> int 0",
            filename,
            buffer_addr
        );
        Some(ApiHookResult::callee(2, Some(0)))
    } else {
        crate::emu_log!(
            "[MSVCRT] _stat(\"{}\", {:#x}) -> int -1",
            filename,
            buffer_addr
        );
        Some(ApiHookResult::callee(2, Some(-1)))
    }
}
