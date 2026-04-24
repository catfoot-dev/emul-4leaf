mod advapi32;
mod comctl32;
mod gdi32;
mod imm32;
pub(crate) mod kernel32;
mod msvcp60;
mod msvcrt;
mod ole32;
mod shell32;
pub mod types;
pub(crate) mod user32;
mod winmm;
mod ws2_32;

pub(crate) use gdi32::GDI32;
pub use types::*;

use crate::{
    dll::{
        rare::{Rare, RareAudioEngine, RareContextState, RareSoundState},
        win32::{
            advapi32::ADVAPI32,
            comctl32::COMCTL32,
            gdi32::{BPP, aligned_stride},
            imm32::IMM32,
            kernel32::KERNEL32,
            msvcp60::MSVCP60,
            msvcrt::MSVCRT,
            ole32::Ole32,
            shell32::SHELL32,
            user32::USER32,
            winmm::WINMM,
            ws2_32::WS2_32,
        },
    },
    helper::{FAKE_IMPORT_BASE, HEAP_BASE, HEAP_SIZE},
    server::packet_logger::PacketLogger,
    ui::{UiCommand, win_event::WinEvent},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::Sender,
    },
    time::Instant,
};
use unicorn_engine::Unicorn;

/// Unicorn 엔진의 `User Data`에 적재되어, 모든 Win32 가상 OS 환경의 전역 상태를
/// 관리하고 유지하는 핵심 컨텍스트 블록입니다.
#[derive(Default)]
struct HeapBookkeeping {
    allocations: HashMap<u32, u32>,
    free_list: Vec<(u32, u32)>,
}

#[derive(Default)]
struct SurfaceBitmapSyncState {
    needs_full_upload: bool,
    ops: Vec<GpuSurfaceOp>,
    has_content: bool,
    active_dc_count: u32,
    needs_release_sync: bool,
}

fn pixels_look_presentable_as_initial_frame(pixels: &[u32]) -> bool {
    if pixels.is_empty() {
        return false;
    }

    let mut opaque_or_translucent = 0usize;
    let mut non_black_visible = 0usize;
    for &pixel in pixels {
        if pixel >> 24 == 0 {
            continue;
        }
        opaque_or_translucent += 1;
        if pixel & 0x00FF_FFFF != 0 {
            non_black_visible += 1;
        }
    }

    let total = pixels.len();
    let visible_percent = opaque_or_translucent.saturating_mul(100) / total;
    let non_black_percent = non_black_visible.saturating_mul(100) / total;

    // Lime은 창 생성 직후 검은 LBuffer나 컬러키가 제거된 테두리만 먼저 올린 뒤
    // 같은 surface에 실제 client buffer를 이어서 올립니다. 이 중간 프레임을 첫 표시
    // 대상으로 삼으면 검은 화면이나 구멍처럼 보이는 프레임이 노출됩니다.
    non_black_percent >= 12 || (visible_percent >= 85 && non_black_visible > 0)
}

pub struct Win32Context {
    /// 가상 힙(Heap) 메모리 할당을 위한 현재 포인터입니다. (단순 증가형)
    pub heap_cursor: AtomicU32,
    /// 가상 힙의 할당/해제 블록 메타데이터입니다.
    heap_blocks: Arc<Mutex<HeapBookkeeping>>,
    /// Fake API(Import) 주소 할당을 위한 카운터입니다.
    pub import_address: AtomicU32,

    /// 로드된 DLL 모듈들의 맵 (이름 ->LoadedDll)
    pub dll_modules: Arc<Mutex<HashMap<String, LoadedDll>>>,
    /// 가상 주소와 함수명 간의 역방향 매핑 (디버깅용)
    pub address_map: Arc<Mutex<HashMap<u64, String>>>,
    /// 프록시 DLL이 생성한 전역/데이터 export 주소 캐시
    pub proxy_exports: Arc<Mutex<HashMap<String, u32>>>,

    /// Win32 에러 코드 (GetLastError / SetLastError)
    pub last_error: Arc<AtomicU32>,
    /// 가상 핸들(HWND, HDC, SOCKET 등) 발급을 위한 전역 카운터
    pub handle_counter: Arc<AtomicU32>,
    /// 채널 기반 가상 소켓 맵
    pub tcp_sockets: Arc<Mutex<HashMap<u32, VirtualSocket>>>,
    /// 소켓의 논리적 상태를 추적하는 맵
    pub sockets: Arc<Mutex<HashMap<u32, SocketState>>>,
    /// 윈도우 이벤트 관리부 (UI와 신호 교환)
    pub win_event: Arc<Mutex<WinEvent>>,
    /// 스플래시 창 종료를 UI 스레드에 알리는 채널입니다.
    pub splash_close_tx: Option<Sender<()>>,
    /// 등록된 가상 윈도우 클래스 정보
    pub window_classes: Arc<Mutex<HashMap<String, WindowClass>>>,
    /// 가상 GDI 오브젝트 맵 (핸들 -> 오브젝트)
    pub gdi_objects: Arc<Mutex<HashMap<u32, GdiObject>>>,
    /// 호스트 창 표면에 직접 연결된 비트맵 핸들 집합입니다.
    surface_bitmaps: Arc<Mutex<HashSet<u32>>>,
    /// surface bitmap의 전체/부분 업로드 상태입니다.
    surface_bitmap_sync: Arc<Mutex<HashMap<u32, SurfaceBitmapSyncState>>>,
    /// 가상 동기화 이벤트 맵
    pub events: Arc<Mutex<HashMap<u32, EventState>>>,
    /// TLS(Thread Local Storage) 슬롯 데이터 (outer key = thread_id, inner = tls_index → value)
    pub tls_slots: Arc<Mutex<HashMap<u32, HashMap<u32, u32>>>>,
    /// TLS 슬롯 할당을 위한 카운터
    pub tls_counter: AtomicU32,
    /// 가상 레지스트리 데이터 (키 경로 -> 값 데이터)
    pub registry: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// 가상 레지스트리 핸들 맵 (HKEY -> 키 경로)
    pub registry_handles: Arc<Mutex<HashMap<u32, String>>>,
    /// 에뮬레이터 구동 시작 시간 (GetTickCount 등의 기준)
    pub start_time: Instant,
    /// rand() 함수를 위한 가상 시드 상태
    pub rand_state: AtomicU32,
    /// 패킷 로거 (Winsock 통신 분석용)
    pub packet_logger: Arc<Mutex<PacketLogger>>,
    /// 가상 파일 핸들 맵 (HFILE -> CRT 파일 상태)
    pub files: Arc<Mutex<HashMap<u32, FileState>>>,
    /// WSA 이벤트 핸들 맵 (event_handle → WsaEventEntry)
    /// WSAEventSelect / WSAEnumNetworkEvents / WaitForSingleObject 구현에 사용
    pub wsa_event_map: Arc<Mutex<HashMap<u32, WsaEventEntry>>>,

