use crate::{
    dll::win32::{ApiHookResult, StackCleanup, Win32Context},
    helper::UnicornHelper,
};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
};
use unicorn_engine::{RegisterX86, Unicorn};

const BASIC_STRING_PTR_OFFSET: u64 = 4;
const BASIC_STRING_LEN_OFFSET: u64 = 8;
const BASIC_STRING_RES_OFFSET: u64 = 12;

const IOS_STREAMBUF_OFFSET: u64 = 4;
const IOS_STREAMBUF_ALT_OFFSET: u64 = 40;
const IOS_FLAGS_OFFSET: u64 = 8;
const IOS_STATE_OFFSET: u64 = 16;
const IOS_LOCALE_OFFSET: u64 = 20;

const STREAMBUF_BUFFER_OFFSET: u64 = 4;
const STREAMBUF_CAPACITY_OFFSET: u64 = 8;
const STREAMBUF_READ_POS_OFFSET: u64 = 12;
const STREAMBUF_WRITE_POS_OFFSET: u64 = 16;
const STREAMBUF_LOCALE_OFFSET: u64 = 20;
const STREAMBUF_LAST_CHAR_OFFSET: u64 = 24;
const STREAMBUF_FILE_HANDLE_OFFSET: u64 = 28;
const STREAMBUF_FILE_MODE_OFFSET: u64 = 32;

const FACET_REFCOUNT_OFFSET: u64 = 4;

const STREAM_OBJECT_SIZE: usize = 0x80;
const STREAMBUF_OBJECT_SIZE: usize = 0x40;
const LOCALE_OBJECT_SIZE: usize = 0x20;
const LOCIMP_OBJECT_SIZE: usize = 0x20;
const STREAMBUF_HOST_BUFFER_SIZE: usize = 0x400;

const EMPTY_STRING_CACHE_KEY: &str = "$internal_empty_cstr";
const LOCALE_IMPL_CACHE_KEY: &str = "$internal_locale_locimp";
const LOCALE_VALUE_CACHE_KEY: &str = "$internal_locale_value";

const BASIC_STREAMBUF_VTABLE: &str = "??_7?$basic_streambuf@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_FILEBUF_VTABLE: &str = "??_7?$basic_filebuf@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_OSTREAM_VTABLE: &str = "??_7?$basic_ostream@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_ISTREAM_VTABLE: &str = "??_7?$basic_istream@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_IOSTREAM_VTABLE: &str = "??_7?$basic_iostream@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_IOS_VTABLE: &str = "??_7?$basic_ios@DU?$char_traits@D@std@@@std@@6B@";
const IOS_BASE_VTABLE: &str = "??_7ios_base@std@@6B@";
/// `MSVCP60.dll` 프록시 구현 모듈
///
/// VC6 STL의 문자열/스트림/locale 초기화에 필요한 최소 런타임 상태를 에뮬레이션합니다.
pub struct MSVCP60;

impl MSVCP60 {
    fn is_cdecl_symbol(func_name: &str) -> bool {
        func_name.contains("@YA") || func_name.contains("@Y?A")
    }

    fn this_ptr(uc: &Unicorn<Win32Context>) -> u32 {
        uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32
    }

    fn proxy_cache_key(func_name: &str) -> String {
        format!("MSVCP60.dll!{}", func_name)
    }

    fn alloc_zeroed(uc: &mut Unicorn<Win32Context>, size: usize) -> u32 {
        let addr = uc.malloc(size) as u32;
        if size != 0 {
            let zeros = vec![0u8; size];
            let _ = uc.mem_write(addr as u64, &zeros);
        }
        addr
    }

    fn read_exact_bytes(uc: &Unicorn<Win32Context>, addr: u32, len: usize) -> Vec<u8> {
        if addr == 0 || len == 0 {
            return Vec::new();
        }

        let mut buf = vec![0u8; len];
        if uc.mem_read(addr as u64, &mut buf).is_err() {
            return Vec::new();
        }
        buf
    }

    fn is_mapped_ptr(uc: &Unicorn<Win32Context>, addr: u32) -> bool {
        if addr == 0 {
            return false;
        }
        let mut probe = [0u8; 1];
        uc.mem_read(addr as u64, &mut probe).is_ok()
    }

    fn cached_proxy_export<F>(uc: &mut Unicorn<Win32Context>, func_name: &str, init: F) -> u32
    where
        F: FnOnce(&mut Unicorn<Win32Context>) -> u32,
    {
        let key = Self::proxy_cache_key(func_name);
        if let Some(addr) = {
            let context = uc.get_data();
            context.proxy_exports.lock().unwrap().get(&key).copied()
        } {
            return addr;
        }

        let addr = init(uc);
        {
            let context = uc.get_data();
            context.proxy_exports.lock().unwrap().insert(key, addr);
        }
        addr
    }

    fn vtable_export_addr(uc: &mut Unicorn<Win32Context>, name: &str) -> u32 {
        Self::cached_proxy_export(uc, name, |uc| Self::alloc_zeroed(uc, 64))
    }

    fn empty_c_string_addr(uc: &mut Unicorn<Win32Context>) -> u32 {
        Self::cached_proxy_export(uc, EMPTY_STRING_CACHE_KEY, |uc| {
            let addr = Self::alloc_zeroed(uc, 1);
            uc.write_u8(addr as u64, 0);
            addr
        })
    }

    fn locale_impl_addr(uc: &mut Unicorn<Win32Context>) -> u32 {
        Self::cached_proxy_export(uc, LOCALE_IMPL_CACHE_KEY, |uc| {
            let addr = Self::alloc_zeroed(uc, LOCIMP_OBJECT_SIZE);
            uc.write_u32(addr as u64 + FACET_REFCOUNT_OFFSET, 1);
            addr
        })
    }

    fn locale_value_addr(uc: &mut Unicorn<Win32Context>) -> u32 {
        Self::cached_proxy_export(uc, LOCALE_VALUE_CACHE_KEY, |uc| {
            let addr = Self::alloc_zeroed(uc, LOCALE_OBJECT_SIZE);
            let locimp_addr = Self::locale_impl_addr(uc);
            Self::write_locale_value(uc, addr, locimp_addr);
            addr
        })
    }

    fn write_locale_value(uc: &mut Unicorn<Win32Context>, locale_addr: u32, locimp_addr: u32) {
        if locale_addr != 0 {
            // 원본은 locale 스택 객체의 뒤쪽 필드도 상태 분기에 사용하므로
            // locimp 포인터만 쓰지 말고 전체 객체를 0으로 초기화합니다.
            let _ = uc.mem_write(locale_addr as u64, &[0u8; LOCALE_OBJECT_SIZE]);
            uc.write_u32(locale_addr as u64, locimp_addr);
        }
    }

    fn read_locale_impl(uc: &Unicorn<Win32Context>, locale_addr: u32) -> u32 {
        if locale_addr == 0 {
            return 0;
        }
        uc.read_u32(locale_addr as u64)
    }

    fn init_streambuf_layout(uc: &mut Unicorn<Win32Context>, this_ptr: u32, vtable_name: &str) {
        if this_ptr == 0 {
            return;
        }

        let vtable_addr = Self::vtable_export_addr(uc, vtable_name);
        let locale_addr = Self::locale_value_addr(uc);
        uc.write_u32(this_ptr as u64, vtable_addr);
        uc.write_u32(this_ptr as u64 + STREAMBUF_BUFFER_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_CAPACITY_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_READ_POS_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_WRITE_POS_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_LOCALE_OFFSET, locale_addr);
        uc.write_u32(this_ptr as u64 + STREAMBUF_LAST_CHAR_OFFSET, u32::MAX);
        uc.write_u32(this_ptr as u64 + STREAMBUF_FILE_HANDLE_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_FILE_MODE_OFFSET, 0);
    }

