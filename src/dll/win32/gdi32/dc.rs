use crate::{
    dll::win32::{ApiHookResult, GdiObject, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

// API: HDC CreateCompatibleDC(HDC hdc)
// 역할: 지정된 디바이스와 호환되는 메모리 디바이스 컨텍스트(DC)를 만듦
pub(super) fn create_compatible_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let ctx = uc.get_data();
    let new_hdc = ctx.alloc_handle();

    // 참조 DC가 있으면 해당 크기를 상속, 없으면 기본값 사용
    let (width, height) = {
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc { width, height, .. }) = gdi_objects.get(&hdc) {
            (*width, *height)
        } else {
            (640, 480)
        }
    };

    // Win32 memory DC는 기본적으로 1x1 monochrome bitmap이 선택된 상태로 시작합니다.
    let default_bitmap = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        default_bitmap,
        GdiObject::Bitmap {
            width: 1,
            height: 1,
            pixels: std::sync::Arc::new(std::sync::Mutex::new(vec![0u32; 1])),
            bits_addr: None,
            stride: 4,
            bit_count: 1,
            top_down: false,
            palette: vec![0x000000, 0x00FF_FFFF],
            red_mask: 0,
            green_mask: 0,
            blue_mask: 0,
            alpha_mask: 0,
        },
    );

    ctx.gdi_objects.lock().unwrap().insert(
        new_hdc,
        GdiObject::Dc {
            associated_window: 0,
            width,
            height,
            origin_x: 0,
            origin_y: 0,
            selected_bitmap: default_bitmap,
            selected_font: 0,
            selected_brush: 0,
            selected_pen: 0,
            selected_region: 0,
            selected_palette: 0,
            bk_mode: 1, // TRANSPARENT(1) or OPAQUE(2)
            bk_color: 0xFFFF_FFFF,
            text_color: 0xFF00_0000,
            rop2_mode: 13, // R2_COPYPEN
            current_x: 0,
            current_y: 0,
        },
    );
    crate::emu_log!(
        "[GDI32] CreateCompatibleDC({:#x}) -> HDC {:#x}",
        hdc,
        new_hdc
    );
    Some(ApiHookResult::callee(1, Some(new_hdc as i32)))
}

// API: BOOL DeleteDC(HDC hdc)
// 역할: 지정된 디바이스 컨텍스트(DC)를 삭제
pub(super) fn delete_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    uc.get_data().release_gdi_dc(hdc);
    crate::emu_log!("[GDI32] DeleteDC({:#x}) -> BOOL 1", hdc);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: HGDIOBJ SelectObject(HDC hdc, HGDIOBJ h)
// 역할: 지정된 DC로 객체를 선택하여 기존의 동일한 유형의 객체를 바꿈
pub(super) fn select_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let hobj = uc.read_arg(1);
    let ctx = uc.get_data();
    let obj_clone = {
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        gdi_objects.get(&hobj).cloned()
    };
    let selected_window = if matches!(obj_clone, Some(GdiObject::Bitmap { .. })) {
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .iter()
            .find_map(|(&hwnd, state)| (state.surface_bitmap == hobj).then_some(hwnd))
    } else {
        None
    };
    let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
    let mut old_hobj = 0;
    if let Some(GdiObject::Dc {
        associated_window,
        width,
        height,
        selected_bitmap,
        selected_font,
        selected_brush,
        selected_pen,
        selected_region,
        selected_palette,
        ..
    }) = gdi_objects.get_mut(&hdc)
    {
        match obj_clone.clone() {
            Some(GdiObject::Bitmap {
                width: bmp_width,
                height: bmp_height,
                ..
            }) => {
                old_hobj = *selected_bitmap;
                *selected_bitmap = hobj;
                *associated_window = selected_window.unwrap_or(0);
                *width = bmp_width as i32;
                *height = bmp_height as i32;
            }
            Some(GdiObject::Font { .. }) => {
                old_hobj = *selected_font;
                *selected_font = hobj;
            }
            Some(GdiObject::Brush { .. }) => {
                old_hobj = *selected_brush;
                *selected_brush = hobj;
            }
            Some(GdiObject::Pen { .. }) => {
                old_hobj = *selected_pen;
                *selected_pen = hobj;
            }
            Some(GdiObject::Region { .. }) => {
                old_hobj = *selected_region;
                *selected_region = hobj;
            }
            Some(GdiObject::Palette { .. }) => {
                old_hobj = *selected_palette;
                *selected_palette = hobj;
            }
            Some(GdiObject::StockObject(id)) => {
                if id < 5 || id == 18 {
                    old_hobj = *selected_brush;
                    *selected_brush = hobj;
                } else if (5..=8).contains(&id) {
                    old_hobj = *selected_pen;
                    *selected_pen = hobj;
                } else if (10..=17).contains(&id) {
                    old_hobj = *selected_font;
                    *selected_font = hobj;
                }
            }
            None => { /* 알 수 없는 객체이거나 이미 삭제된 경우 */ }
            _ => { /* Dc 객체는 선택할 수 없음 */ }
        }
    }
    crate::emu_log!(
        "[GDI32] SelectObject({:#x}, {:#x}) -> HGDIOBJ {:#x}",
        hdc,
        hobj,
        old_hobj
    );
    Some(ApiHookResult::callee(2, Some(old_hobj as i32)))
}