    /// 포커스를 가진 윈도우 핸들
    pub focus_hwnd: Arc<AtomicU32>,
    /// 현재 활성(Active) 상태인 윈도우 핸들
    pub active_hwnd: Arc<AtomicU32>,
    /// 최상위 전면(Foreground) 윈도우 핸들
    pub foreground_hwnd: Arc<AtomicU32>,
    /// 마우스 캡처를 보유한 윈도우 핸들
    pub capture_hwnd: Arc<AtomicU32>,
    /// 마우스 트래킹 상태 (_TrackMouseEvent 용)
    pub track_mouse_event: Arc<Mutex<Option<TrackMouseEventState>>>,

    /// 가상 스레드별 애플리케이션 메시지 큐입니다. 메인 스레드는 내부 키 `0`을 사용합니다.
    pub message_queues: Arc<Mutex<HashMap<u32, std::collections::VecDeque<[u32; 7]>>>>,
    /// 활성화된 타이머 맵 (ID -> Timer 객체)
    pub timers: Arc<Mutex<HashMap<u32, Timer>>>,
    /// 가상 키보드 키 상태 배열 (256키)
    pub key_states: Arc<Mutex<[bool; 256]>>,

    /// 가상 클립보드 데이터 버퍼
    pub clipboard_data: Arc<Mutex<Vec<u8>>>,
    /// 클립보드 열림 상태 (소유 핸들)
    pub clipboard_open: AtomicU32,
    /// 정적 시간(tm) 구조체를 위한 가상 주소
    pub tm_struct_ptr: AtomicU32,
    /// 데스크톱 창의 가상 핸들
    pub desktop_hwnd: AtomicU32,
    /// `SPI_GETWORKAREA` 등에 사용할 가상 작업 영역 좌표 `(left, top, right, bottom)`
    pub work_area: Arc<Mutex<(i32, i32, i32, i32)>>,
    /// 현재 표시되는 커서의 핸들
    pub current_cursor: AtomicU32,
    /// 현재 중첩된 wndproc 호출 스택의 `(hwnd, msg)` 목록
    pub cursor_dispatch_stack: Arc<Mutex<Vec<(u32, u32)>>>,
    /// 마우스 현재 X 좌표
    pub mouse_x: Arc<AtomicU32>,
    /// 마우스 현재 Y 좌표
    pub mouse_y: Arc<AtomicU32>,
    /// CRT 종료 핸들러 리스트
    pub onexit_handlers: Arc<Mutex<Vec<u32>>>,
    /// 에뮬레이션된 가상 스레드 목록
    pub threads: Arc<Mutex<Vec<EmulatedThread>>>,
    /// 현재 실행 중인 스레드 ID (0 = 메인 스레드)
    pub current_thread_idx: Arc<AtomicU32>,
    /// 메인 스레드가 외부 wake 없이도 즉시 실행 가능한 상태인지 여부
    pub main_ready: Arc<AtomicU32>,
    /// 메인 스레드(tid=0)용 재실행 대기 시각
    pub main_resume_time: Arc<Mutex<Option<Instant>>>,
    /// 메인 스레드의 재시도형 대기 API 타임아웃 시각
    pub main_wait_deadline: Arc<Mutex<Option<Instant>>>,
    /// 메인 스레드의 현재 대기 시작 시각
    pub main_wait_start_time: Arc<Mutex<Option<Instant>>>,
    /// 메인 스레드가 현재 대기 중인 커널 오브젝트 핸들 목록
    pub main_wait_handles: Arc<Mutex<Vec<u32>>>,
    /// 메인 스레드가 현재 대기 중인 소켓 목록
    pub main_wait_sockets: Arc<Mutex<Vec<u32>>>,
    /// 중첩 emu_start 호출 깊이 (코드 훅 내 재귀 제한용)
    pub emu_depth: Arc<AtomicU32>,
    /// 에뮬레이터 스레드 핸들 (park/unpark 기반 즉시 깨우기용)
    pub emu_thread: Arc<Mutex<Option<std::thread::Thread>>>,
    /// Rare.dll 프록시가 보유한 CPAL 오디오 엔진 상태
    pub(crate) rare_audio: Arc<Mutex<Option<RareAudioEngine>>>,
    /// Rare.dll 프록시가 생성한 컨텍스트 객체 맵
    pub(crate) rare_contexts: Arc<Mutex<HashMap<u32, RareContextState>>>,
    /// Rare.dll 프록시가 생성한 사운드 객체 맵
    pub(crate) rare_sounds: Arc<Mutex<HashMap<u32, RareSoundState>>>,
}

