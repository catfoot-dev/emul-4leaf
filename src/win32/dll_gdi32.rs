use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, GdiObject, Win32Context, callee_result};

/// `GDI32.dll` 프록시 구현 모듈
///
/// 그래픽 디바이스 인터페이스(GDI) 객체 (Font, DC, Bitmap 등) 생성/소멸 호출을 백그라운드에서 추적 및 가상화
pub struct DllGDI32;

impl DllGDI32 {
    /// 함수명 기준 `GDI32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // API: HDC CreateCompatibleDC(HDC hdc)
            // 역할: 지정된 디바이스와 호환되는 메모리 디바이스 컨텍스트(DC)를 만듦
            "CreateCompatibleDC" => {
                let hdc = uc.read_arg(0);
                let ctx = uc.get_data();
                let new_hdc = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    new_hdc,
                    GdiObject::Dc {
                        associated_window: 0,
                        selected_bitmap: 0,
                        selected_font: 0,
                        selected_brush: 0,
                        selected_pen: 0,
                        selected_region: 0,
                        selected_palette: 0,
                        bk_mode: 1, // TRANSPARENT(1) or OPAQUE(2)
                        bk_color: 0x00FFFFFF,
                        text_color: 0x00000000,
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
                Some((1, Some(new_hdc as i32)))
            }

            // API: BOOL DeleteDC(HDC hdc)
            // 역할: 지정된 디바이스 컨텍스트(DC)를 삭제
            "DeleteDC" => {
                let hdc = uc.read_arg(0);
                uc.get_data().gdi_objects.lock().unwrap().remove(&hdc);
                crate::emu_log!("[GDI32] DeleteDC({:#x}) -> BOOL 1", hdc);
                Some((1, Some(1)))
            }

            // API: HBITMAP CreateDIBSection(HDC hdc, const BITMAPINFO *pbmi, UINT usage, VOID **ppvBits, HANDLE hSection, DWORD offset)
            // 역할: 애플리케이션이 직접 쓸 수 있는 DIB(장치 독립적 비트맵)를 생성
            "CreateDIBSection" => {
                let hdc = uc.read_arg(0);
                let bmi_addr = uc.read_arg(1);
                let usage = uc.read_arg(2);
                let bits_ptr_addr = uc.read_arg(3);
                let hsection = uc.read_arg(4);
                let offset = uc.read_arg(5);
                // BITMAPINFOHEADER: width at +4, height at +8
                let width = uc.read_u32(bmi_addr as u64 + 4);
                let height = uc.read_u32(bmi_addr as u64 + 8);
                let bpp = uc.read_u32(bmi_addr as u64 + 14);
                let row_size = (width * bpp).div_ceil(32) * 4;
                let bmp_size = row_size * height;
                let bits_addr = uc.malloc(bmp_size as usize);
                if bits_ptr_addr != 0 {
                    uc.write_u32(bits_ptr_addr as u64, bits_addr as u32);
                }
                let ctx = uc.get_data();
                let hbmp = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hbmp,
                    GdiObject::Bitmap {
                        width,
                        height,
                        bits_ptr: bits_addr,
                    },
                );
                crate::emu_log!(
                    "[GDI32] CreateDIBSection({:#x}, {:#x}, {}, {:#x}, {:#x}, {}) -> HBITMAP {:#x}",
                    hdc,
                    bmi_addr,
                    usage,
                    bits_ptr_addr,
                    hsection,
                    offset,
                    hbmp
                );
                Some((6, Some(hbmp as i32)))
            }

