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
                let ctx = uc.get_data();
                let hdc = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hdc,
                    GdiObject::Dc {
                        associated_window: 0,
                    },
                );
                crate::emu_log!("[GDI32] CreateCompatibleDC(...) -> HDC {:#x}", hdc);
                Some((1, Some(hdc as i32)))
            }

            // API: BOOL DeleteDC(HDC hdc)
            // 역할: 지정된 디바이스 컨텍스트(DC)를 삭제
            "DeleteDC" => Some((1, Some(1))),

            // API: HBITMAP CreateDIBSection(HDC hdc, const BITMAPINFO *pbmi, UINT usage, VOID **ppvBits, HANDLE hSection, DWORD offset)
            // 역할: 애플리케이션이 직접 쓸 수 있는 DIB(장치 독립적 비트맵)를 생성
            "CreateDIBSection" => {
                let _hdc = uc.read_arg(0);
                let bmi_addr = uc.read_arg(1);
                let _usage = uc.read_arg(2);
                let bits_ptr_addr = uc.read_arg(3);
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
                    "[GDI32] CreateDIBSection({}x{}, {}bpp) -> HBITMAP {:#x}",
                    width,
                    height,
                    bpp,
                    hbmp
                );
                Some((6, Some(hbmp as i32)))
            }

            // API: HBITMAP CreateCompatibleBitmap(HDC hdc, int cx, int cy)
            // 역할: 지정된 디바이스 컨텍스트의 현재 설정과 호환되는 비트맵을 만듦
            "CreateCompatibleBitmap" => {
                let _hdc = uc.read_arg(0);
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
                Some((3, Some(hbmp as i32)))
            }

            // API: HGDIOBJ SelectObject(HDC hdc, HGDIOBJ h)
            // 역할: 지정된 DC로 객체를 선택하여 기존의 동일한 유형의 객체를 바꿈
            "SelectObject" => {
                let _hdc = uc.read_arg(0);
                let _hobj = uc.read_arg(1);
                // 이전 객체 반환 (0으로 간략화)
                Some((2, Some(0)))
            }

            // API: BOOL DeleteObject(HGDIOBJ ho)
            // 역할: 논리적 펜, 브러시, 폰트, 비트맵, 영역, 또는 팔레트를 삭제하여 시스템 리소스를 확보
            "DeleteObject" => Some((1, Some(1))),

            // API: HGDIOBJ GetStockObject(int i)
            // 역할: 미리 정의된 펜, 브러시, 폰트 또는 팔레트 중 하나의 핸들을 가져옴
            "GetStockObject" => {
                let index = uc.read_arg(0);
                let ctx = uc.get_data();
                let handle = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap()
                    .insert(handle, GdiObject::StockObject(index));
                Some((1, Some(handle as i32)))
            }

            // API: int GetDeviceCaps(HDC hdc, int index)
            // 역할: 지정된 디바이스에 대한 특정 장치 구성 정보를 가져옴
            "GetDeviceCaps" => {
                let _hdc = uc.read_arg(0);
                let index = uc.read_arg(1);
                let result = match index {
                    8 => 800,  // HORZRES
                    10 => 600, // VERTRES
                    12 => 32,  // BITSPIXEL
                    88 => 96,  // LOGPIXELSX
                    90 => 96,  // LOGPIXELSY
                    _ => 0,
                };
                Some((2, Some(result)))
            }

            // API: HFONT CreateFontIndirectA(const LOGFONTA *lplf)
            // 역할: LOGFONT 구조체에 지정된 특성을 가진 논리적 폰트를 생성
            "CreateFontIndirectA" => {
                let _logfont_addr = uc.read_arg(0);
                let ctx = uc.get_data();
                let hfont = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hfont,
                    GdiObject::Font {
                        name: "Default".to_string(),
                        height: 12,
                    },
                );
                Some((1, Some(hfont as i32)))
            }

            // API: HFONT CreateFontA(int cHeight, int cWidth, int cEscapement, int cOrientation, int cWeight, DWORD bItalic, DWORD bUnderline, DWORD bStrikeOut, DWORD iCharSet, DWORD iOutPrecision, DWORD iClipPrecision, DWORD iQuality, DWORD iPitchAndFamily, LPCSTR pszFaceName)
            // 역할: 지정된 특성을 가진 논리적 폰트를 생성
            "CreateFontA" => {
                let height = uc.read_arg(0) as i32;
                let ctx = uc.get_data();
                let hfont = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(
                    hfont,
                    GdiObject::Font {
                        name: "Default".to_string(),
                        height,
                    },
                );
                Some((14, Some(hfont as i32)))
            }

            "GetTextMetricsA" => {
                let _hdc = uc.read_arg(0);
                let tm_addr = uc.read_arg(1);
                // TEXTMETRIC 구조체 최소 채움 (tmHeight=16, tmAscent=12, tmDescent=4, ...)
                let zeros = [0u8; 56]; // TEXTMETRIC 크기
                uc.mem_write(tm_addr as u64, &zeros).unwrap();
                uc.write_u32(tm_addr as u64, 16); // tmHeight
                uc.write_u32(tm_addr as u64 + 4, 12); // tmAscent
                uc.write_u32(tm_addr as u64 + 8, 4); // tmDescent
                uc.write_u32(tm_addr as u64 + 20, 8); // tmAveCharWidth
                uc.write_u32(tm_addr as u64 + 24, 8); // tmMaxCharWidth
                Some((2, Some(1)))
            }

            "GetTextExtentPoint32A" => {
                let _hdc = uc.read_arg(0);
                let _str_addr = uc.read_arg(1);
                let len = uc.read_arg(2);
                let size_addr = uc.read_arg(3);
                // SIZE: cx, cy
                uc.write_u32(size_addr as u64, len * 8); // 8px per char
                uc.write_u32(size_addr as u64 + 4, 16); // height
                Some((4, Some(1)))
            }

            "GetTextExtentPointA" => {
                let _hdc = uc.read_arg(0);
                let _str_addr = uc.read_arg(1);
                let len = uc.read_arg(2);
                let size_addr = uc.read_arg(3);
                uc.write_u32(size_addr as u64, len * 8);
                uc.write_u32(size_addr as u64 + 4, 16);
                Some((4, Some(1)))
            }

            "GetCharWidthA" => {
                // GetCharWidthA(HDC, UINT, UINT, LPINT)
                let _hdc = uc.read_arg(0);
                let first = uc.read_arg(1);
                let last = uc.read_arg(2);
                let buf_addr = uc.read_arg(3);
                for i in 0..=(last - first) {
                    uc.write_u32(buf_addr as u64 + (i * 4) as u64, 8);
                }
                Some((4, Some(1)))
            }

            "TextOutA" => Some((5, Some(1))),

            // API: int SetBkMode(HDC hdc, int mode)
            // 역할: 배경 혼합 모드를 설정
            "SetBkMode" => Some((2, Some(1))),

            "GetBkMode" => Some((1, Some(1))), // OPAQUE

            "SetBkColor" => Some((2, Some(0))),

            // API: COLORREF SetTextColor(HDC hdc, COLORREF color)
            // 역할: 텍스트의 색상을 지정된 색상으로 설정
            "SetTextColor" => Some((2, Some(0))),

            "GetTextColor" => Some((1, Some(0))),

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
                Some((3, Some(hpen as i32)))
            }

            // API: HBRUSH CreateSolidBrush(COLORREF color)
            // 역할: 지정된 단색을 가지는 논리적 브러시를 생성
            "CreateSolidBrush" => {
                let color = uc.read_arg(0);
                let ctx = uc.get_data();
                let hbrush = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(hbrush, GdiObject::Brush { color });
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
                Some((4, Some(hrgn as i32)))
            }

            // API: int SelectClipRgn(HDC hdc, HRGN hrgn)
            // 역할: 지정된 영역을 디바이스 컨텍스트의 현재 클리핑 영역으로 선택
            "SelectClipRgn" => Some((2, Some(1))), // SIMPLEREGION

            "CombineRgn" => Some((4, Some(1))), // SIMPLEREGION

            "EqualRgn" => Some((2, Some(0))),

            "GetRgnBox" => {
                let _hrgn = uc.read_arg(0);
                let rect_addr = uc.read_arg(1);
                uc.write_u32(rect_addr as u64, 0);
                uc.write_u32(rect_addr as u64 + 4, 0);
                uc.write_u32(rect_addr as u64 + 8, 640);
                uc.write_u32(rect_addr as u64 + 12, 480);
                Some((2, Some(1))) // SIMPLEREGION
            }

            "SetDIBitsToDevice" => Some((12, Some(0))),

            "StretchDIBits" => Some((13, Some(0))),

            "SetStretchBltMode" => Some((2, Some(1))),

            "BitBlt" => Some((9, Some(1))),

            // API: BOOL Rectangle(HDC hdc, int left, int top, int right, int bottom)
            // 역할: 현재 펜과 브러시를 사용하여 직사각형을 그림
            "Rectangle" => Some((5, Some(1))),

            // API: BOOL MoveToEx(HDC hdc, int x, int y, LPPOINT lppt)
            // 역할: 현재 그리기 위치를 지정된 좌표로 갱신
            "MoveToEx" => Some((4, Some(1))),

            // API: BOOL LineTo(HDC hdc, int x, int y)
            // 역할: 현재 위치에서 지정된 끝점까지 선을 그림
            "LineTo" => Some((3, Some(1))),

            "SetROP2" => Some((2, Some(1))),

            "RealizePalette" => Some((1, Some(0))),

            "SelectPalette" => Some((3, Some(0))),

            "CreatePalette" => {
                let ctx = uc.get_data();
                let hpal = ctx.alloc_handle();
                ctx.gdi_objects.lock().unwrap().insert(hpal, GdiObject::Palette);
                Some((1, Some(hpal as i32)))
            }

            "GetPixel" => Some((3, Some(0))),

            "SetPixel" => Some((4, Some(0))),

            _ => {
                crate::emu_log!("[GDI32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