impl Win32Context {
    /// 새로운 Win32 환경 컨텍스트를 생성합니다.
    ///
    /// # 인자
    /// * `ui_tx`: UI로 명령을 보낼 송신 채널
    pub fn new(ui_tx: Option<Sender<UiCommand>>) -> Self {
        let ctx = Win32Context {
            heap_cursor: AtomicU32::new(HEAP_BASE as u32),
            heap_blocks: Arc::new(Mutex::new(HeapBookkeeping::default())),
            import_address: AtomicU32::new(FAKE_IMPORT_BASE as u32),
            dll_modules: Arc::new(Mutex::new(HashMap::new())),
            address_map: Arc::new(Mutex::new(HashMap::new())),
            proxy_exports: Arc::new(Mutex::new(HashMap::new())),
            last_error: Arc::new(AtomicU32::new(0)),
            handle_counter: Arc::new(AtomicU32::new(0x1000)),
            tcp_sockets: Arc::new(Mutex::new(HashMap::new())),
            sockets: Arc::new(Mutex::new(HashMap::new())),
            win_event: Arc::new(Mutex::new(WinEvent::new(ui_tx))),
            splash_close_tx: None,
            window_classes: Arc::new(Mutex::new(HashMap::new())),
            gdi_objects: Arc::new(Mutex::new(HashMap::new())),
            surface_bitmaps: Arc::new(Mutex::new(HashSet::new())),
            surface_bitmap_sync: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(Mutex::new(HashMap::new())),
            tls_slots: Arc::new(Mutex::new(HashMap::new())),
            tls_counter: AtomicU32::new(1),
            registry: Arc::new(Mutex::new(HashMap::new())),
            registry_handles: Arc::new(Mutex::new({
                let mut m = HashMap::new();
                m.insert(0x80000000, "HKEY_CLASSES_ROOT".to_string());
                m.insert(0x80000001, "HKEY_CURRENT_USER".to_string());
                m.insert(0x80000002, "HKEY_LOCAL_MACHINE".to_string());
                m.insert(0x80000003, "HKEY_USERS".to_string());
                m
            })),
            start_time: Instant::now(),
            rand_state: AtomicU32::new(12345),
            packet_logger: Arc::new(Mutex::new(PacketLogger::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
            wsa_event_map: Arc::new(Mutex::new(HashMap::new())),
            focus_hwnd: Arc::new(AtomicU32::new(0)),
            active_hwnd: Arc::new(AtomicU32::new(0)),
            foreground_hwnd: Arc::new(AtomicU32::new(0)),
            capture_hwnd: Arc::new(AtomicU32::new(0)),
            track_mouse_event: Arc::new(Mutex::new(None)),
            message_queues: Arc::new(Mutex::new(HashMap::from([(
                0,
                std::collections::VecDeque::new(),
            )]))),
            timers: Arc::new(Mutex::new(HashMap::new())),
            key_states: Arc::new(Mutex::new([false; 256])),
            clipboard_data: Arc::new(Mutex::new(Vec::new())),
            clipboard_open: AtomicU32::new(0),
            tm_struct_ptr: AtomicU32::new(0),
            desktop_hwnd: AtomicU32::new(0),
            work_area: Arc::new(Mutex::new((0, 0, 800, 600))),
            current_cursor: AtomicU32::new(0),
            cursor_dispatch_stack: Arc::new(Mutex::new(Vec::new())),
            mouse_x: Arc::new(AtomicU32::new(320)),
            mouse_y: Arc::new(AtomicU32::new(240)),
            onexit_handlers: Arc::new(Mutex::new(Vec::new())),
            threads: Arc::new(Mutex::new(Vec::new())),
            current_thread_idx: Arc::new(AtomicU32::new(0)),
            main_ready: Arc::new(AtomicU32::new(1)),
            main_resume_time: Arc::new(Mutex::new(None)),
            main_wait_deadline: Arc::new(Mutex::new(None)),
            main_wait_start_time: Arc::new(Mutex::new(None)),
            main_wait_handles: Arc::new(Mutex::new(Vec::new())),
            main_wait_sockets: Arc::new(Mutex::new(Vec::new())),
            emu_depth: Arc::new(AtomicU32::new(0)),
            emu_thread: Arc::new(Mutex::new(None)),
            rare_audio: Arc::new(Mutex::new(None)),
            rare_contexts: Arc::new(Mutex::new(HashMap::new())),
            rare_sounds: Arc::new(Mutex::new(HashMap::new())),
        };

        // 데스크톱 핸들 선행 할당
        let desktop_hwnd = ctx.alloc_handle();
        ctx.desktop_hwnd.store(desktop_hwnd, Ordering::SeqCst);
        ctx
    }

    /// 새로운 가상 핸들(u32)을 발급합니다.
    pub fn alloc_handle(&self) -> u32 {
        self.handle_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// 가상 힙에서 지정한 크기의 블록을 할당합니다.
    pub(crate) fn alloc_heap_block(&self, size: usize) -> Option<u32> {
        let aligned_size = align_heap_size(size)?;
        let mut heap_blocks = self.heap_blocks.lock().unwrap();

        if let Some((index, addr, block_size)) = heap_blocks
            .free_list
            .iter()
            .enumerate()
            .filter(|(_, (_, block_size))| *block_size >= aligned_size)
            .min_by_key(|(_, (_, block_size))| *block_size)
            .map(|(index, (addr, block_size))| (index, *addr, *block_size))
        {
            heap_blocks.free_list.swap_remove(index);
            if block_size > aligned_size {
                push_free_block(
                    &mut heap_blocks.free_list,
                    addr + aligned_size,
                    block_size - aligned_size,
                );
            }
            heap_blocks.allocations.insert(addr, aligned_size);
            return Some(addr);
        }

        let addr = self.heap_cursor.load(Ordering::SeqCst);
        let end = addr.checked_add(aligned_size)?;
        if (end as u64) > (HEAP_BASE + HEAP_SIZE) {
            return None;
        }

        self.heap_cursor.store(end, Ordering::SeqCst);
        heap_blocks.allocations.insert(addr, aligned_size);
        Some(addr)
    }

    /// 가상 힙 블록을 해제하여 재사용 가능 상태로 되돌립니다.
    pub(crate) fn free_heap_block(&self, addr: u32) -> bool {
        if addr < HEAP_BASE as u32 {
            crate::diagnostics::record_heap_free(addr, None, false);
            return false;
        }

        let mut heap_blocks = self.heap_blocks.lock().unwrap();
        let Some(size) = heap_blocks.allocations.remove(&addr) else {
            crate::diagnostics::record_heap_free(addr, None, false);
            return false;
        };

        push_free_block(&mut heap_blocks.free_list, addr, size);
        crate::diagnostics::record_heap_free(addr, Some(size), true);
        true
    }

    /// 가상 힙 블록의 현재 추적 크기를 반환합니다.
    pub(crate) fn heap_block_size(&self, addr: u32) -> Option<u32> {
        self.heap_blocks
            .lock()
            .unwrap()
            .allocations
            .get(&addr)
            .copied()
    }

    /// 지정한 메모리 범위를 완전히 포함하는 힙 할당 블록을 반환합니다.
    pub(crate) fn heap_allocation_for_range(&self, addr: u64, size: usize) -> Option<(u32, u32)> {
        let start = u32::try_from(addr).ok()?;
        let len = u32::try_from(size.max(1)).ok()?;
        let end = start.checked_add(len.saturating_sub(1))?;
        self.heap_blocks
            .lock()
            .unwrap()
            .allocations
            .iter()
            .find_map(|(&block_addr, &block_size)| {
                let block_end = block_addr.checked_add(block_size.saturating_sub(1))?;
                (start >= block_addr && end <= block_end).then_some((block_addr, block_size))
            })
    }

    /// 가상 데스크톱 작업 영역 좌표를 갱신합니다.
    pub(crate) fn set_work_area(&self, left: i32, top: i32, right: i32, bottom: i32) {
        *self.work_area.lock().unwrap() = (left, top, right, bottom);
    }

    /// 현재 가상 데스크톱 작업 영역 좌표를 반환합니다.
    pub(crate) fn work_area_rect(&self) -> (i32, i32, i32, i32) {
        *self.work_area.lock().unwrap()
    }

    /// 윈도우용 표면(Surface) 비트맵을 생성하고 GDI 오브젝트로 등록합니다.
    ///
    /// # 인자
    /// * `width`: 비트맵 너비
    /// * `height`: 비트맵 높이
    ///
    /// # 반환
    /// * `u32`: 생성된 비트맵의 가상 핸들
    pub fn create_surface_bitmap(&self, width: u32, height: u32) -> u32 {
        let hbmp = self.alloc_handle();
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let pixels_vec = vec![0u32; pixel_count];
        debug_assert_eq!(
            pixels_vec.len(),
            pixel_count,
            "surface bitmap pixel buffer length must equal width*height"
        );
        let pixels = Arc::new(Mutex::new(pixels_vec));
        self.gdi_objects.lock().unwrap().insert(
            hbmp,
            GdiObject::Bitmap {
                width,
                height,
                pixels,
                bits_addr: None,
                stride: aligned_stride(width, BPP as u16),
                bit_count: BPP as u16,
                top_down: false,
                palette: Vec::new(),
                red_mask: 0,
                green_mask: 0,
                blue_mask: 0,
                alpha_mask: 0,
            },
        );
        self.surface_bitmaps.lock().unwrap().insert(hbmp);
        self.surface_bitmap_sync.lock().unwrap().insert(
            hbmp,
            SurfaceBitmapSyncState {
                needs_full_upload: true,
                ops: Vec::new(),
                has_content: false,
                active_dc_count: 0,
                needs_release_sync: false,
            },
        );
        hbmp
    }

    /// 지정한 비트맵이 호스트 창 표면에 직접 연결된 bitmap인지 반환합니다.
    pub(crate) fn is_surface_bitmap(&self, hbmp: u32) -> bool {
        self.surface_bitmaps.lock().unwrap().contains(&hbmp)
    }

    /// 지정한 surface bitmap이 다음 렌더에서 전체 업로드되도록 표시합니다.
    pub(crate) fn mark_surface_bitmap_dirty(&self, hbmp: u32) {
        if !self.is_surface_bitmap(hbmp) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        state.needs_full_upload = true;
        state.needs_release_sync = false;
        state.ops.clear();
    }

    /// surface bitmap이 실제 GDI 출력으로 초기화되었음을 표시합니다.
    pub(crate) fn mark_surface_bitmap_has_content(&self, hbmp: u32) {
        if !self.is_surface_bitmap(hbmp) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        state.has_content = true;
    }

    /// surface bitmap의 최종 CPU 스냅샷을 DC 종료 시 한 번 더 동기화해야 함을 표시합니다.
    pub(crate) fn note_surface_bitmap_release_sync(&self, hbmp: u32) {
        if !self.is_surface_bitmap(hbmp) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        state.has_content = true;
        if !state.needs_full_upload {
            state.needs_release_sync = true;
        }
    }

    /// surface bitmap이 아직 빈 초기 버퍼인지 여부를 반환합니다.
    pub(crate) fn surface_bitmap_has_content(&self, hbmp: u32) -> bool {
        if !self.is_surface_bitmap(hbmp) {
            return false;
        }

        self.surface_bitmap_sync
            .lock()
            .unwrap()
            .get(&hbmp)
            .is_some_and(|state| state.has_content)
    }

    /// surface bitmap을 선택한 창 DC가 열렸음을 기록합니다.
    pub(crate) fn begin_surface_bitmap_dc(&self, hbmp: u32) {
        if !self.is_surface_bitmap(hbmp) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        state.active_dc_count = state.active_dc_count.saturating_add(1);
    }

    /// surface bitmap을 선택한 창 DC가 닫혔음을 기록합니다.
    pub(crate) fn end_surface_bitmap_dc(&self, hbmp: u32) {
        if !self.is_surface_bitmap(hbmp) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        state.active_dc_count = state.active_dc_count.saturating_sub(1);
    }

    /// surface bitmap을 선택한 창 DC가 아직 살아 있는지 반환합니다.
    pub(crate) fn surface_bitmap_dc_active(&self, hbmp: u32) -> bool {
        if !self.is_surface_bitmap(hbmp) {
            return false;
        }

        self.surface_bitmap_sync
            .lock()
            .unwrap()
            .get(&hbmp)
            .is_some_and(|state| state.active_dc_count > 0)
    }

    /// GDI DC를 제거하고 surface bitmap DC 생명주기 추적을 정리합니다.
    pub(crate) fn release_gdi_dc(&self, hdc: u32) -> bool {
        let removed = self.gdi_objects.lock().unwrap().remove(&hdc);
        let Some(GdiObject::Dc {
            selected_bitmap, ..
        }) = removed
        else {
            return false;
        };

        self.end_surface_bitmap_dc(selected_bitmap);
        true
    }

    /// surface bitmap의 전체 업로드 플래그를 소비하고 이전 값을 반환합니다.
    pub(crate) fn consume_surface_bitmap_full_upload(&self, hbmp: u32) -> bool {
        if !self.is_surface_bitmap(hbmp) {
            return false;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        let needs_full_upload = state.needs_full_upload;
        state.needs_full_upload = false;
        if needs_full_upload {
            state.ops.clear();
        }
        needs_full_upload
    }

    /// surface bitmap용 부분 업로드를 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_upload(&self, update: GpuBitmapUpdate) {
        if !self.is_surface_bitmap(update.surface_bitmap) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(update.surface_bitmap).or_default();
        if !state.has_content && !pixels_look_presentable_as_initial_frame(&update.pixels) {
            return;
        }
        if !state.has_content {
            state.has_content = true;
            state.needs_full_upload = true;
            state.needs_release_sync = false;
            state.ops.clear();
            return;
        }
        if state.needs_full_upload {
            return;
        }
        state.needs_release_sync = true;
        state.ops.push(GpuSurfaceOp::Upload(update));
    }

    /// surface bitmap용 GPU 직접 그리기 명령을 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_draw_command(&self, command: GpuDrawCommand) {
        let surface_bitmap = match &command {
            GpuDrawCommand::FillRect { surface_bitmap, .. } => *surface_bitmap,
            GpuDrawCommand::Line { surface_bitmap, .. } => *surface_bitmap,
            GpuDrawCommand::TextMask { surface_bitmap, .. } => *surface_bitmap,
            GpuDrawCommand::Blit { surface_bitmap, .. } => *surface_bitmap,
        };
        if !self.is_surface_bitmap(surface_bitmap) {
            return;
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(surface_bitmap).or_default();
        if !state.has_content {
            state.has_content = true;
            state.needs_full_upload = false;
        }
        if state.needs_full_upload {
            return;
        }
        state.needs_release_sync = true;
        state.ops.push(GpuSurfaceOp::Draw(command));
    }

    /// surface bitmap에 단색 채우기 직사각형 명령을 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_fill_rect(
        &self,
        surface_bitmap: u32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        color: u32,
    ) {
        self.queue_surface_bitmap_draw_command(GpuDrawCommand::FillRect {
            surface_bitmap,
            left,
            top,
            right,
            bottom,
            color,
        });
    }

    /// surface bitmap에 1픽셀 두께 선분 명령을 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_line(
        &self,
        surface_bitmap: u32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: u32,
    ) {
        self.queue_surface_bitmap_draw_command(GpuDrawCommand::Line {
            surface_bitmap,
            x1,
            y1,
            x2,
            y2,
            color,
        });
    }

    /// surface bitmap에 텍스트 알파 마스크 명령을 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_text_mask(
        &self,
        surface_bitmap: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        color: u32,
        alpha: Vec<u8>,
    ) {
        self.queue_surface_bitmap_draw_command(GpuDrawCommand::TextMask {
            surface_bitmap,
            x,
            y,
            width,
            height,
            color,
            alpha,
        });
    }