// API: BOOL DeleteObject(HGDIOBJ ho)
// 역할: 논리적 펜, 브러시, 폰트, 비트맵, 영역, 또는 팔레트를 삭제하여 시스템 리소스를 확보
pub(super) fn delete_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hobj = uc.read_arg(0);
    let ctx = uc.get_data();
    let window_owned_region = ctx
        .win_event
        .lock()
        .unwrap()
        .windows
        .values()
        .any(|window| window.window_rgn == hobj);
    let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
    let selected_somewhere = gdi_objects.values().any(|obj| {
        matches!(
            obj,
            GdiObject::Dc {
                selected_bitmap,
                selected_font,
                selected_brush,
                selected_pen,
                selected_region,
                selected_palette,
                ..
            } if *selected_bitmap == hobj
                || *selected_font == hobj
                || *selected_brush == hobj
                || *selected_pen == hobj
                || *selected_region == hobj
                || *selected_palette == hobj
        )
    });

    let removed = if selected_somewhere || window_owned_region {
        None
    } else {
        gdi_objects.remove(&hobj)
    };
    drop(gdi_objects);

    let ret = if selected_somewhere || window_owned_region {
        0
    } else {
        ctx.forget_surface_bitmap(hobj);
        if let Some(GdiObject::Bitmap {
            bits_addr: Some(bits_addr),
            ..
        }) = removed
        {
            let _ = ctx.free_heap_block(bits_addr);
        }
        1
    };
    Some(ApiHookResult::callee(1, Some(ret)))
}

// API: HGDIOBJ GetStockObject(int i)
// 역할: 미리 정의된 펜, 브러시, 폰트 또는 팔레트 중 하나의 핸들을 가져옴
pub(super) fn get_stock_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let index = uc.read_arg(0);
    let ctx = uc.get_data();
    let handle = ctx.alloc_handle();
    ctx.gdi_objects
        .lock()
        .unwrap()
        .insert(handle, GdiObject::StockObject(index));
    crate::emu_log!("[GDI32] GetStockObject({}) -> HGDIOBJ {:#x}", index, handle);
    Some(ApiHookResult::callee(1, Some(handle as i32)))
}

// API: int GetDeviceCaps(HDC hdc, int index)
// 역할: 지정된 디바이스에 대한 특정 장치 구성 정보를 가져옴
pub(super) fn get_device_caps(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let index = uc.read_arg(1);
    let ctx = uc.get_data();
    let gdi_objects = ctx.gdi_objects.lock().unwrap();
    let mut result = 0;
    if let Some(GdiObject::Dc { width, height, .. }) = gdi_objects.get(&hdc) {
        match index {
            1 => result = *width,                      // HORZRES
            2 => result = *height,                     // VERTRES
            3 => result = (*width).max(1) * 254 / 96,  // HORZSIZE (mm, 96 DPI 가정)
            4 => result = (*height).max(1) * 254 / 96, // VERTSIZE (mm, 96 DPI 가정)
            5 => result = 96,                          // LOGPIXELSX
            6 => result = 96,                          // LOGPIXELSY
            10 => result = 32,                         // BITSPIXEL
            11 => result = 1,                          // PLANES
            12 => result = 32, // NUMBRUSHES 대용이 아니라 일부 앱이 COLORRES처럼 조회하는 경우 대응
            14 => result = 0,  // SHADEBLENDCAPS
            16 => result = 1 << 24, // NUMCOLORS/COLORRES 계열 조회에 대해 24-bit color depth
            17 => result = 0,  // NUMRESERVED
            18 => result = 0,  // CURVECAPS
            19 => result = 0,  // LINECAPS/FONTTYPE 등 미지원은 0
            20 => result = *width, // HORZRES
            21 => result = *height, // VERTRES
            22 => result = 96, // LOGPIXELSX
            23 => result = 96, // LOGPIXELSY
            24 => result = 32, // BITSPIXEL 계열 fallback
            25 => result = 0,  // NUMRESERVED
            26 => result = 0,  // CURVECAPS
            27 => result = 0,  // FONTTYPE
            _ => {}            // 알 수 없는 인덱스
        }
    }
    crate::emu_log!(
        "[GDI32] GetDeviceCaps({:#x}, {}) -> int {}",
        hdc,
        index,
        result
    );
    Some(ApiHookResult::callee(2, Some(result)))
}

// API: int GetObject(HANDLE h, int c, LPVOID pv)
// 역할: GDI 오브젝트의 정보를 BITMAP 구조체 등으로 반환
pub(super) fn get_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h = uc.read_arg(0);
    let _c = uc.read_arg(1);
    let pv = uc.read_arg(2);

    // 락을 먼저 해제하고 write (borrow 충돌 방지)
    let bitmap_info = {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        match gdi.get(&h) {
            Some(GdiObject::Bitmap {
                width,
                height,
                bits_addr,
                bit_count,
                ..
            }) => {
                let width_bytes = ((*width * u32::from(*bit_count).max(1)).div_ceil(16) * 2).max(1);
                Some((
                    *width,
                    *height,
                    width_bytes,
                    *bit_count,
                    bits_addr.unwrap_or(0),
                ))
            }
            Some(GdiObject::Font { .. }) => None,
            _ => None,
        }
    };

    let result = if let Some((width, height, width_bytes, bit_count, bits)) = bitmap_info {
        if pv != 0 {
            uc.write_u32(pv as u64, 0); // bmType
            uc.write_u32(pv as u64 + 4, width); // bmWidth
            uc.write_u32(pv as u64 + 8, height); // bmHeight
            uc.write_u32(pv as u64 + 12, width_bytes); // bmWidthBytes
            uc.write_u32(pv as u64 + 16, 1); // bmPlanes
            uc.write_u32(pv as u64 + 20, u32::from(bit_count)); // bmBitsPixel
            uc.write_u32(pv as u64 + 24, bits); // bmBits
        }
        28
    } else {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        match gdi.get(&h) {
            Some(GdiObject::Font { .. }) => 60,
            _ => 0,
        }
    };
    crate::emu_log!("[GDI32] GetObject({:#x}) -> {}", h, result);
    Some(ApiHookResult::callee(3, Some(result)))
}