    fn init_filebuf_layout(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        file_handle: u32,
        mode: u32,
    ) {
        Self::init_streambuf_layout(uc, this_ptr, BASIC_FILEBUF_VTABLE);
        let buffer_addr = Self::alloc_zeroed(uc, STREAMBUF_HOST_BUFFER_SIZE);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buffer_addr);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_CAPACITY_OFFSET,
            STREAMBUF_HOST_BUFFER_SIZE as u32,
        );
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET, file_handle);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_MODE_OFFSET, mode);
    }

    fn init_ios_base_layout(uc: &mut Unicorn<Win32Context>, this_ptr: u32, vtable_name: &str) {
        if this_ptr == 0 {
            return;
        }

        let vtable_addr = Self::vtable_export_addr(uc, vtable_name);
        let locale_addr = Self::locale_value_addr(uc);
        uc.write_u32(this_ptr as u64, vtable_addr);
        uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + IOS_LOCALE_OFFSET, locale_addr);
    }

    fn init_basic_ios_layout(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        vtable_name: &str,
        streambuf_ptr: u32,
    ) {
        Self::init_ios_base_layout(uc, this_ptr, vtable_name);
        Self::write_basic_ios_streambuf_ptr(uc, this_ptr, streambuf_ptr);
    }

    fn write_basic_ios_streambuf_ptr(uc: &mut Unicorn<Win32Context>, this_ptr: u32, streambuf_ptr: u32) {
        if this_ptr == 0 {
            return;
        }
        uc.write_u32(this_ptr as u64 + IOS_STREAMBUF_OFFSET, streambuf_ptr);
        uc.write_u32(this_ptr as u64 + IOS_STREAMBUF_ALT_OFFSET, streambuf_ptr);
    }

    fn looks_like_streambuf_ptr(uc: &Unicorn<Win32Context>, ptr: u32) -> bool {
        ptr != 0 && Self::is_mapped_ptr(uc, ptr) && uc.read_u32(ptr as u64) != 0
    }

    fn read_basic_ios_streambuf_ptr(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        let primary = uc.read_u32(this_ptr as u64 + IOS_STREAMBUF_OFFSET);
        if Self::looks_like_streambuf_ptr(uc, primary) {
            return primary;
        }
        let alternate = uc.read_u32(this_ptr as u64 + IOS_STREAMBUF_ALT_OFFSET);
        if Self::looks_like_streambuf_ptr(uc, alternate) {
            return alternate;
        }
        primary
    }

    fn init_global_stream_object(uc: &mut Unicorn<Win32Context>, func_name: &str) -> u32 {
        Self::cached_proxy_export(uc, func_name, |uc| {
            let stream_addr = Self::alloc_zeroed(uc, STREAM_OBJECT_SIZE);
            let buffer_addr = Self::alloc_zeroed(uc, STREAMBUF_OBJECT_SIZE);
            Self::init_streambuf_layout(uc, buffer_addr, BASIC_STREAMBUF_VTABLE);

            let vtable = match func_name {
                "?cin@std@@3V?$basic_istream@DU?$char_traits@D@std@@@1@A" => BASIC_ISTREAM_VTABLE,
                _ => BASIC_OSTREAM_VTABLE,
            };
            Self::init_basic_ios_layout(uc, stream_addr, vtable, buffer_addr);
            stream_addr
        })
    }

    fn init_basic_string_empty(uc: &mut Unicorn<Win32Context>, this_ptr: u32) {
        if this_ptr == 0 {
            return;
        }

        let empty_addr = Self::empty_c_string_addr(uc);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET, empty_addr);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_LEN_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_RES_OFFSET, 0);
    }

    fn basic_string_len(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        uc.read_u32(this_ptr as u64 + BASIC_STRING_LEN_OFFSET)
    }

    fn basic_string_ptr(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        uc.read_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET)
    }

    fn basic_string_bytes(uc: &Unicorn<Win32Context>, this_ptr: u32) -> Vec<u8> {
        let len = Self::basic_string_len(uc, this_ptr) as usize;
        let ptr = Self::basic_string_ptr(uc, this_ptr);
        Self::read_exact_bytes(uc, ptr, len)
    }

    fn source_bytes_from_ptr(
        uc: &Unicorn<Win32Context>,
        ptr: u32,
        explicit_len: Option<usize>,
    ) -> Vec<u8> {
        if ptr == 0 {
            return Vec::new();
        }

        match explicit_len {
            Some(len) => Self::read_exact_bytes(uc, ptr, len),
            None => uc.read_string_bytes(ptr as u64, 4096),
        }
    }

    fn write_bytes_to_new_buffer(uc: &mut Unicorn<Win32Context>, data: &[u8]) -> u32 {
        if data.is_empty() {
            return Self::empty_c_string_addr(uc);
        }

        let addr = Self::alloc_zeroed(uc, data.len() + 1);
        let _ = uc.mem_write(addr as u64, data);
        uc.write_u8(addr as u64 + data.len() as u64, 0);
        addr
    }

    fn set_basic_string_bytes(uc: &mut Unicorn<Win32Context>, this_ptr: u32, data: &[u8]) {
        if this_ptr == 0 {
            return;
        }

        let ptr = Self::write_bytes_to_new_buffer(uc, data);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET, ptr);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_LEN_OFFSET, data.len() as u32);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_RES_OFFSET, data.len() as u32);
    }

    fn ensure_basic_string_capacity(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        capacity: usize,
        preserve_current: bool,
    ) {
        if this_ptr == 0 {
            return;
        }

        let current = if preserve_current {
            Self::basic_string_bytes(uc, this_ptr)
        } else {
            Vec::new()
        };
        let preserved_len = current.len().min(capacity);
        let new_ptr = if capacity == 0 {
            Self::empty_c_string_addr(uc)
        } else {
            let addr = Self::alloc_zeroed(uc, capacity + 1);
            if preserved_len != 0 {
                let _ = uc.mem_write(addr as u64, &current[..preserved_len]);
            }
            uc.write_u8(addr as u64 + preserved_len as u64, 0);
            addr
        };

        uc.write_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET, new_ptr);
        uc.write_u32(
            this_ptr as u64 + BASIC_STRING_LEN_OFFSET,
            preserved_len as u32,
        );
        uc.write_u32(this_ptr as u64 + BASIC_STRING_RES_OFFSET, capacity as u32);
    }

    fn basic_string_subrange(
        uc: &Unicorn<Win32Context>,
        source_ptr: u32,
        offset: u32,
        count: u32,
    ) -> Vec<u8> {
        let bytes = Self::basic_string_bytes(uc, source_ptr);
        let start = (offset as usize).min(bytes.len());
        let end = if count == u32::MAX {
            bytes.len()
        } else {
            start.saturating_add(count as usize).min(bytes.len())
        };
        bytes[start..end].to_vec()
    }

    fn basic_string_replace_range(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        pos: usize,
        remove_len: usize,
        replacement: &[u8],
    ) {
        let current = Self::basic_string_bytes(uc, this_ptr);
        let start = pos.min(current.len());
        let end = start.saturating_add(remove_len).min(current.len());

        let mut next = Vec::with_capacity(current.len() + replacement.len());
        next.extend_from_slice(&current[..start]);
        next.extend_from_slice(replacement);
        next.extend_from_slice(&current[end..]);
        Self::set_basic_string_bytes(uc, this_ptr, &next);
    }

    fn read_streambuf_field(uc: &Unicorn<Win32Context>, this_ptr: u32, offset: u64) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        uc.read_u32(this_ptr as u64 + offset)
    }

    fn write_streambuf_field(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        offset: u64,
        value: u32,
    ) {
        if this_ptr != 0 {
            uc.write_u32(this_ptr as u64 + offset, value);
        }
    }

    fn streambuf_file_handle(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        Self::read_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET)
    }

    fn ensure_streambuf_buffer(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> (u32, usize) {
        let mut buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let mut capacity =
            Self::read_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET) as usize;
        if buffer_ptr == 0 || capacity == 0 {
            buffer_ptr = Self::alloc_zeroed(uc, STREAMBUF_HOST_BUFFER_SIZE);
            capacity = STREAMBUF_HOST_BUFFER_SIZE;
            Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buffer_ptr);
            Self::write_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET, capacity as u32);
        }
        (buffer_ptr, capacity)
    }

    fn open_host_file_by_name(
        uc: &mut Unicorn<Win32Context>,
        raw_filename: &str,
        mode: u32,
    ) -> Option<u32> {
        if raw_filename.is_empty() {
            return None;
        }

        let mut candidates = vec![raw_filename.to_string()];
        if !raw_filename.contains('/') && !raw_filename.contains('\\') {
            candidates.insert(0, format!("Resources/{}", raw_filename));
        }

        let want_read = mode == 0 || (mode & 0x01) != 0;
        let mut want_write = (mode & 0x02) != 0;
        let seek_to_end = (mode & 0x04) != 0;
        let append = (mode & 0x08) != 0;
        let truncate = (mode & 0x10) != 0;

        if append {
            want_write = true;
        }

        for candidate in candidates {
            let mut options = OpenOptions::new();
            options.read(want_read || !want_write);
            options.write(want_write);
            options.append(append);
            options.create(want_write);
            options.truncate(truncate);

            let mut file = match options.open(&candidate) {
                Ok(file) => file,
                Err(_) => continue,
            };

            if seek_to_end {
                let _ = file.seek(SeekFrom::End(0));
            }

            let handle = {
                let context = uc.get_data();
                let handle = context.alloc_handle();
                context.files.lock().unwrap().insert(handle, file);
                handle
            };
            return Some(handle);
        }

        None
    }

    fn open_host_file_from_guest(
        uc: &mut Unicorn<Win32Context>,
        filename_ptr: u32,
        mode: u32,
    ) -> Option<(u32, String)> {
        let filename = if filename_ptr != 0 {
            uc.read_euc_kr(filename_ptr as u64)
        } else {
            String::new()
        };
        Self::open_host_file_by_name(uc, &filename, mode).map(|handle| (handle, filename))
    }

    fn attach_version_dat_fallback(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
    ) -> bool {
        if this_ptr == 0 || Self::streambuf_file_handle(uc, this_ptr) != 0 {
            return false;
        }
        if Self::streambuf_available(uc, this_ptr) != 0 {
            return false;
        }

        let mode = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_FILE_MODE_OFFSET);
        let fallback_mode = if mode == 0 { 1 } else { mode };
        let Some(file_handle) = Self::open_host_file_by_name(uc, "version.dat", fallback_mode)
        else {
            return false;
        };

        // 원본 런타임에서는 이 시점에 Version.dat가 이미 연결돼 있어야 합니다.
        // 현재 에뮬레이터는 일부 생성 경로가 누락돼 있어, 빈 filebuf일 때만 국소적으로 보정합니다.
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET, file_handle);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_MODE_OFFSET, fallback_mode);
        let _ = Self::refill_streambuf_from_file(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) attached fallback Resources/version.dat",
            this_ptr
        );
        true
    }

    fn close_streambuf_file_handle(uc: &mut Unicorn<Win32Context>, this_ptr: u32) {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return;
        }

        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        files.remove(&file_handle);
        drop(files);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET, 0);
    }

    fn refill_streambuf_from_file(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> usize {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return 0;
        }

        let (buffer_ptr, capacity) = Self::ensure_streambuf_buffer(uc, this_ptr);
        let mut data = vec![0u8; capacity];
        let bytes_read = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&file_handle) {
                file.read(&mut data).unwrap_or(0)
            } else {
                0
            }
        };

        if bytes_read != 0 {
            let _ = uc.mem_write(buffer_ptr as u64, &data[..bytes_read]);
        }
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, bytes_read as u32);
        bytes_read
    }

    fn prepare_streambuf_read(uc: &mut Unicorn<Win32Context>, this_ptr: u32) {
        if Self::streambuf_available(uc, this_ptr) == 0 {
            let _ = Self::refill_streambuf_from_file(uc, this_ptr);
        }
    }

    fn streambuf_peek_byte(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> Option<u8> {
        Self::prepare_streambuf_read(uc, this_ptr);
        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        let available = Self::streambuf_available(uc, this_ptr);
        if buffer_ptr == 0 || available == 0 {
            None
        } else {
            Some(uc.read_u8(buffer_ptr as u64 + read_pos as u64))
        }
    }

    fn streambuf_take_byte(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> Option<u8> {
        let value = Self::streambuf_peek_byte(uc, this_ptr)?;
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, read_pos + 1);
        Some(value)
    }

    fn write_bytes_to_streambuf(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        bytes: &[u8],
    ) -> usize {
        if bytes.is_empty() {
            return 0;
        }

        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle != 0 {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&file_handle) {
                return file.write(bytes).unwrap_or(0);
            }
            return 0;
        }

        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let capacity = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET);
        let write_pos =
            Self::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET) as usize;
        if buffer_ptr == 0 || capacity == 0 {
            return 0;
        }

        let writable = bytes.len().min(capacity as usize).saturating_sub(write_pos);
        if writable == 0 {
            return 0;
        }

        let _ = uc.mem_write(buffer_ptr as u64 + write_pos as u64, &bytes[..writable]);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_WRITE_POS_OFFSET,
            (write_pos + writable) as u32,
        );
        writable
    }

    fn seek_streambuf_file(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        position: SeekFrom,
    ) -> Option<u32> {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return None;
        }

        let next = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            let file = files.get_mut(&file_handle)?;
            file.seek(position).ok()? as u32
        };

        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, 0);
        Some(next)
    }

    fn streambuf_copy_assign(uc: &mut Unicorn<Win32Context>, this_ptr: u32, other_ptr: u32) {
        if this_ptr == 0 || other_ptr == 0 {
            return;
        }

        for offset in [
            STREAMBUF_BUFFER_OFFSET,
            STREAMBUF_CAPACITY_OFFSET,
            STREAMBUF_READ_POS_OFFSET,
            STREAMBUF_WRITE_POS_OFFSET,
            STREAMBUF_LOCALE_OFFSET,
            STREAMBUF_LAST_CHAR_OFFSET,
        ] {
            let value = uc.read_u32(other_ptr as u64 + offset);
            uc.write_u32(this_ptr as u64 + offset, value);
        }
    }

    fn ios_base_copy_assign(uc: &mut Unicorn<Win32Context>, this_ptr: u32, other_ptr: u32) {
        if this_ptr == 0 || other_ptr == 0 {
            return;
        }

        for offset in [IOS_FLAGS_OFFSET, IOS_STATE_OFFSET, IOS_LOCALE_OFFSET] {
            let value = uc.read_u32(other_ptr as u64 + offset);
            uc.write_u32(this_ptr as u64 + offset, value);
        }
    }

    fn basic_ios_copy_assign(uc: &mut Unicorn<Win32Context>, this_ptr: u32, other_ptr: u32) {
        Self::ios_base_copy_assign(uc, this_ptr, other_ptr);
        if this_ptr != 0 && other_ptr != 0 {
            let streambuf_ptr = Self::read_basic_ios_streambuf_ptr(uc, other_ptr);
            Self::write_basic_ios_streambuf_ptr(uc, this_ptr, streambuf_ptr);
        }
    }

    fn basic_ostream_write_bytes(uc: &mut Unicorn<Win32Context>, this_ptr: u32, bytes: &[u8]) {
        if this_ptr == 0 || bytes.is_empty() {
            return;
        }

        let streambuf_ptr = Self::read_basic_ios_streambuf_ptr(uc, this_ptr);
        if streambuf_ptr == 0 {
            return;
        }
        let _ = Self::write_bytes_to_streambuf(uc, streambuf_ptr, bytes);
    }

    fn streambuf_available(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        let write_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET);
        write_pos.saturating_sub(read_pos)
    }

    fn streambuf_return_fpos(
        uc: &mut Unicorn<Win32Context>,
        pos: u32,
        cleanup_without_hidden: usize,
        cleanup_with_hidden: usize,
    ) -> ApiHookResult {
        let ret_ptr = uc.read_arg(0);
        if Self::is_mapped_ptr(uc, ret_ptr) {
            uc.write_u32(ret_ptr as u64, pos);
            uc.write_u32(ret_ptr as u64 + 4, 0);
            ApiHookResult::callee(cleanup_with_hidden, Some(ret_ptr as i32))
        } else {
            ApiHookResult::callee(cleanup_without_hidden, Some(pos as i32))
        }
    }

    fn basic_string_ctor_default(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let allocator = uc.read_arg(0);
        Self::init_basic_string_empty(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::basic_string({:#x}) -> (this={:#x})",
            this_ptr,
            allocator,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_string_ctor_cstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let str_ptr = uc.read_arg(0);
        let allocator = uc.read_arg(1);
        let bytes = Self::source_bytes_from_ptr(uc, str_ptr, None);
        Self::set_basic_string_bytes(uc, this_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::basic_string({:#x}, {:#x}) -> (this={:#x})",
            this_ptr,
            str_ptr,
            allocator,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_string_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::init_basic_string_empty(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::~basic_string()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn basic_string_tidy(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let preserve = uc.read_arg(0);
        Self::init_basic_string_empty(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Tidy({}) -> VOID",
            this_ptr,
            preserve
        );
        Some(ApiHookResult::callee(1, None))
    }

    fn basic_string_grow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let size = uc.read_arg(0) as usize;
        let preserve = uc.read_arg(1) != 0;
        Self::ensure_basic_string_capacity(uc, this_ptr, size, preserve);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Grow({}, {}) -> BOOL 1",
            this_ptr,
            size,
            preserve
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    fn basic_string_copy(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let size = uc.read_arg(0) as usize;
        Self::ensure_basic_string_capacity(uc, this_ptr, size, true);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Copy({}) -> VOID",
            this_ptr,
            size
        );
        Some(ApiHookResult::callee(1, None))
    }

    fn basic_string_eos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let size = uc.read_arg(0) as usize;
        let mut current = Self::basic_string_bytes(uc, this_ptr);
        current.resize(size, 0);
        Self::set_basic_string_bytes(uc, this_ptr, &current);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Eos({}) -> VOID",
            this_ptr,
            size
        );
        Some(ApiHookResult::callee(1, None))
    }

    fn basic_string_freeze(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) basic_string::_Freeze()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_string_split(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) basic_string::_Split()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_string_assign_ptr_len(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ptr = uc.read_arg(0);
        let len = uc.read_arg(1) as usize;
        let bytes = Self::source_bytes_from_ptr(uc, ptr, Some(len));
        Self::set_basic_string_bytes(uc, this_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::assign({:#x}, {}) -> (this={:#x})",
            this_ptr,
            ptr,
            len,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_string_assign_ptr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ptr = uc.read_arg(0);
        let bytes = Self::source_bytes_from_ptr(uc, ptr, None);
        Self::set_basic_string_bytes(uc, this_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::assign({:#x}) -> (this={:#x})",
            this_ptr,
            ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_string_assign_substr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        let offset = uc.read_arg(1);
        let count = uc.read_arg(2);
        let bytes = Self::basic_string_subrange(uc, other_ptr, offset, count);
        Self::set_basic_string_bytes(uc, this_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::assign({:#x}, {}, {}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            offset,
            count,
            this_ptr
        );
        Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
    }

    fn basic_string_append_substr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        let offset = uc.read_arg(1);
        let count = uc.read_arg(2);

        let mut bytes = Self::basic_string_bytes(uc, this_ptr);
        bytes.extend(Self::basic_string_subrange(uc, other_ptr, offset, count));
        Self::set_basic_string_bytes(uc, this_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::append({:#x}, {}, {}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            offset,
            count,
            this_ptr
        );
        Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
    }

    fn basic_string_compare_other(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        let lhs = Self::basic_string_bytes(uc, this_ptr);
        let rhs = Self::basic_string_bytes(uc, other_ptr);
        let cmp = lhs.cmp(&rhs);
        let result = match cmp {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::compare({:#x}) -> {}",
            this_ptr,
            other_ptr,
            result
        );
        Some(ApiHookResult::callee(1, Some(result)))
    }

    fn basic_string_compare_ptr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let offset = uc.read_arg(0);
        let count = uc.read_arg(1);
        let ptr = uc.read_arg(2);
        let len = uc.read_arg(3) as usize;

        let lhs = Self::basic_string_subrange(uc, this_ptr, offset, count);
        let rhs = Self::source_bytes_from_ptr(uc, ptr, Some(len));
        let cmp = lhs.cmp(&rhs);
        let result = match cmp {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::compare({}, {}, {:#x}, {}) -> {}",
            this_ptr,
            offset,
            count,
            ptr,
            len,
            result
        );
        Some(ApiHookResult::callee(4, Some(result)))
    }

    fn basic_string_erase(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let offset = uc.read_arg(0) as usize;
        let count = uc.read_arg(1) as usize;
        Self::basic_string_replace_range(uc, this_ptr, offset, count, &[]);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::erase({}, {}) -> (this={:#x})",
            this_ptr,
            offset,
            count,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_string_replace_repeat(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let pos = uc.read_arg(0) as usize;
        let remove_len = uc.read_arg(1) as usize;
        let repeat = uc.read_arg(2) as usize;
        let ch = uc.read_arg(3) as u8;
        let replacement = vec![ch; repeat];
        Self::basic_string_replace_range(uc, this_ptr, pos, remove_len, &replacement);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::replace({}, {}, {}, '{}') -> (this={:#x})",
            this_ptr,
            pos,
            remove_len,
            repeat,
            ch as char,
            this_ptr
        );
        Some(ApiHookResult::callee(4, Some(this_ptr as i32)))
    }

    fn basic_string_replace_range_ptrs(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let begin_ptr = uc.read_arg(0);
        let end_ptr = uc.read_arg(1);
        let src_begin = uc.read_arg(2);
        let src_end = uc.read_arg(3);

        let base_ptr = Self::basic_string_ptr(uc, this_ptr);
        let len = Self::basic_string_len(uc, this_ptr);
        let start = begin_ptr.saturating_sub(base_ptr).min(len) as usize;
        let end = end_ptr.saturating_sub(base_ptr).min(len) as usize;
        let replacement_len = src_end.saturating_sub(src_begin) as usize;
        let replacement = Self::read_exact_bytes(uc, src_begin, replacement_len);
        Self::basic_string_replace_range(
            uc,
            this_ptr,
            start,
            end.saturating_sub(start),
            &replacement,
        );
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::replace({:#x}, {:#x}, {:#x}, {:#x}) -> (this={:#x})",
            this_ptr,
            begin_ptr,
            end_ptr,
            src_begin,
            src_end,
            this_ptr
        );
        Some(ApiHookResult::callee(4, Some(this_ptr as i32)))
    }

    fn basic_string_resize(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let target = uc.read_arg(0) as usize;
        let mut current = Self::basic_string_bytes(uc, this_ptr);
        current.resize(target, 0);
        Self::set_basic_string_bytes(uc, this_ptr, &current);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::resize({}) -> VOID",
            this_ptr,
            target
        );
        Some(ApiHookResult::callee(1, None))
    }

    fn basic_string_swap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        if this_ptr != 0 && other_ptr != 0 {
            for offset in [
                BASIC_STRING_PTR_OFFSET,
                BASIC_STRING_LEN_OFFSET,
                BASIC_STRING_RES_OFFSET,
            ] {
                let lhs = uc.read_u32(this_ptr as u64 + offset);
                let rhs = uc.read_u32(other_ptr as u64 + offset);
                uc.write_u32(this_ptr as u64 + offset, rhs);
                uc.write_u32(other_ptr as u64 + offset, lhs);
            }
        }
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::swap({:#x})",
            this_ptr,
            other_ptr
        );
        Some(ApiHookResult::callee(1, None))
    }

    fn basic_string_c_str(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ptr = Self::basic_string_ptr(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::c_str() -> {:#x}",
            this_ptr,
            ptr
        );
        Some(ApiHookResult::callee(0, Some(ptr as i32)))
    }

    fn basic_string_end(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ptr = Self::basic_string_ptr(uc, this_ptr);
        let end = ptr.saturating_add(Self::basic_string_len(uc, this_ptr));
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::end() -> {:#x}",
            this_ptr,
            end
        );
        Some(ApiHookResult::callee(0, Some(end as i32)))
    }

    fn basic_string_size(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let size = Self::basic_string_len(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::size() -> {}",
            this_ptr,
            size
        );
        Some(ApiHookResult::callee(0, Some(size as i32)))
    }

    fn basic_string_max_size(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let max_size = 0x7fff_fffeu32;
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::max_size() -> {}",
            this_ptr,
            max_size
        );
        Some(ApiHookResult::callee(0, Some(max_size as i32)))
    }

    fn basic_string_substr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ret_ptr = uc.read_arg(0);
        let has_hidden_ret = Self::is_mapped_ptr(uc, ret_ptr);
        let offset_index = if has_hidden_ret { 1 } else { 0 };
        let count_index = if has_hidden_ret { 2 } else { 1 };
        let offset = uc.read_arg(offset_index);
        let count = uc.read_arg(count_index);
        let bytes = Self::basic_string_subrange(uc, this_ptr, offset, count);

        let result_ptr = if has_hidden_ret {
            ret_ptr
        } else {
            Self::alloc_zeroed(uc, 16)
        };
        Self::init_basic_string_empty(uc, result_ptr);
        Self::set_basic_string_bytes(uc, result_ptr, &bytes);

        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::substr({}, {}) -> {:#x}",
            this_ptr,
            offset,
            count,
            result_ptr
        );
        Some(ApiHookResult::callee(
            if has_hidden_ret { 3 } else { 2 },
            Some(result_ptr as i32),
        ))
    }

    fn basic_streambuf_copy_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        Self::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
        Self::streambuf_copy_assign(uc, this_ptr, other_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::basic_streambuf({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_streambuf_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::~basic_streambuf()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_streambuf_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        Self::streambuf_copy_assign(uc, this_ptr, other_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::operator=({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_streambuf_setbuf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let buf_ptr = uc.read_arg(0);
        let len = uc.read_arg(1);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buf_ptr);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET, len);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, 0);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::setbuf({:#x}, {}) -> (this={:#x})",
            this_ptr,
            buf_ptr,
            len,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_streambuf_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
        crate::emu_log!("[MSVCP60] (this={:#x}) basic_streambuf::_Init()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_streambuf_init_ranges(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let arg0 = uc.read_arg(0);
        let arg1 = uc.read_arg(1);
        let arg2 = uc.read_arg(2);
        let arg3 = uc.read_arg(3);
        let arg4 = uc.read_arg(4);
        let arg5 = uc.read_arg(5);

        Self::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
        for ptr in [arg0, arg1, arg3, arg4] {
            if Self::is_mapped_ptr(uc, ptr) {
                uc.write_u32(ptr as u64, 0);
            }
        }
        for ptr in [arg2, arg5] {
            if Self::is_mapped_ptr(uc, ptr) {
                uc.write_u32(ptr as u64, 0);
            }
        }

        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::_Init({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
            this_ptr,
            arg0,
            arg1,
            arg2,
            arg3,
            arg4,
            arg5
        );
        Some(ApiHookResult::callee(6, None))
    }

    fn basic_streambuf_setg(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let begin = uc.read_arg(0);
        let current = uc.read_arg(1);
        let end = uc.read_arg(2);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, begin);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_CAPACITY_OFFSET,
            end.saturating_sub(begin),
        );
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_READ_POS_OFFSET,
            current.saturating_sub(begin),
        );
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_WRITE_POS_OFFSET,
            end.saturating_sub(begin),
        );
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::setg({:#x}, {:#x}, {:#x})",
            this_ptr,
            begin,
            current,
            end
        );
        Some(ApiHookResult::callee(3, None))
    }

    fn basic_streambuf_setp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let begin = uc.read_arg(0);
        let end = uc.read_arg(1);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, begin);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_CAPACITY_OFFSET,
            end.saturating_sub(begin),
        );
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, 0);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::setp({:#x}, {:#x})",
            this_ptr,
            begin,
            end
        );
        Some(ApiHookResult::callee(2, None))
    }

    fn basic_streambuf_seekoff(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ret_ptr = uc.read_arg(0);
        let has_hidden_ret = Self::is_mapped_ptr(uc, ret_ptr);
        let off_index = if has_hidden_ret { 1 } else { 0 };
        let dir_index = if has_hidden_ret { 2 } else { 1 };
        let off = uc.read_arg(off_index) as i32;
        let seekdir = uc.read_arg(dir_index);
        if let Some(next) = Self::seek_streambuf_file(
            uc,
            this_ptr,
            match seekdir {
                1 => SeekFrom::Current(off as i64),
                2 => SeekFrom::End(off as i64),
                _ => SeekFrom::Start(off.max(0) as u64),
            },
        ) {
            crate::emu_log!(
                "[MSVCP60] (this={:#x}) basic_streambuf::seekoff({}, {}) -> {}",
                this_ptr,
                off,
                seekdir,
                next
            );
            return Some(Self::streambuf_return_fpos(
                uc,
                next,
                3,
                if has_hidden_ret { 4 } else { 3 },
            ));
        }
        let available = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET) as i32;
        let current = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET) as i32;

        let next = match seekdir {
            1 => current.saturating_add(off),
            2 => available.saturating_add(off),
            _ => off,
        }
        .clamp(0, available) as u32;
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, next);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::seekoff({}, {}) -> {}",
            this_ptr,
            off,
            seekdir,
            next
        );
        Some(Self::streambuf_return_fpos(
            uc,
            next,
            3,
            if has_hidden_ret { 4 } else { 3 },
        ))
    }

    fn basic_streambuf_seekpos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ret_ptr = uc.read_arg(0);
        let has_hidden_ret = Self::is_mapped_ptr(uc, ret_ptr);
        let pos_index = if has_hidden_ret { 1 } else { 0 };
        let pos = uc.read_arg(pos_index);
        let mode_index = if has_hidden_ret { 3 } else { 2 };
        let mode = uc.read_arg(mode_index);
        if let Some(next) = Self::seek_streambuf_file(uc, this_ptr, SeekFrom::Start(pos as u64)) {
            crate::emu_log!(
                "[MSVCP60] (this={:#x}) basic_streambuf::seekpos({}, {}) -> {}",
                this_ptr,
                pos,
                mode,
                next
            );
            return Some(Self::streambuf_return_fpos(
                uc,
                next,
                3,
                if has_hidden_ret { 4 } else { 3 },
            ));
        }
        let write_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET);
        let next = pos.min(write_pos);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, next);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::seekpos({}, {}) -> {}",
            this_ptr,
            pos,
            mode,
            next
        );
        Some(Self::streambuf_return_fpos(
            uc,
            next,
            3,
            if has_hidden_ret { 4 } else { 3 },
        ))
    }

    fn basic_streambuf_xsputn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ptr = uc.read_arg(0);
        let len = uc.read_arg(1) as usize;
        let bytes = Self::source_bytes_from_ptr(uc, ptr, Some(len));
        let written = Self::write_bytes_to_streambuf(uc, this_ptr, &bytes);

        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::xsputn({:#x}, {}) -> {}",
            this_ptr,
            ptr,
            len,
            written
        );
        Some(ApiHookResult::callee(2, Some(written as i32)))
    }

    fn basic_streambuf_xsgetn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let dst_ptr = uc.read_arg(0);
        let requested = uc.read_arg(1) as usize;
        Self::prepare_streambuf_read(uc, this_ptr);
        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET) as usize;
        let available = Self::streambuf_available(uc, this_ptr) as usize;
        let count = requested.min(available);
        if count != 0 && dst_ptr != 0 && buffer_ptr != 0 {
            let bytes = Self::read_exact_bytes(uc, buffer_ptr + read_pos as u32, count);
            let _ = uc.mem_write(dst_ptr as u64, &bytes);
        }
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_READ_POS_OFFSET,
            (read_pos + count) as u32,
        );
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::xsgetn({:#x}, {}) -> {}",
            this_ptr,
            dst_ptr,
            requested,
            count
        );
        Some(ApiHookResult::callee(2, Some(count as i32)))
    }

    fn basic_streambuf_underflow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::prepare_streambuf_read(uc, this_ptr);
        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        let available = Self::streambuf_available(uc, this_ptr);
        let value = if buffer_ptr != 0 && available != 0 {
            uc.read_u8(buffer_ptr as u64 + read_pos as u64) as i32
        } else {
            -1
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::underflow() -> {}",
            this_ptr,
            value
        );
        Some(ApiHookResult::callee(0, Some(value)))
    }

    fn basic_streambuf_uflow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let value = Self::basic_streambuf_underflow(uc)?
            .return_value
            .unwrap_or(-1);
        if value >= 0 {
            let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
            Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, read_pos + 1);
        }
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::uflow() -> {}",
            this_ptr,
            value
        );
        Some(ApiHookResult::callee(0, Some(value)))
    }

    fn basic_streambuf_showmanyc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::prepare_streambuf_read(uc, this_ptr);
        let value = Self::streambuf_available(uc, this_ptr) as i32;
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::showmanyc() -> {}",
            this_ptr,
            value
        );
        Some(ApiHookResult::callee(0, Some(value)))
    }

    fn basic_streambuf_pbackfail(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ch = uc.read_arg(0) as i32;
        Self::prepare_streambuf_read(uc, this_ptr);
        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        let result = if buffer_ptr != 0 && read_pos != 0 {
            let new_pos = read_pos - 1;
            Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, new_pos);
            if ch >= 0 {
                uc.write_u8(buffer_ptr as u64 + new_pos as u64, ch as u8);
            }
            uc.read_u8(buffer_ptr as u64 + new_pos as u64) as i32
        } else {
            -1
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::pbackfail({}) -> {}",
            this_ptr,
            ch,
            result
        );
        Some(ApiHookResult::callee(1, Some(result)))
    }

    fn basic_streambuf_sync(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        let result = if file_handle != 0 {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&file_handle) {
                file.flush().map(|_| 0).unwrap_or(-1)
            } else {
                -1
            }
        } else {
            0
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::sync() -> {}",
            this_ptr,
            result
        );
        Some(ApiHookResult::callee(0, Some(result)))
    }

    fn basic_ostream_copy_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        Self::init_basic_ios_layout(
            uc,
            this_ptr,
            BASIC_OSTREAM_VTABLE,
            uc.read_u32(other_ptr as u64 + IOS_STREAMBUF_OFFSET),
        );
        Self::basic_ios_copy_assign(uc, this_ptr, other_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_ostream_ctor3(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let buf = uc.read_arg(0);
        let flags = uc.read_arg(1);
        let tied = uc.read_arg(2);
        Self::init_basic_ios_layout(uc, this_ptr, BASIC_OSTREAM_VTABLE, buf);
        uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, flags);
        let _ = tied;
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}, {}, {}) -> (this={:#x})",
            this_ptr,
            buf,
            flags,
            tied,
            this_ptr
        );
        Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
    }

    fn basic_ostream_ctor2(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let buf = uc.read_arg(0);
        let flags = uc.read_arg(1);
        Self::init_basic_ios_layout(uc, this_ptr, BASIC_OSTREAM_VTABLE, buf);
        uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, flags);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}, {}) -> (this={:#x})",
            this_ptr,
            buf,
            flags,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_ostream_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ostream::~basic_ostream()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_ostream_write(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ptr = uc.read_arg(0);
        let len = uc.read_arg(1) as usize;
        let bytes = Self::source_bytes_from_ptr(uc, ptr, Some(len));
        Self::basic_ostream_write_bytes(uc, this_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ostream::write({:#x}, {}) -> (this={:#x})",
            this_ptr,
            ptr,
            len,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_istream_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let buf = uc.read_arg(0);
        let flags = uc.read_arg(1);
        Self::init_basic_ios_layout(uc, this_ptr, BASIC_ISTREAM_VTABLE, buf);
        uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, flags);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_istream::basic_istream({:#x}, {}) -> (this={:#x})",
            this_ptr,
            buf,
            flags,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_istream_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_istream::~basic_istream()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_istream_seekg(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let pos = uc.read_arg(0);
        let _high = uc.read_arg(1);
        let streambuf_ptr = Self::read_basic_ios_streambuf_ptr(uc, this_ptr);
        if Self::seek_streambuf_file(uc, streambuf_ptr, SeekFrom::Start(pos as u64)).is_some() {
            crate::emu_log!(
                "[MSVCP60] (this={:#x}) basic_istream::seekg({:#x}) -> (this={:#x})",
                this_ptr,
                pos,
                this_ptr
            );
            return Some(ApiHookResult::callee(2, Some(this_ptr as i32)));
        }
        Self::write_streambuf_field(uc, streambuf_ptr, STREAMBUF_READ_POS_OFFSET, pos);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_istream::seekg({:#x}) -> (this={:#x})",
            this_ptr,
            pos,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    fn basic_istream_extract_unsigned_short(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let out_ptr = uc.read_arg(0);
        let streambuf_ptr = Self::read_basic_ios_streambuf_ptr(uc, this_ptr);
        let used_fallback = Self::attach_version_dat_fallback(uc, streambuf_ptr);

        while let Some(byte) = Self::streambuf_peek_byte(uc, streambuf_ptr) {
            if !(byte as char).is_ascii_whitespace() {
                break;
            }
            let _ = Self::streambuf_take_byte(uc, streambuf_ptr);
        }

        let mut token = Vec::new();
        while let Some(byte) = Self::streambuf_peek_byte(uc, streambuf_ptr) {
            if !(byte as char).is_ascii_digit() {
                break;
            }
            if let Some(next) = Self::streambuf_take_byte(uc, streambuf_ptr) {
                token.push(next);
            }
        }

        let mut state = 0;
        if token.is_empty() {
            state |= 0x4;
            if Self::streambuf_peek_byte(uc, streambuf_ptr).is_none() {
                state |= 0x2;
            }
        } else if let Ok(text) = std::str::from_utf8(&token) {
            if let Ok(value) = text.parse::<u16>() {
                if out_ptr != 0 {
                    uc.write_u16(out_ptr as u64, value);
                }
                if Self::streambuf_peek_byte(uc, streambuf_ptr).is_none() {
                    state |= 0x2;
                }
            } else {
                state |= 0x4;
            }
        } else {
            state |= 0x4;
        }
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);

        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_istream::operator>>({:#x}) fallback={}",
            this_ptr,
            out_ptr,
            used_fallback
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_istream_getline(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let buf_addr = uc.read_arg(0);
        let count = uc.read_arg(1) as usize;
        let delim = uc.read_arg(2) as u8;
        if buf_addr != 0 && count != 0 {
            uc.write_u8(buf_addr as u64, 0);
        }
        let state = 0x2 | 0x4;
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_istream::getline({:#x}, {}, '{}')",
            this_ptr,
            buf_addr,
            count,
            delim as char
        );
        Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
    }

    fn basic_iostream_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let streambuf_ptr = uc.read_arg(0);
        Self::init_basic_ios_layout(uc, this_ptr, BASIC_IOSTREAM_VTABLE, streambuf_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_iostream::basic_iostream({:#x}) -> (this={:#x})",
            this_ptr,
            streambuf_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_iostream_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_iostream::~basic_iostream()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_filebuf_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let file_ptr = uc.read_arg(0);
        Self::init_filebuf_layout(uc, this_ptr, file_ptr, 0);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_filebuf::basic_filebuf({:#x}) -> (this={:#x})",
            this_ptr,
            file_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_filebuf_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let file_ptr = uc.read_arg(0);
        let init_flag = uc.read_arg(1);
        Self::init_filebuf_layout(uc, this_ptr, file_ptr, init_flag);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_filebuf::_Init({:#x}, {})",
            this_ptr,
            file_ptr,
            init_flag
        );
        Some(ApiHookResult::callee(2, None))
    }

    fn basic_filebuf_open(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let filename_ptr = uc.read_arg(0);
        let mode = uc.read_arg(1);
        let (result, filename) = if let Some((file_handle, filename)) =
            Self::open_host_file_from_guest(uc, filename_ptr, mode)
        {
            Self::close_streambuf_file_handle(uc, this_ptr);
            Self::init_filebuf_layout(uc, this_ptr, file_handle, mode);
            (this_ptr, filename)
        } else {
            (
                0,
                if filename_ptr != 0 {
                    uc.read_euc_kr(filename_ptr as u64)
                } else {
                    String::new()
                },
            )
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_filebuf::open(\"{}\", {}) -> (this={:#x})",
            this_ptr,
            filename,
            mode,
            result
        );
        Some(ApiHookResult::callee(2, Some(result as i32)))
    }

    fn basic_filebuf_initcvt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) basic_filebuf::_Initcvt()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_filebuf_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let _ = Self::basic_streambuf_sync(uc);
        Self::close_streambuf_file_handle(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_filebuf::~basic_filebuf()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_ifstream_vbase_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ifstream::`vbase dtor`()",
            this_ptr
        );
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_ios_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) basic_ios::~basic_ios()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn basic_ios_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::init_basic_ios_layout(uc, this_ptr, BASIC_IOS_VTABLE, 0);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ios::basic_ios() -> (this={:#x})",
            this_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn basic_ios_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        Self::basic_ios_copy_assign(uc, this_ptr, other_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ios::operator=({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn basic_ios_clear(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let state = uc.read_arg(0);
        let _throw = uc.read_arg(1);
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ios::clear({})",
            this_ptr,
            state
        );
        Some(ApiHookResult::callee(2, None))
    }

    fn basic_ios_setstate(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let state = uc.read_arg(0);
        let _throw = uc.read_arg(1);
        let next = uc.read_u32(this_ptr as u64 + IOS_STATE_OFFSET) | state;
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, next);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ios::setstate({})",
            this_ptr,
            state
        );
        Some(ApiHookResult::callee(2, None))
    }

    fn basic_ios_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let buf = uc.read_arg(0);
        let _flags = uc.read_arg(1);
        Self::init_basic_ios_layout(uc, this_ptr, BASIC_IOS_VTABLE, buf);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ios::init({:#x})",
            this_ptr,
            buf
        );
        Some(ApiHookResult::callee(2, None))
    }

    fn basic_ios_widen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ch = uc.read_arg(0) as u8;
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ios::widen('{}') -> '{}'",
            this_ptr,
            ch as char,
            ch as char
        );
        Some(ApiHookResult::callee(1, Some(ch as i32)))
    }

    fn ios_base_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::init_ios_base_layout(uc, this_ptr, IOS_BASE_VTABLE);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) ios_base::ios_base() -> (this={:#x})",
            this_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn ios_base_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::~ios_base()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn ios_base_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        Self::ios_base_copy_assign(uc, this_ptr, other_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) ios_base::operator=({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn ios_base_clear(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let state = uc.read_arg(0);
        let _throw = uc.read_arg(1);
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) ios_base::clear({})",
            this_ptr,
            state
        );
        Some(ApiHookResult::callee(2, None))
    }

    fn ios_base_copyfmt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        Self::ios_base_copy_assign(uc, this_ptr, other_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) ios_base::copyfmt({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn ios_base_getloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let ret_ptr = uc.read_arg(0);
        let result_ptr = if Self::is_mapped_ptr(uc, ret_ptr) {
            ret_ptr
        } else {
            Self::alloc_zeroed(uc, LOCALE_OBJECT_SIZE)
        };

        let locimp = uc.read_u32(this_ptr as u64 + IOS_LOCALE_OFFSET);
        let locimp = if locimp != 0 {
            Self::read_locale_impl(uc, locimp)
        } else {
            Self::locale_impl_addr(uc)
        };
        Self::write_locale_value(uc, result_ptr, locimp);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) ios_base::getloc() -> {:#x}",
            this_ptr,
            result_ptr
        );
        Some(ApiHookResult::callee(
            if Self::is_mapped_ptr(uc, ret_ptr) {
                1
            } else {
                0
            },
            Some(result_ptr as i32),
        ))
    }

    fn ios_base_init_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::Init::Init()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn ios_base_init_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::Init::~Init()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn ios_base_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        Self::init_ios_base_layout(uc, this_ptr, IOS_BASE_VTABLE);
        crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::_Init()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn locale_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let locimp_addr = Self::locale_impl_addr(uc);
        crate::emu_log!("[MSVCP60] locale::_Init() -> {:#x}", locimp_addr);
        Some(ApiHookResult::caller(Some(locimp_addr as i32)))
    }

    fn locale_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let locimp_addr = Self::locale_impl_addr(uc);
        Self::write_locale_value(uc, this_ptr, locimp_addr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) locale::locale() -> (this={:#x})",
            this_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn locale_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) locale::~locale()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn locale_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let other_ptr = uc.read_arg(0);
        let locimp = Self::read_locale_impl(uc, other_ptr);
        Self::write_locale_value(uc, this_ptr, locimp);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) locale::operator=({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn locale_facet_incref(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        if this_ptr != 0 {
            let current = uc.read_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET);
            let next = current.max(1).saturating_add(1);
            uc.write_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET, next);
        }
        crate::emu_log!("[MSVCP60] (this={:#x}) locale::facet::_Incref()", this_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn locale_facet_decref(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let result = if this_ptr != 0 {
            let current = uc.read_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET).max(1);
            let next = current.saturating_sub(1);
            uc.write_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET, next);
            if next == 0 { 0 } else { this_ptr }
        } else {
            0
        };
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) locale::facet::_Decref() -> {:#x}",
            this_ptr,
            result
        );
        Some(ApiHookResult::callee(0, Some(result as i32)))
    }

    fn streambuf_imbue(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let locale_ptr = uc.read_arg(0);
        let fallback_locale = Self::locale_value_addr(uc);
        let locale_value = if locale_ptr != 0 {
            locale_ptr
        } else {
            fallback_locale
        };
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_LOCALE_OFFSET, locale_value);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::imbue({:#x})",
            this_ptr,
            locale_ptr
        );
        Some(ApiHookResult::callee(1, None))
    }

    fn streambuf_init_strstream(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let flags = uc.read_arg(0);
        let buffer = uc.read_arg(1);
        let end = uc.read_arg(2);
        let len = uc.read_arg(3);
        Self::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buffer);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET, len);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_WRITE_POS_OFFSET,
            end.saturating_sub(buffer),
        );
        let _ = flags;
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) strstreambuf::_Init({}, {:#x}, {:#x}, {})",
            this_ptr,
            flags,
            buffer,
            end,
            len
        );
        Some(ApiHookResult::callee(4, None))
    }

    fn fiopen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let filename_ptr = uc.read_arg(0);
        let mode = uc.read_arg(1);
        let (handle, filename) = if let Some((file_handle, filename)) =
            Self::open_host_file_from_guest(uc, filename_ptr, mode)
        {
            (file_handle, filename)
        } else {
            (
                0,
                if filename_ptr != 0 {
                    uc.read_euc_kr(filename_ptr as u64)
                } else {
                    String::new()
                },
            )
        };
        crate::emu_log!(
            "[MSVCP60] __Fiopen(\"{}\", {}) -> {:#x}",
            filename,
            mode,
            handle
        );
        Some(ApiHookResult::callee(2, Some(handle as i32)))
    }

    fn winit_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) _Winit::_Winit()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn winit_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) _Winit::~_Winit()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn lockit_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) _Lockit::_Lockit()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn lockit_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        crate::emu_log!("[MSVCP60] (this={:#x}) _Lockit::~_Lockit()", this_ptr);
        Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
    }

    fn ostream_insert_int(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = Self::this_ptr(uc);
        let value = uc.read_arg(0);
        Self::basic_ostream_write_bytes(uc, this_ptr, value.to_string().as_bytes());
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_ostream::operator<<({}) -> (this={:#x})",
            this_ptr,
            value,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    fn ostream_insert_cstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let os_ptr = uc.read_arg(0);
        let str_ptr = uc.read_arg(1);
        let bytes = Self::source_bytes_from_ptr(uc, str_ptr, None);
        Self::basic_ostream_write_bytes(uc, os_ptr, &bytes);
        crate::emu_log!(
            "[MSVCP60] std::operator<<({:#x}, {:#x}) -> (this={:#x})",
            os_ptr,
            str_ptr,
            os_ptr
        );
        Some(ApiHookResult::callee(2, Some(os_ptr as i32)))
    }

    fn ostream_insert_char(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let os_ptr = uc.read_arg(0);
        let ch = uc.read_arg(1) as u8;
        Self::basic_ostream_write_bytes(uc, os_ptr, &[ch]);
        crate::emu_log!(
            "[MSVCP60] std::operator<<({:#x}, '{}') -> (this={:#x})",
            os_ptr,
            ch as char,
            os_ptr
        );
        Some(ApiHookResult::callee(2, Some(os_ptr as i32)))
    }

    fn ostream_flush(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let os_ptr = uc.read_arg(0);
        crate::emu_log!(
            "[MSVCP60] std::flush({:#x}) -> (this={:#x})",
            os_ptr,
            os_ptr
        );
        Some(ApiHookResult::callee(1, Some(os_ptr as i32)))
    }

    fn ostream_endl(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let os_ptr = uc.read_arg(0);
        Self::basic_ostream_write_bytes(uc, os_ptr, b"\n");
        crate::emu_log!("[MSVCP60] std::endl({:#x}) -> (this={:#x})", os_ptr, os_ptr);
        Some(ApiHookResult::callee(1, Some(os_ptr as i32)))
    }

    fn xlen(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        crate::emu_log!("[MSVCP60] std::_Xlen()");
        Some(ApiHookResult::caller(None))
    }

    fn xran(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        crate::emu_log!("[MSVCP60] std::_Xran()");
        Some(ApiHookResult::caller(None))
    }

    fn xoff(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        crate::emu_log!("[MSVCP60] std::_Xoff()");
        Some(ApiHookResult::caller(None))
    }

    fn nullstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let addr = Self::empty_c_string_addr(uc);
        crate::emu_log!("[MSVCP60] basic_string::_Nullstr() -> {:#x}", addr);
        Some(ApiHookResult::caller(Some(addr as i32)))
    }

    /// `MSVCP60.dll`의 데이터/전역 export 주소를 해소합니다.
    ///
    /// 함수 호출이 아닌 전역 객체, vtable, 정적 데이터 심볼만 처리합니다.
    pub fn resolve_export(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<u32> {
        match func_name {
            "?npos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@2IB" => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    uc.write_u32(addr as u64, u32::MAX);
                    crate::emu_log!("[MSVCP60] basic_string::npos resolved to {:#x}", addr);
                    addr
                }))
            }
            "?_C@?1??_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ@4DB" =>
            {
                let addr = Self::empty_c_string_addr(uc);
                crate::emu_log!(
                    "[MSVCP60] basic_string::_Nullstr data resolved to {:#x}",
                    addr
                );
                Some(addr)
            }
            "?_Fpz@std@@3_JB" => Some(Self::cached_proxy_export(uc, func_name, |uc| {
                let addr = Self::alloc_zeroed(uc, 8);
                crate::emu_log!("[MSVCP60] std::_Fpz resolved to {:#x}", addr);
                addr
            })),
            name if name.starts_with("??_7") || name.starts_with("??_8") => {
                let addr = Self::vtable_export_addr(uc, name);
                crate::emu_log!("[MSVCP60] vtable/vbtable {} resolved to {:#x}", name, addr);
                Some(addr)
            }
            "?cin@std@@3V?$basic_istream@DU?$char_traits@D@std@@@1@A"
            | "?cout@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"
            | "?cerr@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"
            | "?clog@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A" => {
                let addr = Self::init_global_stream_object(uc, func_name);
                crate::emu_log!(
                    "[MSVCP60] Global object {} resolved to {:#x}",
                    func_name,
                    addr
                );
                Some(addr)
            }
            "?_Global@_Locimp@locale@std@@0PAV123@A"
            | "?_Clocptr@_Locimp@locale@std@@0PAV123@A" => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    let locimp_addr = Self::locale_impl_addr(uc);
                    uc.write_u32(addr as u64, locimp_addr);
                    crate::emu_log!(
                        "[MSVCP60] Global locale ptr {} resolved to {:#x}",
                        func_name,
                        addr
                    );
                    addr
                }))
            }
            "?_Id_cnt@facet@locale@std@@0HA" | "?_Id_cnt@id@locale@std@@0HA" => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    crate::emu_log!(
                        "[MSVCP60] Facet ID counter {} resolved to {:#x}",
                        func_name,
                        addr
                    );
                    addr
                }))
            }
            "?_Stinit@?1??_Init@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXPAU_iobuf@@W4_Initfl@23@@Z@4HA" => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    crate::emu_log!("[MSVCP60] Static init flag resolved to {:#x}", addr);
                    addr
                }))
            }
            _ => None,
        }
    }

    /// 함수명 기준으로 `MSVCP60.dll` API를 처리합니다.
    ///
    /// 초기화 경로에서 실제 호출되는 STL 심볼을 guest 메모리 상태와 함께 최소 구현합니다.
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        let is_cdecl = Self::is_cdecl_symbol(func_name);
        let result = match func_name {
            "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@ABV?$allocator@D@1@@Z" => {
                Self::basic_string_ctor_default(uc)
            }
            "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@PBDABV?$allocator@D@1@@Z" => {
                Self::basic_string_ctor_cstr(uc)
            }
            "??1?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@XZ" => {
                Self::basic_string_destructor(uc)
            }
            "?_Tidy@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEX_N@Z" => {
                Self::basic_string_tidy(uc)
            }
            "?_Grow@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAE_NI_N@Z" => {
                Self::basic_string_grow(uc)
            }
            "?_Copy@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXI@Z" => {
                Self::basic_string_copy(uc)
            }
            "?_Eos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXI@Z" => {
                Self::basic_string_eos(uc)
            }
            "?_Freeze@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXXZ" => {
                Self::basic_string_freeze(uc)
            }
            "?_Split@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXXZ" => {
                Self::basic_string_split(uc)
            }
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z" => {
                Self::basic_string_assign_ptr_len(uc)
            }
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBD@Z" => {
                Self::basic_string_assign_ptr(uc)
            }
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z" => {
                Self::basic_string_assign_substr(uc)
            }
            "?append@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z" => {
                Self::basic_string_append_substr(uc)
            }
            "?compare@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEHABV12@@Z" => {
                Self::basic_string_compare_other(uc)
            }
            "?compare@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEHIIPBDI@Z" => {
                Self::basic_string_compare_ptr(uc)
            }
            "?erase@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@II@Z" => {
                Self::basic_string_erase(uc)
            }
            "?replace@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@IIID@Z" => {
                Self::basic_string_replace_repeat(uc)
            }
            "?replace@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PAD0PBD1@Z" => {
                Self::basic_string_replace_range_ptrs(uc)
            }
            "?resize@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEXI@Z" => {
                Self::basic_string_resize(uc)
            }
            "?swap@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEXAAV12@@Z" => {
                Self::basic_string_swap(uc)
            }
            "?substr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBE?AV12@II@Z" => {
                Self::basic_string_substr(uc)
            }
            "?c_str@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEPBDXZ" => {
                Self::basic_string_c_str(uc)
            }
            "?end@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEPADXZ" => {
                Self::basic_string_end(uc)
            }
            "?size@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEIXZ" => {
                Self::basic_string_size(uc)
            }
            "?max_size@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEIXZ" => {
                Self::basic_string_max_size(uc)
            }
            "?_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ" => {
                Self::nullstr(uc)
            }

            "??0?$basic_streambuf@DU?$char_traits@D@std@@@std@@QAE@ABV01@@Z" => {
                Self::basic_streambuf_copy_ctor(uc)
            }
            "?_Init@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXXZ" => {
                Self::basic_streambuf_init(uc)
            }
            "?_Init@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXPAPAD0PAH001@Z" => {
                Self::basic_streambuf_init_ranges(uc)
            }
            "??1?$basic_streambuf@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                Self::basic_streambuf_destructor(uc)
            }
            "??4?$basic_streambuf@DU?$char_traits@D@std@@@std@@QAEAAV01@ABV01@@Z" => {
                Self::basic_streambuf_assign(uc)
            }
            "?setg@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXPAD00@Z" => {
                Self::basic_streambuf_setg(uc)
            }
            "?setp@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXPAD0@Z" => {
                Self::basic_streambuf_setp(uc)
            }
            "?imbue@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEXABVlocale@2@@Z" => {
                Self::streambuf_imbue(uc)
            }
            "?setbuf@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEPAV12@PADH@Z" => {
                Self::basic_streambuf_setbuf(uc)
            }
            "?seekoff@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAE?AV?$fpos@H@2@JW4seekdir@ios_base@2@H@Z" => {
                Self::basic_streambuf_seekoff(uc)
            }
            "?seekpos@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAE?AV?$fpos@H@2@V32@H@Z" => {
                Self::basic_streambuf_seekpos(uc)
            }
            "?xsputn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHPBDH@Z" => {
                Self::basic_streambuf_xsputn(uc)
            }
            "?xsgetn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHPADH@Z" => {
                Self::basic_streambuf_xsgetn(uc)
            }
            "?underflow@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ" => {
                Self::basic_streambuf_underflow(uc)
            }
            "?uflow@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ" => {
                Self::basic_streambuf_uflow(uc)
            }
            "?showmanyc@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ" => {
                Self::basic_streambuf_showmanyc(uc)
            }
            "?pbackfail@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHH@Z" => {
                Self::basic_streambuf_pbackfail(uc)
            }
            "?sync@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ" => {
                Self::basic_streambuf_sync(uc)
            }

            "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@ABV01@@Z" => {
                Self::basic_ostream_copy_ctor(uc)
            }
            "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N1@Z" => {
                Self::basic_ostream_ctor3(uc)
            }
            "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z" => {
                Self::basic_ostream_ctor2(uc)
            }
            "??1?$basic_ostream@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                Self::basic_ostream_destructor(uc)
            }
            "??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV01@H@Z" => {
                Self::ostream_insert_int(uc)
            }
            "?write@?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV12@PBDH@Z" => {
                Self::basic_ostream_write(uc)
            }

            "??0?$basic_istream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z" => {
                Self::basic_istream_ctor(uc)
            }
            "??1?$basic_istream@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                Self::basic_istream_destructor(uc)
            }
            "?seekg@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@V?$fpos@H@2@@Z" => {
                Self::basic_istream_seekg(uc)
            }
            "?getline@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@PADHD@Z" => {
                Self::basic_istream_getline(uc)
            }

            "??0?$basic_iostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z" => {
                Self::basic_iostream_ctor(uc)
            }
            "??1?$basic_iostream@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                Self::basic_iostream_destructor(uc)
            }

            "??0?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAE@PAU_iobuf@@@Z" => {
                Self::basic_filebuf_ctor(uc)
            }
            "?_Init@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXPAU_iobuf@@W4_Initfl@12@@Z" => {
                Self::basic_filebuf_init(uc)
            }
            "?open@?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAEPAV12@PBDH@Z" => {
                Self::basic_filebuf_open(uc)
            }
            "?_Initcvt@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXXZ" => {
                Self::basic_filebuf_initcvt(uc)
            }
            "??1?$basic_filebuf@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                Self::basic_filebuf_destructor(uc)
            }
            "??_D?$basic_ifstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                Self::basic_ifstream_vbase_dtor(uc)
            }

            "??0?$basic_ios@DU?$char_traits@D@std@@@std@@IAE@XZ" => Self::basic_ios_ctor(uc),
            "?clear@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z" => {
                Self::basic_ios_clear(uc)
            }
            "?setstate@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z" => {
                Self::basic_ios_setstate(uc)
            }
            "?init@?$basic_ios@DU?$char_traits@D@std@@@std@@IAEXPAV?$basic_streambuf@DU?$char_traits@D@std@@@2@_N@Z" => {
                Self::basic_ios_init(uc)
            }
            "?widen@?$basic_ios@DU?$char_traits@D@std@@@std@@QBEDD@Z" => Self::basic_ios_widen(uc),
            "??1?$basic_ios@DU?$char_traits@D@std@@@std@@UAE@XZ" => Self::basic_ios_destructor(uc),
            "??4?$basic_ios@DU?$char_traits@D@std@@@std@@QAEAAV01@ABV01@@Z" => {
                Self::basic_ios_assign(uc)
            }

            "??0ios_base@std@@IAE@XZ" => Self::ios_base_ctor(uc),
            "??1ios_base@std@@UAE@XZ" => Self::ios_base_dtor(uc),
            "??4ios_base@std@@QAEAAV01@ABV01@@Z" => Self::ios_base_assign(uc),
            "?clear@ios_base@std@@QAEXH_N@Z" => Self::ios_base_clear(uc),
            "?copyfmt@ios_base@std@@QAEAAV12@ABV12@@Z" => Self::ios_base_copyfmt(uc),
            "?getloc@ios_base@std@@QBE?AVlocale@2@XZ" => Self::ios_base_getloc(uc),
            "?_Init@ios_base@std@@IAEXXZ" => Self::ios_base_init(uc),
            "??0Init@ios_base@std@@QAE@XZ" => Self::ios_base_init_ctor(uc),
            "??1Init@ios_base@std@@QAE@XZ" => Self::ios_base_init_dtor(uc),

            "?_Init@locale@std@@CAPAV_Locimp@12@XZ" => Self::locale_init(uc),
            "??0locale@std@@QAE@XZ" => Self::locale_ctor(uc),
            "??1locale@std@@QAE@XZ" => Self::locale_destructor(uc),
            "??4locale@std@@QAEAAV01@ABV01@@Z" => Self::locale_assign(uc),
            "?_Incref@facet@locale@std@@QAEXXZ" => Self::locale_facet_incref(uc),
            "?_Decref@facet@locale@std@@QAEPAV123@XZ" => Self::locale_facet_decref(uc),

            "?_Init@strstreambuf@std@@IAEXHPAD0H@Z" => Self::streambuf_init_strstream(uc),

            "??0_Winit@std@@QAE@XZ" => Self::winit_ctor(uc),
            "??1_Winit@std@@QAE@XZ" => Self::winit_dtor(uc),
            "??0_Lockit@std@@QAE@XZ" => Self::lockit_ctor(uc),
            "??1_Lockit@std@@QAE@XZ" => Self::lockit_dtor(uc),

            "??6std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@0@AAV10@PBD@Z" => {
                Self::ostream_insert_cstr(uc)
            }
            "??6std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@0@AAV10@D@Z" => {
                Self::ostream_insert_char(uc)
            }
            "?flush@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z" => {
                Self::ostream_flush(uc)
            }
            "?endl@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z" => {
                Self::ostream_endl(uc)
            }
            "?_Xlen@std@@YAXXZ" => Self::xlen(uc),
            "?_Xran@std@@YAXXZ" => Self::xran(uc),
            "?_Xoff@std@@YAXXZ" => Self::xoff(uc),
            "?__Fiopen@std@@YAPAU_iobuf@@PBDH@Z" => Self::fiopen(uc),

            "??0?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAE@XZ" => {
                let this_ptr = Self::this_ptr(uc);
                Self::init_basic_ios_layout(uc, this_ptr, BASIC_OSTREAM_VTABLE, 0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ofstream::basic_ofstream() -> (this={:#x})",
                    this_ptr,
                    this_ptr
                );
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
            "??_D?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                let this_ptr = Self::this_ptr(uc);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ofstream::~basic_ofstream()",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }
            "??_D?$basic_fstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                let this_ptr = Self::this_ptr(uc);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_fstream::~basic_fstream()",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }
            "??5?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV01@AAG@Z" => {
                Self::basic_istream_extract_unsigned_short(uc)
            }

            _ => {
                crate::emu_log!("[!] MSVCP60 Unhandled: {}", func_name);
                let this_ptr = Self::this_ptr(uc);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
        };

        if is_cdecl {
            result.map(|mut value| {
                value.cleanup = StackCleanup::Caller;
                value
            })
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::UnicornHelper;
    use std::fs;
    use unicorn_engine::{Arch, Mode};

    fn new_test_uc() -> Unicorn<'static, Win32Context> {
        let mut uc =
            Unicorn::new_with_data(Arch::X86, Mode::MODE_32, Win32Context::new(None)).unwrap();
        uc.setup(None, None).unwrap();
        uc
    }

    fn write_call_frame(uc: &mut Unicorn<Win32Context>, args: &[u32]) {
        let esp = uc.reg_read(RegisterX86::ESP).unwrap() as u32;
        uc.write_u32(esp as u64, 0xDEAD_BEEF);
        for (index, value) in args.iter().enumerate() {
            uc.write_u32(esp as u64 + 4 + (index as u64 * 4), *value);
        }
    }

    #[test]
    fn proxy_cache_key_prefixes_dll_name() {
        assert_eq!(
            MSVCP60::proxy_cache_key("?cout@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"),
            "MSVCP60.dll!?cout@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"
        );
    }

    #[test]
    fn cdecl_detection_matches_msvc_mangling() {
        assert!(MSVCP60::is_cdecl_symbol(
            "?flush@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z"
        ));
        assert!(!MSVCP60::is_cdecl_symbol(
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z"
        ));
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn resolve_export_reuses_cached_global_addresses() {
        let mut uc = new_test_uc();

        let vtable_a = MSVCP60::resolve_export(&mut uc, BASIC_OSTREAM_VTABLE).unwrap();
        let vtable_b = MSVCP60::resolve_export(&mut uc, BASIC_OSTREAM_VTABLE).unwrap();
        assert_eq!(vtable_a, vtable_b);

        let global_a =
            MSVCP60::resolve_export(&mut uc, "?_Global@_Locimp@locale@std@@0PAV123@A").unwrap();
        let global_b =
            MSVCP60::resolve_export(&mut uc, "?_Global@_Locimp@locale@std@@0PAV123@A").unwrap();
        assert_eq!(global_a, global_b);
        assert_ne!(uc.read_u32(global_a as u64), 0);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn basic_string_operations_update_guest_layout() {
        let mut uc = new_test_uc();
        let string_a = MSVCP60::alloc_zeroed(&mut uc, 32);
        let string_b = MSVCP60::alloc_zeroed(&mut uc, 32);
        let string_sub = MSVCP60::alloc_zeroed(&mut uc, 32);
        let hello = uc.alloc_str("abcd");
        let other = uc.alloc_str("xyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[0]);
        MSVCP60::handle(
            &mut uc,
            "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@ABV?$allocator@D@1@@Z",
        )
        .unwrap();

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[hello, 4]);
        MSVCP60::handle(
            &mut uc,
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z",
        )
        .unwrap();
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_a), b"abcd");

        uc.reg_write(RegisterX86::ECX, string_b as u64).unwrap();
        write_call_frame(&mut uc, &[other, 0]);
        MSVCP60::handle(
            &mut uc,
            "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@PBDABV?$allocator@D@1@@Z",
        )
        .unwrap();

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[string_b, 1, 2]);
        MSVCP60::handle(
            &mut uc,
            "?append@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z",
        )
        .unwrap();
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_a), b"abcdyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[1, 2, 3, b'Q' as u32]);
        MSVCP60::handle(
            &mut uc,
            "?replace@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@IIID@Z",
        )
        .unwrap();
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_a), b"aQQQdyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[string_sub, 2, 3]);
        let substr_result = MSVCP60::handle(
            &mut uc,
            "?substr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBE?AV12@II@Z",
        )
        .unwrap();
        assert_eq!(substr_result.cleanup, StackCleanup::Callee(3));
        assert_eq!(substr_result.return_value, Some(string_sub as i32));
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_sub), b"QQd");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let c_str_result = MSVCP60::handle(
            &mut uc,
            "?c_str@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEPBDXZ",
        )
        .unwrap();
        let c_str_ptr = c_str_result.return_value.unwrap() as u32;
        assert_eq!(uc.read_string(c_str_ptr as u64), "aQQQdyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let size_result = MSVCP60::handle(
            &mut uc,
            "?size@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEIXZ",
        )
        .unwrap();
        assert_eq!(size_result.return_value, Some(7));
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn cdecl_and_thiscall_cleanup_are_distinguished() {
        let mut uc = new_test_uc();
        let string_ptr = MSVCP60::alloc_zeroed(&mut uc, 32);

        uc.reg_write(RegisterX86::ECX, string_ptr as u64).unwrap();
        write_call_frame(&mut uc, &[0, 0]);
        let assign_result = MSVCP60::handle(
            &mut uc,
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z",
        )
        .unwrap();
        assert_eq!(assign_result.cleanup, StackCleanup::Callee(2));

        write_call_frame(&mut uc, &[0x1234]);
        let flush_result = MSVCP60::handle(
            &mut uc,
            "?flush@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z",
        )
        .unwrap();
        assert_eq!(flush_result.cleanup, StackCleanup::Caller);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn locale_singletons_stay_stable_across_init_and_refcounting() {
        let mut uc = new_test_uc();
        let global_ptr =
            MSVCP60::resolve_export(&mut uc, "?_Global@_Locimp@locale@std@@0PAV123@A").unwrap();
        let initial_locimp = uc.read_u32(global_ptr as u64);

        write_call_frame(&mut uc, &[]);
        let init_result =
            MSVCP60::handle(&mut uc, "?_Init@locale@std@@CAPAV_Locimp@12@XZ").unwrap();
        assert_eq!(init_result.cleanup, StackCleanup::Caller);
        assert_eq!(init_result.return_value, Some(initial_locimp as i32));

        let facet = MSVCP60::alloc_zeroed(&mut uc, 0x10);
        uc.write_u32(facet as u64 + FACET_REFCOUNT_OFFSET, 1);
        uc.reg_write(RegisterX86::ECX, facet as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        MSVCP60::handle(&mut uc, "?_Incref@facet@locale@std@@QAEXXZ").unwrap();
        assert_eq!(uc.read_u32(facet as u64 + FACET_REFCOUNT_OFFSET), 2);

        uc.reg_write(RegisterX86::ECX, facet as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let decref_result =
            MSVCP60::handle(&mut uc, "?_Decref@facet@locale@std@@QAEPAV123@XZ").unwrap();
        assert_eq!(uc.read_u32(facet as u64 + FACET_REFCOUNT_OFFSET), 1);
        assert_eq!(decref_result.return_value, Some(facet as i32));

        let global_again =
            MSVCP60::resolve_export(&mut uc, "?_Global@_Locimp@locale@std@@0PAV123@A").unwrap();
        assert_eq!(global_ptr, global_again);
        assert_eq!(uc.read_u32(global_again as u64), initial_locimp);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn locale_ctor_and_streambuf_init_fill_default_layout() {
        let mut uc = new_test_uc();
        let locale_obj = MSVCP60::alloc_zeroed(&mut uc, LOCALE_OBJECT_SIZE);
        let streambuf_obj = MSVCP60::alloc_zeroed(&mut uc, STREAMBUF_OBJECT_SIZE);

        uc.reg_write(RegisterX86::ECX, locale_obj as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let ctor_result = MSVCP60::handle(&mut uc, "??0locale@std@@QAE@XZ").unwrap();
        assert_eq!(ctor_result.cleanup, StackCleanup::Callee(0));
        assert_eq!(ctor_result.return_value, Some(locale_obj as i32));
        assert_ne!(MSVCP60::read_locale_impl(&uc, locale_obj), 0);
        assert_eq!(uc.read_u32(locale_obj as u64 + 0x1c), 0);

        uc.reg_write(RegisterX86::ECX, streambuf_obj as u64)
            .unwrap();
        write_call_frame(&mut uc, &[]);
        let init_result = MSVCP60::handle(
            &mut uc,
            "?_Init@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXXZ",
        )
        .unwrap();
        assert_eq!(init_result.cleanup, StackCleanup::Callee(0));
        assert_eq!(
            uc.read_u32(streambuf_obj as u64),
            MSVCP60::vtable_export_addr(&mut uc, BASIC_STREAMBUF_VTABLE)
        );
        assert_ne!(
            uc.read_u32(streambuf_obj as u64 + STREAMBUF_LOCALE_OFFSET),
            0
        );
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn filebuf_open_and_extract_unsigned_short_reads_host_file() {
        let mut uc = new_test_uc();
        let temp_path = std::env::temp_dir().join("emul4leaf_msvcp60_extract_u16.txt");
        fs::write(&temp_path, b" 4321 ").unwrap();

        let filebuf = MSVCP60::alloc_zeroed(&mut uc, STREAMBUF_OBJECT_SIZE);
        let istream = MSVCP60::alloc_zeroed(&mut uc, STREAM_OBJECT_SIZE);
        let out_value = MSVCP60::alloc_zeroed(&mut uc, 2);
        let path_ptr = uc.alloc_str(temp_path.to_string_lossy().as_ref());

        uc.reg_write(RegisterX86::ECX, filebuf as u64).unwrap();
        write_call_frame(&mut uc, &[0]);
        MSVCP60::handle(
            &mut uc,
            "??0?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAE@PAU_iobuf@@@Z",
        )
        .unwrap();

        uc.reg_write(RegisterX86::ECX, filebuf as u64).unwrap();
        write_call_frame(&mut uc, &[path_ptr, 1]);
        let open_result = MSVCP60::handle(
            &mut uc,
            "?open@?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAEPAV12@PBDH@Z",
        )
        .unwrap();
        assert_eq!(open_result.return_value, Some(filebuf as i32));

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[filebuf, 0]);
        MSVCP60::handle(
            &mut uc,
            "??0?$basic_istream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z",
        )
        .unwrap();

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[out_value]);
        let extract_result = MSVCP60::handle(
            &mut uc,
            "??5?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV01@AAG@Z",
        )
        .unwrap();
        assert_eq!(extract_result.return_value, Some(istream as i32));
        assert_eq!(uc.read_u16(out_value as u64), 4321);
        assert_eq!(uc.read_u32(istream as u64 + IOS_STATE_OFFSET) & 0x6, 0);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn extract_unsigned_short_uses_version_dat_fallback_for_empty_filebuf() {
        let mut uc = new_test_uc();
        let filebuf = MSVCP60::alloc_zeroed(&mut uc, STREAMBUF_OBJECT_SIZE);
        let istream = MSVCP60::alloc_zeroed(&mut uc, STREAM_OBJECT_SIZE);
        let out_value = MSVCP60::alloc_zeroed(&mut uc, 2);

        uc.reg_write(RegisterX86::ECX, filebuf as u64).unwrap();
        write_call_frame(&mut uc, &[0]);
        MSVCP60::handle(
            &mut uc,
            "??0?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAE@PAU_iobuf@@@Z",
        )
        .unwrap();

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[filebuf, 0]);
        MSVCP60::handle(
            &mut uc,
            "??0?$basic_istream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z",
        )
        .unwrap();

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[out_value]);
        let extract_result = MSVCP60::handle(
            &mut uc,
            "??5?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV01@AAG@Z",
        )
        .unwrap();
        assert_eq!(extract_result.return_value, Some(istream as i32));
        assert_eq!(uc.read_u16(out_value as u64), 54);
        assert_eq!(uc.read_u32(istream as u64 + IOS_STATE_OFFSET) & 0x6, 0);
        assert_ne!(MSVCP60::streambuf_file_handle(&uc, filebuf), 0);
    }
}