    /// surface bitmap에 GPU blit 명령을 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_blit(
        &self,
        surface_bitmap: u32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        src_width: u32,
        src_height: u32,
        uv: [f32; 4],
        pixels: Vec<u32>,
    ) {
        self.queue_surface_bitmap_draw_command(GpuDrawCommand::Blit {
            surface_bitmap,
            left,
            top,
            right,
            bottom,
            src_width,
            src_height,
            uv,
            pixels,
        });
    }

    /// 현재 픽셀 버퍼에서 지정한 직사각형만 잘라 surface bitmap 부분 업로드를 큐에 추가합니다.
    pub(crate) fn queue_surface_bitmap_rect_upload(
        &self,
        surface_bitmap: u32,
        pixels: &[u32],
        bitmap_width: u32,
        bitmap_height: u32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    ) {
        if !self.is_surface_bitmap(surface_bitmap) {
            return;
        }

        let clipped_left = left.max(0).min(bitmap_width as i32);
        let clipped_top = top.max(0).min(bitmap_height as i32);
        let clipped_right = right.max(0).min(bitmap_width as i32);
        let clipped_bottom = bottom.max(0).min(bitmap_height as i32);
        if clipped_left >= clipped_right || clipped_top >= clipped_bottom {
            return;
        }

        let width = (clipped_right - clipped_left) as u32;
        let height = (clipped_bottom - clipped_top) as u32;
        let mut rect_pixels = Vec::with_capacity(width.saturating_mul(height) as usize);

        for y in clipped_top..clipped_bottom {
            let row_start = y as usize * bitmap_width as usize + clipped_left as usize;
            let row_end = row_start + width as usize;
            rect_pixels.extend_from_slice(&pixels[row_start..row_end]);
        }

        self.queue_surface_bitmap_upload(GpuBitmapUpdate {
            surface_bitmap,
            x: clipped_left as u32,
            y: clipped_top as u32,
            width,
            height,
            pixels: rect_pixels,
        });
    }

    /// surface bitmap에 쌓인 부분 업로드 목록을 비웁니다.
    pub(crate) fn take_surface_bitmap_ops(&self, hbmp: u32) -> Vec<GpuSurfaceOp> {
        if !self.is_surface_bitmap(hbmp) {
            return Vec::new();
        }

        let mut sync = self.surface_bitmap_sync.lock().unwrap();
        let state = sync.entry(hbmp).or_default();
        std::mem::take(&mut state.ops)
    }

    #[cfg(test)]
    pub(crate) fn take_surface_bitmap_uploads(&self, hbmp: u32) -> Vec<GpuBitmapUpdate> {
        self.take_surface_bitmap_ops(hbmp)
            .into_iter()
            .filter_map(|op| match op {
                GpuSurfaceOp::Upload(update) => Some(update),
                GpuSurfaceOp::Draw(_) => None,
            })
            .collect()
    }

    /// surface bitmap에 쌓인 GPU 직접 그리기 명령 목록을 비웁니다.
    #[cfg(test)]
    pub(crate) fn take_surface_bitmap_draw_commands(&self, hbmp: u32) -> Vec<GpuDrawCommand> {
        self.take_surface_bitmap_ops(hbmp)
            .into_iter()
            .filter_map(|op| match op {
                GpuSurfaceOp::Draw(command) => Some(command),
                GpuSurfaceOp::Upload(_) => None,
            })
            .collect()
    }

    /// 지정한 HWND의 surface bitmap이 DC 종료 시 최종 CPU 스냅샷 동기화가 필요하면 전체 업로드를 예약합니다.
    pub(crate) fn ensure_window_surface_bitmap_sync(&self, hwnd: u32) {
        let surface_bitmap = self
            .win_event
            .lock()
            .unwrap()
            .windows
            .get(&hwnd)
            .map(|state| state.surface_bitmap)
            .unwrap_or(0);
        if surface_bitmap == 0 {
            return;
        }

        let needs_release_sync = {
            let mut sync = self.surface_bitmap_sync.lock().unwrap();
            let state = sync.entry(surface_bitmap).or_default();
            let needs_release_sync = state.needs_release_sync;
            state.needs_release_sync = false;
            needs_release_sync
        };

        if needs_release_sync {
            self.mark_surface_bitmap_dirty(surface_bitmap);
        }
    }

    /// surface bitmap 추적 정보를 제거합니다.
    pub(crate) fn forget_surface_bitmap(&self, hbmp: u32) {
        self.surface_bitmaps.lock().unwrap().remove(&hbmp);
        self.surface_bitmap_sync.lock().unwrap().remove(&hbmp);
    }

    /// 게스트에 노출되는 스레드 ID를 내부 메시지 큐 키로 정규화합니다.
    ///
    /// 메인 스레드는 guest 입장에서는 `1`로 보이지만, 내부 스케줄러에서는 `0`으로 유지합니다.
    pub(crate) fn normalize_queue_thread_id(&self, thread_id: u32) -> u32 {
        if thread_id == 1 { 0 } else { thread_id }
    }

    /// 현재 실행 중인 가상 스레드의 내부 메시지 큐 키를 반환합니다.
    pub(crate) fn current_queue_thread_id(&self) -> u32 {
        self.current_thread_idx.load(Ordering::SeqCst)
    }

    /// 지정된 HWND를 소유한 가상 스레드의 내부 메시지 큐 키를 반환합니다.
    pub(crate) fn window_owner_thread_id(&self, hwnd: u32) -> u32 {
        self.win_event
            .lock()
            .unwrap()
            .windows
            .get(&hwnd)
            .map(|state| state.owner_thread_id)
            .unwrap_or(0)
    }

    /// 지정된 가상 스레드의 메시지 큐를 가변 참조로 노출합니다.
    pub(crate) fn with_thread_message_queue<T, F>(&self, thread_id: u32, f: F) -> T
    where
        F: FnOnce(&mut std::collections::VecDeque<[u32; 7]>) -> T,
    {
        let thread_id = self.normalize_queue_thread_id(thread_id);
        let mut queues = self.message_queues.lock().unwrap();
        let queue = queues.entry(thread_id).or_default();
        f(queue)
    }

    /// 지정된 가상 스레드 큐에 메시지를 추가하고 실제 사용된 큐 키를 반환합니다.
    pub(crate) fn queue_message_for_thread(&self, thread_id: u32, message: [u32; 7]) -> u32 {
        let thread_id = self.normalize_queue_thread_id(thread_id);
        self.with_thread_message_queue(thread_id, |queue| queue.push_back(message));
        thread_id
    }

    /// 지정된 HWND를 소유한 가상 스레드 큐에 메시지를 추가하고 큐 키를 반환합니다.
    pub(crate) fn queue_message_for_window(&self, hwnd: u32, message: [u32; 7]) -> u32 {
        let thread_id = if hwnd == 0 {
            self.current_queue_thread_id()
        } else {
            self.window_owner_thread_id(hwnd)
        };
        self.queue_message_for_thread(thread_id, message)
    }

    /// 지정된 가상 스레드의 재시도형 대기 상태를 즉시 해제합니다.
    pub(crate) fn wake_thread_message_wait(&self, thread_id: u32) -> u32 {
        let thread_id = self.normalize_queue_thread_id(thread_id);
        if thread_id == 0 {
            self.main_ready.store(1, Ordering::SeqCst);
            return thread_id;
        }

        let mut threads = self.threads.lock().unwrap();
        if let Some(thread) = threads
            .iter_mut()
            .find(|thread| thread.thread_id == thread_id)
        {
            thread.ready = true;
        }
        thread_id
    }

    /// UI 스레드에 메시지 박스 표시를 요청하고 응답을 기다립니다.
    pub(crate) fn message_box(
        &self,
        owner_hwnd: u32,
        caption: String,
        text: String,
        u_type: u32,
    ) -> i32 {
        let (tx, rx) = std::sync::mpsc::channel();
        self.win_event
            .lock()
            .unwrap()
            .send_ui_command(UiCommand::MessageBox {
                owner_hwnd,
                caption,
                text,
                u_type,
                response_tx: tx,
            });

        rx.recv().unwrap_or(1)
    }

    /// 에뮬레이터 호스트 스레드를 즉시 깨웁니다.
    pub(crate) fn unpark_emulator_thread(&self) {
        if let Ok(guard) = self.emu_thread.lock()
            && let Some(thread) = guard.as_ref()
        {
            thread.unpark();
        }
    }

    /// 윈도우 상태의 현재 크기에 맞춰 연결된 표면 비트맵 저장소를 동기화합니다.
    pub(crate) fn sync_window_surface_bitmap(&self, hwnd: u32) {
        let Some((surface_bitmap, target_width, target_height)) = ({
            let win_event = self.win_event.lock().unwrap();
            win_event.windows.get(&hwnd).and_then(|state| {
                let width = u32::try_from(state.width).ok()?;
                let height = u32::try_from(state.height).ok()?;
                Some((state.surface_bitmap, width, height))
            })
        }) else {
            return;
        };

        let mut gdi_objects = self.gdi_objects.lock().unwrap();
        let Some(GdiObject::Bitmap {
            width,
            height,
            pixels,
            stride,
            ..
        }) = gdi_objects.get_mut(&surface_bitmap)
        else {
            return;
        };

        if *width == target_width && *height == target_height {
            return;
        }

        let old_width = *width as usize;
        let old_height = *height as usize;
        let new_width = target_width as usize;
        let new_height = target_height as usize;
        let copy_width = old_width.min(new_width);
        let copy_height = old_height.min(new_height);

        let mut pixels_guard = pixels.lock().unwrap();
        let mut resized_pixels = vec![0u32; new_width.saturating_mul(new_height)];

        // 기존 프레임의 겹치는 영역만 보존해 리사이즈 직후에도 화면이 깨지지 않게 합니다.
        for row in 0..copy_height {
            let src_row = row * old_width;
            let dst_row = row * new_width;
            resized_pixels[dst_row..dst_row + copy_width]
                .copy_from_slice(&pixels_guard[src_row..src_row + copy_width]);
        }

        debug_assert_eq!(
            resized_pixels.len(),
            (new_width).saturating_mul(new_height),
            "resized surface bitmap buffer length must equal new_width*new_height"
        );
        *pixels_guard = resized_pixels;
        *width = target_width;
        *height = target_height;
        *stride = aligned_stride(target_width, BPP as u16);
        self.mark_surface_bitmap_dirty(surface_bitmap);
    }

    /// DLL 이름과 함수 이름을 기반으로 적절한 Win32 API 핸들러로 분기합니다.
    ///
    /// # 인자
    /// * `uc`: Unicorn 엔진 인스턴스 (Win32Context 포함)
    /// * `dll_name`: 호출된 DLL의 이름
    /// * `func_name`: 호출된 함수의 이름
    ///
    /// # 반환
    /// * `Option<ApiHookResult>`: 핸들러 실행 결과 (성공 시 Some, 정의되지 않은 경우 None)
    pub fn handle(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str,
    ) -> Option<ApiHookResult> {
        match dll_name {
            "ADVAPI32.dll" => ADVAPI32::handle(uc, func_name),
            "COMCTL32.dll" => COMCTL32::handle(uc, func_name),
            "GDI32.dll" => GDI32::handle(uc, func_name),
            "IMM32.dll" => IMM32::handle(uc, func_name),
            "KERNEL32.dll" => KERNEL32::handle(uc, func_name),
            "MSVCP60.dll" => MSVCP60::handle(uc, func_name),
            "MSVCRT.dll" => MSVCRT::handle(uc, func_name),
            "Rare.dll" => Rare::handle(uc, func_name),
            "ole32.dll" => Ole32::handle(uc, func_name),
            "SHELL32.dll" => SHELL32::handle(uc, func_name),
            "USER32.dll" => USER32::handle(uc, func_name),
            "WINMM.dll" => WINMM::handle(uc, func_name),
            "WS2_32.dll" => WS2_32::handle(uc, func_name),
            _ => {
                crate::emu_log!("[!] Undefined DLL: {}", dll_name);
                None
            }
        }
    }

    /// DLL의 임포트 해소(Resolve) 과정에서 프록시 DLL이 특수하게 관리하는 데이터 주소 등이 있는지 확인합니다.
    pub fn resolve_proxy_export(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str,
    ) -> Option<u32> {
        match dll_name {
            "MSVCRT.dll" => MSVCRT::resolve_export(uc, func_name),
            "MSVCP60.dll" => MSVCP60::resolve_export(uc, func_name),
            "Rare.dll" => Rare::resolve_export(uc, func_name),
            _ => None,
        }
    }
}