            // API: HBITMAP CreateCompatibleBitmap(HDC hdc, int cx, int cy)
            // 역할: 지정된 디바이스 컨텍스트의 현재 설정과 호환되는 비트맵을 만듦
            "CreateCompatibleBitmap" => {
                let hdc = uc.read_arg(0);
                let width = uc.read_arg(1);
                let height = uc.read_arg(2);
                let ctx = uc.get_data();
                let hbmp = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hbmp,
                    GdiObject::Bitmap {
                        width,
                        height,
                        bits_ptr: 0,
                    },
                );
                crate::emu_log!(
                    "[GDI32] CreateCompatibleBitmap({:#x}, {}, {}) -> HBITMAP {:#x}",
                    hdc,
                    width,
                    height,
                    hbmp
                );
                Some((3, Some(hbmp as i32)))
            }

            // API: HGDIOBJ SelectObject(HDC hdc, HGDIOBJ h)
            // 역할: 지정된 DC로 객체를 선택하여 기존의 동일한 유형의 객체를 바꿈
            "SelectObject" => {
                let hdc = uc.read_arg(0);
                let hobj = uc.read_arg(1);

                let ctx = uc.get_data();
                let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
                let obj_clone = gdi_objects.get(&hobj).cloned();

                let mut old_hobj = 0;
                if let Some(GdiObject::Dc {
                    selected_bitmap,
                    selected_font,
                    selected_brush,
                    selected_pen,
                    selected_region,
                    selected_palette,
                    ..
                }) = gdi_objects.get_mut(&hdc)
                {
                    match obj_clone {
                        Some(GdiObject::Bitmap { .. }) => {
                            old_hobj = *selected_bitmap;
                            *selected_bitmap = hobj;
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
                        Some(GdiObject::Palette) => {
                            old_hobj = *selected_palette;
                            *selected_palette = hobj;
                        }
                        Some(GdiObject::StockObject(id)) => {
                            if id < 5 || id == 18 {
                                old_hobj = *selected_brush;
                                *selected_brush = hobj;
                            } else if id >= 5 && id <= 8 {
                                old_hobj = *selected_pen;
                                *selected_pen = hobj;
                            } else if id >= 10 && id <= 17 {
                                old_hobj = *selected_font;
                                *selected_font = hobj;
                            }
                        }
                        _ => {}
                    }
                }
                crate::emu_log!(
                    "[GDI32] SelectObject({:#x}, {:#x}) -> HGDIOBJ {:#x}",
                    hdc,
                    hobj,
                    old_hobj
                );
                Some((2, Some(old_hobj as i32)))
            }

            // API: BOOL DeleteObject(HGDIOBJ ho)
            // 역할: 논리적 펜, 브러시, 폰트, 비트맵, 영역, 또는 팔레트를 삭제하여 시스템 리소스를 확보
            "DeleteObject" => {
                let hobj = uc.read_arg(0);
                uc.get_data().gdi_objects.lock().unwrap().remove(&hobj);
                crate::emu_log!("[GDI32] DeleteObject({:#x}) -> BOOL 1", hobj);
                Some((1, Some(1)))
            }

            // API: HGDIOBJ GetStockObject(int i)
            // 역할: 미리 정의된 펜, 브러시, 폰트 또는 팔레트 중 하나의 핸들을 가져옴
            "GetStockObject" => {
                let index = uc.read_arg(0);
                let ctx = uc.get_data();
                let handle = ctx.alloc_handle();
                ctx.gdi_objects
                    .lock()
                    .unwrap()
                    .insert(handle, GdiObject::StockObject(index));
                crate::emu_log!("[GDI32] GetStockObject({}) -> HGDIOBJ {:#x}", index, handle);
                Some((1, Some(handle as i32)))
            }

            // API: int GetDeviceCaps(HDC hdc, int index)
            // 역할: 지정된 디바이스에 대한 특정 장치 구성 정보를 가져옴
            "GetDeviceCaps" => {
                let hdc = uc.read_arg(0);
                let index = uc.read_arg(1);
                let result = match index {
                    8 => 800,  // HORZRES
                    10 => 600, // VERTRES
                    12 => 32,  // BITSPIXEL
                    88 => 96,  // LOGPIXELSX
                    90 => 96,  // LOGPIXELSY
                    _ => 0,
                };
                crate::emu_log!(
                    "[GDI32] GetDeviceCaps({:#x}, {}) -> int {:#x}",
                    hdc,
                    index,
                    result
                );
                Some((2, Some(result)))
            }

            // API: HFONT CreateFontIndirectA(const LOGFONTA *lplf)
            // 역할: LOGFONT 구조체에 지정된 특성을 가진 논리적 폰트를 생성
            "CreateFontIndirectA" => {
                let logfont_addr = uc.read_arg(0);
                let logfont = if logfont_addr != 0 {
                    uc.read_euc_kr(logfont_addr as u64)
                } else {
                    String::new()
                };
                let ctx = uc.get_data();
                let hfont = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hfont,
                    GdiObject::Font {
                        name: "Default".to_string(),
                        height: 12,
                    },
                );
                crate::emu_log!(
                    "[GDI32] CreateFontIndirectA({:#x}=\"{}\") -> HFONT {:#x}",
                    logfont_addr,
                    logfont,
                    hfont
                );
                Some((1, Some(hfont as i32)))
            }

            // API: HFONT CreateFontA(int cHeight, int cWidth, int cEscapement, int cOrientation, int cWeight, DWORD bItalic, DWORD bUnderline, DWORD bStrikeOut, DWORD iCharSet, DWORD iOutPrecision, DWORD iClipPrecision, DWORD iQuality, DWORD iPitchAndFamily, LPCSTR pszFaceName)
            // 역할: 지정된 특성을 가진 논리적 폰트를 생성
            "CreateFontA" => {
                let height = uc.read_arg(0) as i32;
                let width = uc.read_arg(1) as i32;
                let escapement = uc.read_arg(2) as i32;
                let orientation = uc.read_arg(3) as i32;
                let weight = uc.read_arg(4) as i32;
                let italic = uc.read_arg(5) as i32;
                let underline = uc.read_arg(6) as i32;
                let strikeout = uc.read_arg(7) as i32;
                let charset = uc.read_arg(8) as i32;
                let out_precision = uc.read_arg(9) as i32;
                let clip_precision = uc.read_arg(10) as i32;
                let quality = uc.read_arg(11) as i32;
                let pitch_and_family = uc.read_arg(12) as i32;
                let face_name_addr = uc.read_arg(13);
                let face_name = if face_name_addr != 0 {
                    uc.read_euc_kr(face_name_addr as u64)
                } else {
                    String::new()
                };

                let ctx = uc.get_data();
                let hfont = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hfont,
                    GdiObject::Font {
                        name: "Default".to_string(),
                        height,
                    },
                );
                crate::emu_log!(
                    "[GDI32] CreateFontA({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {:#x}=\"{}\") -> HFONT {:#x}",
                    height,
                    width,
                    escapement,
                    orientation,
                    weight,
                    italic,
                    underline,
                    strikeout,
                    charset,
                    out_precision,
                    clip_precision,
                    quality,
                    pitch_and_family,
                    face_name_addr,
                    face_name,
                    hfont
                );
                Some((14, Some(hfont as i32)))
            }

            "GetTextMetricsA" => {
                let hdc = uc.read_arg(0);
                let tm_addr = uc.read_arg(1);
                // TEXTMETRIC 구조체 최소 채움 (tmHeight=16, tmAscent=12, tmDescent=4, ...)
                let zeros = [0u8; 56]; // TEXTMETRIC 크기
                uc.mem_write(tm_addr as u64, &zeros).unwrap();
                uc.write_u32(tm_addr as u64, 16); // tmHeight
                uc.write_u32(tm_addr as u64 + 4, 12); // tmAscent
                uc.write_u32(tm_addr as u64 + 8, 4); // tmDescent
                uc.write_u32(tm_addr as u64 + 20, 8); // tmAveCharWidth
                uc.write_u32(tm_addr as u64 + 24, 8); // tmMaxCharWidth
                crate::emu_log!(
                    "[GDI32] GetTextMetricsA({:#x}, {:#x}) -> BOOL 1",
                    hdc,
                    tm_addr
                );
                Some((2, Some(1)))
            }

            "GetTextExtentPoint32A" => {
                let hdc = uc.read_arg(0);
                let str_addr = uc.read_arg(1);
                let str = if str_addr != 0 {
                    uc.read_euc_kr(str_addr as u64)
                } else {
                    String::new()
                };
                let len = uc.read_arg(2);
                let size_addr = uc.read_arg(3);
                // SIZE: cx, cy
                uc.write_u32(size_addr as u64, len * 8); // 8px per char
                uc.write_u32(size_addr as u64 + 4, 16); // height
                crate::emu_log!(
                    "[GDI32] GetTextExtentPoint32A({:#x}, {:#x}=\"{}\", {}, {:#x}) -> BOOL 1",
                    hdc,
                    str_addr,
                    str,
                    len,
                    size_addr
                );
                Some((4, Some(1)))
            }

            "GetTextExtentPointA" => {
                let hdc = uc.read_arg(0);
                let str_addr = uc.read_arg(1);
                let str = if str_addr != 0 {
                    uc.read_euc_kr(str_addr as u64)
                } else {
                    String::new()
                };
                let len = uc.read_arg(2);
                let size_addr = uc.read_arg(3);
                uc.write_u32(size_addr as u64, len * 8);
                uc.write_u32(size_addr as u64 + 4, 16);
                crate::emu_log!(
                    "[GDI32] GetTextExtentPointA({:#x}, {:#x}=\"{}\", {}, {:#x}) -> BOOL 1",
                    hdc,
                    str_addr,
                    str,
                    len,
                    size_addr
                );
                Some((4, Some(1)))
            }

            "GetCharWidthA" => {
                // GetCharWidthA(HDC, UINT, UINT, LPINT)
                let hdc = uc.read_arg(0);
                let first = uc.read_arg(1);
                let last = uc.read_arg(2);
                let buf_addr = uc.read_arg(3);
                for i in 0..=(last - first) {
                    uc.write_u32(buf_addr as u64 + (i * 4) as u64, 8);
                }
                crate::emu_log!(
                    "[GDI32] GetCharWidthA({:#x}, {}, {}, {:#x}) -> BOOL 1",
                    hdc,
                    first,
                    last,
                    buf_addr
                );
                Some((4, Some(1)))
            }

            "TextOutA" => {
                let hdc = uc.read_arg(0);
                let x = uc.read_arg(1);
                let y = uc.read_arg(2);
                let text_addr = uc.read_arg(3);
                let text = if text_addr != 0 {
                    uc.read_euc_kr(text_addr as u64)
                } else {
                    String::new()
                };
                let len = uc.read_arg(4) as usize;

                if let Some(GdiObject::Dc { current_x, .. }) =
                    uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    *current_x += (len * 8) as i32;
                }
                crate::emu_log!(
                    "[GDI32] TextOutA({:#x}, {}, {}, {:#x}=\"{}\", {}) -> BOOL 1",
                    hdc,
                    x,
                    y,
                    text_addr,
                    text,
                    len
                );
                Some((5, Some(1)))
            }

            // API: int SetBkMode(HDC hdc, int mode)
            // 역할: 배경 혼합 모드를 설정
            "SetBkMode" => {
                let hdc = uc.read_arg(0);
                let mode = uc.read_arg(1) as i32;
                let mut old_mode = 1;
                if let Some(GdiObject::Dc { bk_mode, .. }) =
                    uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    old_mode = *bk_mode;
                    *bk_mode = mode;
                }
                crate::emu_log!(
                    "[GDI32] SetBkMode({:#x}, {:#x}) -> int {:#x}",
                    hdc,
                    mode,
                    old_mode
                );
                Some((2, Some(old_mode)))
            }

            "GetBkMode" => {
                let hdc = uc.read_arg(0);
                let mode = uc
                    .get_data()
                    .gdi_objects
                    .lock()
                    .unwrap()
                    .get(&hdc)
                    .map(|obj| {
                        if let GdiObject::Dc { bk_mode, .. } = obj {
                            *bk_mode
                        } else {
                            1
                        }
                    })
                    .unwrap_or(1);
                crate::emu_log!("[GDI32] GetBkMode({:#x}) -> int {:#x}", hdc, mode);
                Some((1, Some(mode)))
            }

            "SetBkColor" => {
                let hdc = uc.read_arg(0);
                let color = uc.read_arg(1);
                let mut old_color = 0x00FFFFFF;
                if let Some(GdiObject::Dc { bk_color, .. }) =
                    uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    old_color = *bk_color;
                    *bk_color = color;
                }
                crate::emu_log!(
                    "[GDI32] SetBkColor({:#x}, {:#x}) -> COLORREF {:#x}",
                    hdc,
                    color,
                    old_color
                );
                Some((2, Some(old_color as i32)))
            }

            // API: COLORREF SetTextColor(HDC hdc, COLORREF color)
            // 역할: 텍스트의 색상을 지정된 색상으로 설정
            "SetTextColor" => {
                let hdc = uc.read_arg(0);
                let color = uc.read_arg(1);
                let mut old_color = 0;
                if let Some(GdiObject::Dc { text_color, .. }) =
                    uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    old_color = *text_color;
                    *text_color = color;
                }
                crate::emu_log!(
                    "[GDI32] SetTextColor({:#x}, {:#x}) -> COLORREF {:#x}",
                    hdc,
                    color,
                    old_color
                );
                Some((2, Some(old_color as i32)))
            }

            "GetTextColor" => {
                let hdc = uc.read_arg(0);
                let color = uc
                    .get_data()
                    .gdi_objects
                    .lock()
                    .unwrap()
                    .get(&hdc)
                    .map(|obj| {
                        if let GdiObject::Dc { text_color, .. } = obj {
                            *text_color
                        } else {
                            0
                        }
                    })
                    .unwrap_or(0);
                crate::emu_log!("[GDI32] GetTextColor({:#x}) -> COLORREF {:#x}", hdc, color);
                Some((1, Some(color as i32)))
            }

            // API: HPEN CreatePen(int iStyle, int cWidth, COLORREF color)
            // 역할: 지정된 스타일, 너비 및 색상을 가진 논리적 펜을 생성
            "CreatePen" => {
                let style = uc.read_arg(0);
                let width = uc.read_arg(1);
                let color = uc.read_arg(2);
                let ctx = uc.get_data();
                let hpen = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hpen,
                    GdiObject::Pen {
                        style,
                        width,
                        color,
                    },
                );
                crate::emu_log!(
                    "[GDI32] CreatePen({:#x}, {}, {:#x}) -> HPEN {:#x}",
                    style,
                    width,
                    color,
                    hpen
                );
                Some((3, Some(hpen as i32)))
            }

            // API: HBRUSH CreateSolidBrush(COLORREF color)
            // 역할: 지정된 단색을 가지는 논리적 브러시를 생성
            "CreateSolidBrush" => {
                let color = uc.read_arg(0);
                let ctx = uc.get_data();
                let hbrush = ctx.alloc_handle();
                ctx.gdi_objects
                    .lock()
                    .unwrap()
                    .insert(hbrush, GdiObject::Brush { color });
                crate::emu_log!(
                    "[GDI32] CreateSolidBrush({:#x}) -> HBRUSH {:#x}",
                    color,
                    hbrush
                );
                Some((1, Some(hbrush as i32)))
            }

            // API: HRGN CreateRectRgn(int x1, int y1, int x2, int y2)
            // 역할: 직사각형 영역(Region)을 생성
            "CreateRectRgn" => {
                let left = uc.read_arg(0) as i32;
                let top = uc.read_arg(1) as i32;
                let right = uc.read_arg(2) as i32;
                let bottom = uc.read_arg(3) as i32;
                let ctx = uc.get_data();
                let hrgn = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hrgn,
                    GdiObject::Region {
                        left,
                        top,
                        right,
                        bottom,
                    },
                );
                crate::emu_log!(
                    "[GDI32] CreateRectRgn({}, {}, {}, {}) -> HRGN {:#x}",
                    left,
                    top,
                    right,
                    bottom,
                    hrgn
                );
                Some((4, Some(hrgn as i32)))
            }

            // API: int SelectClipRgn(HDC hdc, HRGN hrgn)
            "SelectClipRgn" => {
                let hdc = uc.read_arg(0);
                let hrgn = uc.read_arg(1);
                if let Some(GdiObject::Dc {
                    selected_region, ..
                }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    *selected_region = hrgn;
                }
                crate::emu_log!("[GDI32] SelectClipRgn({:#x}, {:#x}) -> int 1", hdc, hrgn);
                Some((2, Some(1))) // SIMPLEREGION
            }

            "CombineRgn" => {
                let hdc = uc.read_arg(0);
                let hrgn1 = uc.read_arg(1);
                let hrgn2 = uc.read_arg(2);
                let hrgn3 = uc.read_arg(3);
                crate::emu_log!(
                    "[GDI32] CombineRgn({:#x}, {:#x}, {:#x}, {:#x}) -> int 1",
                    hdc,
                    hrgn1,
                    hrgn2,
                    hrgn3
                );
                Some((4, Some(1))) // SIMPLEREGION
            }

            "EqualRgn" => {
                let hrgn1 = uc.read_arg(0);
                let hrgn2 = uc.read_arg(1);
                crate::emu_log!("[GDI32] EqualRgn({:#x}, {:#x}) -> int 0", hrgn1, hrgn2);
                Some((2, Some(0)))
            }

            "GetRgnBox" => {
                let hrgn = uc.read_arg(0);
                let rect_addr = uc.read_arg(1);
                uc.write_u32(rect_addr as u64, 0);
                uc.write_u32(rect_addr as u64 + 4, 0);
                uc.write_u32(rect_addr as u64 + 8, 640);
                uc.write_u32(rect_addr as u64 + 12, 480);
                crate::emu_log!("[GDI32] GetRgnBox({:#x}, {:#x}) -> int 1", hrgn, rect_addr);
                Some((2, Some(1))) // SIMPLEREGION
            }

            "SetDIBitsToDevice" => {
                let hdc = uc.read_arg(0);
                let x_dest = uc.read_arg(1) as i32;
                let y_dest = uc.read_arg(2) as i32;
                let w = uc.read_arg(3);
                let h = uc.read_arg(4);
                crate::emu_log!(
                    "[GDI32] SetDIBitsToDevice({:#x}, {}, {} {}, {}) -> int {:#x}",
                    hdc,
                    x_dest,
                    y_dest,
                    w,
                    h,
                    h
                );
                Some((12, Some(h as i32))) // Number of scanlines set
            }

            "StretchDIBits" => {
                let hdc = uc.read_arg(0);
                let x_dest = uc.read_arg(1) as i32;
                let y_dest = uc.read_arg(2) as i32;
                let dest_w = uc.read_arg(3);
                let dest_h = uc.read_arg(4);
                crate::emu_log!(
                    "[GDI32] StretchDIBits({:#x}, {}, {} {}, {}) -> int 1",
                    hdc,
                    x_dest,
                    y_dest,
                    dest_w,
                    dest_h
                );
                // Windows normally returns number of scanlines copied
                Some((13, Some(1)))
            }

            "SetStretchBltMode" => {
                let hdc = uc.read_arg(0);
                let mode = uc.read_arg(1);
                crate::emu_log!("[GDI32] SetStretchBltMode({:#x}, {}) -> int 1", hdc, mode);
                Some((2, Some(1))) // BLACKONWHITE
            }

            "BitBlt" => {
                let hdc_dest = uc.read_arg(0);
                let x = uc.read_arg(1) as i32;
                let y = uc.read_arg(2) as i32;
                let w = uc.read_arg(3);
                let h = uc.read_arg(4);
                let hdc_src = uc.read_arg(5);
                let x_src = uc.read_arg(6) as i32;
                let y_src = uc.read_arg(7) as i32;
                let rop = uc.read_arg(8);
                crate::emu_log!(
                    "[GDI32] BitBlt({:#x}, {}, {}, {}, {}, {:#x}, {}, {}, {:#x}) -> int 1",
                    hdc_dest,
                    x,
                    y,
                    w,
                    h,
                    hdc_src,
                    x_src,
                    y_src,
                    rop
                );
                Some((9, Some(1)))
            }

            // API: BOOL Rectangle(HDC hdc, int left, int top, int right, int bottom)
            // 역할: 현재 펜과 브러시를 사용하여 직사각형을 그림
            "Rectangle" => {
                let hdc = uc.read_arg(0);
                let l = uc.read_arg(1) as i32;
                let t = uc.read_arg(2) as i32;
                let r = uc.read_arg(3) as i32;
                let b = uc.read_arg(4) as i32;
                crate::emu_log!(
                    "[GDI32] Rectangle({:#x}, {}, {}, {}, {}) -> int 1",
                    hdc,
                    l,
                    t,
                    r,
                    b
                );
                Some((5, Some(1)))
            }

            // API: BOOL MoveToEx(HDC hdc, int x, int y, LPPOINT lppt)
            // 역할: 현재 그리기 위치를 지정된 좌표로 갱신
            "MoveToEx" => {
                let hdc = uc.read_arg(0);
                let x = uc.read_arg(1) as i32;
                let y = uc.read_arg(2) as i32;
                let lppt = uc.read_arg(3);

                let mut old_x = 0;
                let mut old_y = 0;
                if let Some(GdiObject::Dc {
                    current_x,
                    current_y,
                    ..
                }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    old_x = *current_x;
                    old_y = *current_y;
                    *current_x = x;
                    *current_y = y;
                }

                if lppt != 0 {
                    uc.write_u32(lppt as u64, old_x as u32);
                    uc.write_u32(lppt as u64 + 4, old_y as u32);
                }
                crate::emu_log!(
                    "[GDI32] MoveToEx({:#x}, {}, {}, {:#x}) -> int 1",
                    hdc,
                    x,
                    y,
                    lppt
                );
                Some((4, Some(1)))
            }

            // API: BOOL LineTo(HDC hdc, int x, int y)
            // 역할: 현재 위치에서 지정된 끝점까지 선을 그림
            "LineTo" => {
                let hdc = uc.read_arg(0);
                let x = uc.read_arg(1) as i32;
                let y = uc.read_arg(2) as i32;
                if let Some(GdiObject::Dc {
                    current_x,
                    current_y,
                    ..
                }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    *current_x = x;
                    *current_y = y;
                }
                crate::emu_log!("[GDI32] LineTo({:#x}, {}, {}) -> int 1", hdc, x, y);
                Some((3, Some(1)))
            }

            "SetROP2" => {
                let hdc = uc.read_arg(0);
                let mode = uc.read_arg(1) as i32;
                let mut old_mode = 13;
                if let Some(GdiObject::Dc { rop2_mode, .. }) =
                    uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    old_mode = *rop2_mode;
                    *rop2_mode = mode;
                }
                crate::emu_log!(
                    "[GDI32] SetROP2({:#x}, {}) -> int {:#x}",
                    hdc,
                    mode,
                    old_mode
                );
                Some((2, Some(old_mode)))
            }

            "RealizePalette" => {
                let hdc = uc.read_arg(0);
                let hpal = uc.read_arg(1);
                crate::emu_log!("[GDI32] RealizePalette({:#x}, {:#x}) -> UINT 0", hdc, hpal);
                Some((2, Some(0)))
            }

            "SelectPalette" => {
                let hdc = uc.read_arg(0);
                let hpal = uc.read_arg(1);
                let force_background = uc.read_arg(2);
                let mut old_pal = 0;
                if let Some(GdiObject::Dc {
                    selected_palette, ..
                }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
                {
                    old_pal = *selected_palette;
                    *selected_palette = hpal;
                }
                crate::emu_log!(
                    "[GDI32] SelectPalette({:#x}, {:#x}, {}) -> HPAL {:#x}",
                    hdc,
                    hpal,
                    force_background,
                    old_pal
                );
                Some((3, Some(old_pal as i32)))
            }

            "CreatePalette" => {
                let logpal_addr = uc.read_arg(0);
                let ctx = uc.get_data();
                let hpal = ctx.alloc_handle();
                ctx.gdi_objects
                    .lock()
                    .unwrap()
                    .insert(hpal, GdiObject::Palette);
                crate::emu_log!(
                    "[GDI32] CreatePalette({:#x}) -> HPAL {:#x}",
                    logpal_addr,
                    hpal
                );
                Some((1, Some(hpal as i32)))
            }

            "GetPixel" => {
                let hdc = uc.read_arg(0);
                let x = uc.read_arg(1);
                let y = uc.read_arg(2);
                crate::emu_log!(
                    "[GDI32] GetPixel({:#x}, {}, {}) -> COLORREF {:#x}",
                    hdc,
                    x,
                    y,
                    0x00FFFFFF
                );
                Some((3, Some(0x00FFFFFF))) // White dummy
            }

            "SetPixel" => {
                let hdc = uc.read_arg(0);
                let x = uc.read_arg(1);
                let y = uc.read_arg(2);
                let color = uc.read_arg(3);
                crate::emu_log!(
                    "[GDI32] SetPixel({:#x}, {}, {}, {:#x}) -> COLORREF {:#x}",
                    hdc,
                    x,
                    y,
                    color,
                    color
                );
                Some((4, Some(color as i32))) // Returns color set
            }

            _ => {
                crate::emu_log!("[GDI32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