impl Clone for Win32Context {
    fn clone(&self) -> Self {
        Self {
            heap_cursor: AtomicU32::new(self.heap_cursor.load(Ordering::SeqCst)),
            heap_blocks: self.heap_blocks.clone(),
            import_address: AtomicU32::new(self.import_address.load(Ordering::SeqCst)),
            dll_modules: self.dll_modules.clone(),
            address_map: self.address_map.clone(),
            proxy_exports: self.proxy_exports.clone(),
            last_error: self.last_error.clone(),
            handle_counter: self.handle_counter.clone(),
            tcp_sockets: self.tcp_sockets.clone(),
            sockets: self.sockets.clone(),
            win_event: self.win_event.clone(),
            splash_close_tx: self.splash_close_tx.clone(),
            window_classes: self.window_classes.clone(),
            gdi_objects: self.gdi_objects.clone(),
            surface_bitmaps: self.surface_bitmaps.clone(),
            surface_bitmap_sync: self.surface_bitmap_sync.clone(),
            events: self.events.clone(),
            tls_slots: self.tls_slots.clone(),
            tls_counter: AtomicU32::new(self.tls_counter.load(Ordering::SeqCst)),
            registry: self.registry.clone(),
            registry_handles: self.registry_handles.clone(),
            start_time: self.start_time,
            rand_state: AtomicU32::new(self.rand_state.load(Ordering::SeqCst)),
            packet_logger: self.packet_logger.clone(),
            files: self.files.clone(),
            wsa_event_map: self.wsa_event_map.clone(),
            focus_hwnd: self.focus_hwnd.clone(),
            active_hwnd: self.active_hwnd.clone(),
            foreground_hwnd: self.foreground_hwnd.clone(),
            capture_hwnd: self.capture_hwnd.clone(),
            track_mouse_event: self.track_mouse_event.clone(),
            message_queues: self.message_queues.clone(),
            timers: self.timers.clone(),
            key_states: self.key_states.clone(),
            clipboard_data: self.clipboard_data.clone(),
            clipboard_open: AtomicU32::new(self.clipboard_open.load(Ordering::SeqCst)),
            tm_struct_ptr: AtomicU32::new(self.tm_struct_ptr.load(Ordering::SeqCst)),
            desktop_hwnd: AtomicU32::new(self.desktop_hwnd.load(Ordering::SeqCst)),
            work_area: self.work_area.clone(),
            current_cursor: AtomicU32::new(self.current_cursor.load(Ordering::SeqCst)),
            cursor_dispatch_stack: self.cursor_dispatch_stack.clone(),
            mouse_x: self.mouse_x.clone(),
            mouse_y: self.mouse_y.clone(),
            onexit_handlers: self.onexit_handlers.clone(),
            threads: self.threads.clone(),
            current_thread_idx: self.current_thread_idx.clone(),
            main_ready: self.main_ready.clone(),
            main_resume_time: self.main_resume_time.clone(),
            main_wait_deadline: self.main_wait_deadline.clone(),
            main_wait_start_time: self.main_wait_start_time.clone(),
            main_wait_handles: self.main_wait_handles.clone(),
            main_wait_sockets: self.main_wait_sockets.clone(),
            emu_depth: self.emu_depth.clone(),
            emu_thread: self.emu_thread.clone(),
            rare_audio: self.rare_audio.clone(),
            rare_contexts: self.rare_contexts.clone(),
            rare_sounds: self.rare_sounds.clone(),
        }
    }
}

fn align_heap_size(size: usize) -> Option<u32> {
    let base = u32::try_from(size.max(1)).ok()?;
    base.checked_add(3).map(|value| value & !3)
}

fn push_free_block(free_list: &mut Vec<(u32, u32)>, addr: u32, size: u32) {
    if size == 0 {
        return;
    }

    free_list.push((addr, size));
    free_list.sort_unstable_by_key(|(block_addr, _)| *block_addr);

    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(free_list.len());
    for (block_addr, block_size) in free_list.drain(..) {
        if let Some((prev_addr, prev_size)) = merged.last_mut() {
            let prev_end = prev_addr.saturating_add(*prev_size);
            if prev_end >= block_addr {
                let block_end = block_addr.saturating_add(block_size);
                *prev_size = block_end.max(prev_end).saturating_sub(*prev_addr);
                continue;
            }
        }
        merged.push((block_addr, block_size));
    }
    *free_list = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_block_is_reused_after_free() {
        let ctx = Win32Context::new(None);

        let first = ctx.alloc_heap_block(16).unwrap();
        let second = ctx.alloc_heap_block(32).unwrap();

        assert_eq!(first, HEAP_BASE as u32);
        assert_eq!(second, first + 16);

        assert!(ctx.free_heap_block(first));
        let reused = ctx.alloc_heap_block(8).unwrap();

        assert_eq!(reused, first);
    }

    #[test]
    fn adjacent_free_blocks_are_merged() {
        let ctx = Win32Context::new(None);

        let first = ctx.alloc_heap_block(16).unwrap();
        let second = ctx.alloc_heap_block(16).unwrap();

        assert!(ctx.free_heap_block(first));
        assert!(ctx.free_heap_block(second));

        let merged = ctx.alloc_heap_block(24).unwrap();
        assert_eq!(merged, first);
    }

    #[test]
    fn heap_allocation_range_detects_untracked_writes() {
        let ctx = Win32Context::new(None);
        let first = ctx.alloc_heap_block(16).unwrap();

        assert_eq!(
            ctx.heap_allocation_for_range(first as u64 + 4, 4),
            Some((first, 16))
        );
        assert_eq!(ctx.heap_allocation_for_range(first as u64 + 15, 2), None);
    }

    #[test]
    fn surface_bitmap_upload_queue_tracks_full_and_partial_updates() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(4, 4);

        assert!(!ctx.surface_bitmap_has_content(hbmp));
        assert!(ctx.consume_surface_bitmap_full_upload(hbmp));

        ctx.queue_surface_bitmap_upload(GpuBitmapUpdate {
            surface_bitmap: hbmp,
            x: 1,
            y: 1,
            width: 2,
            height: 2,
            pixels: vec![0xFFFF_FFFF; 4],
        });

        assert!(ctx.surface_bitmap_has_content(hbmp));
        assert!(ctx.take_surface_bitmap_uploads(hbmp).is_empty());
        assert!(ctx.consume_surface_bitmap_full_upload(hbmp));

        ctx.queue_surface_bitmap_upload(GpuBitmapUpdate {
            surface_bitmap: hbmp,
            x: 1,
            y: 1,
            width: 2,
            height: 2,
            pixels: vec![0xFFFF_FFFF; 4],
        });
        let updates = ctx.take_surface_bitmap_uploads(hbmp);
        assert_eq!(updates.len(), 1);
        assert!(!ctx.consume_surface_bitmap_full_upload(hbmp));

        ctx.mark_surface_bitmap_dirty(hbmp);
        ctx.queue_surface_bitmap_upload(GpuBitmapUpdate {
            surface_bitmap: hbmp,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            pixels: vec![0xFFFF_FFFF],
        });

        assert!(ctx.consume_surface_bitmap_full_upload(hbmp));
        assert!(ctx.take_surface_bitmap_uploads(hbmp).is_empty());
        assert!(ctx.surface_bitmap_has_content(hbmp));
    }

    // #[test]
    // fn surface_bitmap_ignores_initial_black_upload() {
    //     let ctx = Win32Context::new(None);
    //     let hbmp = ctx.create_surface_bitmap(2, 2);
    //     let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

    //     ctx.queue_surface_bitmap_upload(GpuBitmapUpdate {
    //         surface_bitmap: hbmp,
    //         x: 0,
    //         y: 0,
    //         width: 2,
    //         height: 2,
    //         pixels: vec![0xFF00_0000; 4],
    //     });

    //     assert!(!ctx.surface_bitmap_has_content(hbmp));
    //     assert!(ctx.take_surface_bitmap_uploads(hbmp).is_empty());
    // }

    // #[test]
    // fn surface_bitmap_ignores_initial_border_only_upload() {
    //     let ctx = Win32Context::new(None);
    //     let hbmp = ctx.create_surface_bitmap(10, 10);
    //     let _ = ctx.consume_surface_bitmap_full_upload(hbmp);
    //     let mut pixels = vec![0; 100];
    //     pixels[0] = 0xFFFF_FFFF;
    //     pixels[1] = 0xFF00_0000;
    //     pixels[2] = 0xFF6E_81D0;

    //     ctx.queue_surface_bitmap_upload(GpuBitmapUpdate {
    //         surface_bitmap: hbmp,
    //         x: 0,
    //         y: 0,
    //         width: 10,
    //         height: 10,
    //         pixels,
    //     });

    //     assert!(!ctx.surface_bitmap_has_content(hbmp));
    //     assert!(ctx.take_surface_bitmap_uploads(hbmp).is_empty());
    // }

    // #[test]
    // fn surface_bitmap_accepts_first_meaningful_upload() {
    //     let ctx = Win32Context::new(None);
    //     let hbmp = ctx.create_surface_bitmap(10, 10);
    //     let _ = ctx.consume_surface_bitmap_full_upload(hbmp);
    //     let mut pixels = vec![0xFF00_0000; 100];
    //     pixels[..12].fill(0xFFFF_FFFF);

    //     ctx.queue_surface_bitmap_upload(GpuBitmapUpdate {
    //         surface_bitmap: hbmp,
    //         x: 0,
    //         y: 0,
    //         width: 10,
    //         height: 10,
    //         pixels,
    //     });

    //     assert!(ctx.surface_bitmap_has_content(hbmp));
    //     assert_eq!(ctx.take_surface_bitmap_uploads(hbmp).len(), 1);
    // }

    #[test]
    fn surface_bitmap_dc_lifetime_tracks_active_state() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(4, 4);

        assert!(!ctx.surface_bitmap_dc_active(hbmp));

        ctx.begin_surface_bitmap_dc(hbmp);
        assert!(ctx.surface_bitmap_dc_active(hbmp));

        ctx.end_surface_bitmap_dc(hbmp);
        assert!(!ctx.surface_bitmap_dc_active(hbmp));

        ctx.end_surface_bitmap_dc(hbmp);
        assert!(!ctx.surface_bitmap_dc_active(hbmp));
    }

    #[test]
    fn surface_bitmap_draw_command_marks_content_ready() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(4, 4);
        let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

        ctx.queue_surface_bitmap_line(hbmp, 0, 1, 3, 2, 0xFF11_2233);

        assert!(ctx.surface_bitmap_has_content(hbmp));
    }

    #[test]
    fn surface_bitmap_release_sync_flag_is_cleared_by_full_upload() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(4, 4);
        let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

        ctx.note_surface_bitmap_release_sync(hbmp);
        {
            let sync = ctx.surface_bitmap_sync.lock().unwrap();
            let state = sync.get(&hbmp).unwrap();
            assert!(state.needs_release_sync);
        }

        ctx.mark_surface_bitmap_dirty(hbmp);
        {
            let sync = ctx.surface_bitmap_sync.lock().unwrap();
            let state = sync.get(&hbmp).unwrap();
            assert!(!state.needs_release_sync);
            assert!(state.needs_full_upload);
        }
    }

    #[test]
    fn surface_bitmap_rect_upload_is_clipped_to_bitmap_bounds() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(2, 2);
        let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

        ctx.mark_surface_bitmap_has_content(hbmp);
        ctx.queue_surface_bitmap_rect_upload(
            hbmp,
            &[0xFF00_0001, 0xFF00_0002, 0xFF00_0003, 0xFF00_0004],
            2,
            2,
            -1,
            -1,
            2,
            1,
        );

        let updates = ctx.take_surface_bitmap_uploads(hbmp);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].x, 0);
        assert_eq!(updates[0].y, 0);
        assert_eq!(updates[0].width, 2);
        assert_eq!(updates[0].height, 1);
        assert_eq!(updates[0].pixels, vec![0xFF00_0001, 0xFF00_0002]);
    }

    #[test]
    fn surface_bitmap_line_command_is_queued() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(4, 4);
        let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

        ctx.queue_surface_bitmap_line(hbmp, 0, 1, 3, 2, 0xFF11_2233);

        let commands = ctx.take_surface_bitmap_draw_commands(hbmp);
        assert_eq!(
            commands,
            vec![GpuDrawCommand::Line {
                surface_bitmap: hbmp,
                x1: 0,
                y1: 1,
                x2: 3,
                y2: 2,
                color: 0xFF11_2233,
            }]
        );
    }

    #[test]
    fn surface_bitmap_text_mask_command_is_queued() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(4, 4);
        let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

        ctx.queue_surface_bitmap_text_mask(hbmp, 1, 2, 3, 4, 0xFFAA_BBCC, vec![1, 2, 3]);

        let commands = ctx.take_surface_bitmap_draw_commands(hbmp);
        assert_eq!(
            commands,
            vec![GpuDrawCommand::TextMask {
                surface_bitmap: hbmp,
                x: 1,
                y: 2,
                width: 3,
                height: 4,
                color: 0xFFAA_BBCC,
                alpha: vec![1, 2, 3],
            }]
        );
    }

    #[test]
    fn surface_bitmap_blit_command_is_queued() {
        let ctx = Win32Context::new(None);
        let hbmp = ctx.create_surface_bitmap(8, 8);
        let _ = ctx.consume_surface_bitmap_full_upload(hbmp);

        ctx.queue_surface_bitmap_blit(
            hbmp,
            1,
            2,
            5,
            6,
            4,
            4,
            [0.0, 0.0, 1.0, 1.0],
            vec![0xFFFF_FFFF; 16],
        );

        let commands = ctx.take_surface_bitmap_draw_commands(hbmp);
        assert_eq!(
            commands,
            vec![GpuDrawCommand::Blit {
                surface_bitmap: hbmp,
                left: 1,
                top: 2,
                right: 5,
                bottom: 6,
                src_width: 4,
                src_height: 4,
                uv: [0.0, 0.0, 1.0, 1.0],
                pixels: vec![0xFFFF_FFFF; 16],
            }]
        );
    }

    #[test]
    fn message_box_releases_win_event_lock_while_waiting() {
        let (ui_tx, ui_rx) = std::sync::mpsc::channel();
        let ctx = Win32Context::new(Some(ui_tx));
        let ctx_for_ui = ctx.clone();

        let responder = std::thread::spawn(move || {
            let command = ui_rx.recv().expect("message box command");
            match command {
                UiCommand::MessageBox {
                    owner_hwnd,
                    caption,
                    text,
                    u_type,
                    response_tx,
                } => {
                    assert_eq!(owner_hwnd, 0x1000);
                    assert_eq!(caption, "caption");
                    assert_eq!(text, "text");
                    assert_eq!(u_type, 1);
                    assert!(ctx_for_ui.win_event.try_lock().is_ok());
                    response_tx.send(2).expect("message box response");
                }
                _ => panic!("unexpected UI command"),
            }
        });

        let result = ctx.message_box(0x1000, "caption".into(), "text".into(), 1);
        assert_eq!(result, 2);
        responder.join().expect("responder thread");
    }
}
