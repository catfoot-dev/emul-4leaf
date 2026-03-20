use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, GdiObject, Win32Context, callee_result};

pub struct DllGDI32 {}

impl DllGDI32 {
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "CreateCompatibleDC" => {
                let ctx = uc.get_data_mut();
                let hdc = ctx.alloc_handle();
                ctx.gdi_objects.insert(
                    hdc,
                    GdiObject::Dc {
                        associated_window: 0,
                    },
                );
                println!("[GDI32] CreateCompatibleDC(...) -> HDC {:#x}", hdc);
                Some((1, Some(hdc as i32)))
            }
            "DeleteDC" => Some((1, Some(1))),
            "CreateDIBSection" => {
                let _hdc = uc.read_arg(0);
                let bmi_addr = uc.read_arg(1);
                let _usage = uc.read_arg(2);
                let bits_ptr_addr = uc.read_arg(3);
                // BITMAPINFOHEADER: width at +4, height at +8
                let width = uc.read_u32(bmi_addr as u64 + 4);
                let height = uc.read_u32(bmi_addr as u64 + 8);
                let bpp = uc.read_u32(bmi_addr as u64 + 14) as u32;
                let row_size = ((width * bpp + 31) / 32) * 4;
                let bmp_size = row_size * height;
                let bits_addr = uc.malloc(bmp_size as usize);
                if bits_ptr_addr != 0 {
                    uc.write_u32(bits_ptr_addr as u64, bits_addr as u32);
                }
                let ctx = uc.get_data_mut();
                let hbmp = ctx.alloc_handle();
                ctx.gdi_objects.insert(
                    hbmp,
                    GdiObject::Bitmap {
                        width,
                        height,
                        bits_ptr: bits_addr,
                    },
                );
                println!(
                    "[GDI32] CreateDIBSection({}x{}, {}bpp) -> HBITMAP {:#x}",
                    width, height, bpp, hbmp
                );
                Some((6, Some(hbmp as i32)))
            }
            "CreateCompatibleBitmap" => {
                let _hdc = uc.read_arg(0);
                let width = uc.read_arg(1);
                let height = uc.read_arg(2);
                let ctx = uc.get_data_mut();
                let hbmp = ctx.alloc_handle();
                ctx.gdi_objects.insert(
                    hbmp,
                    GdiObject::Bitmap {
                        width,
                        height,
                        bits_ptr: 0,
                    },
                );
                Some((3, Some(hbmp as i32)))
            }
            "SelectObject" => {
                let _hdc = uc.read_arg(0);
                let _hobj = uc.read_arg(1);
                // 이전 객체 반환 (0으로 간략화)
                Some((2, Some(0)))
            }
            "DeleteObject" => Some((1, Some(1))),
            "GetStockObject" => {
                let index = uc.read_arg(0);
                let ctx = uc.get_data_mut();
                let handle = ctx.alloc_handle();
                ctx.gdi_objects
                    .insert(handle, GdiObject::StockObject(index));
                Some((1, Some(handle as i32)))
            }
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
            "CreateFontIndirectA" => {
                let _logfont_addr = uc.read_arg(0);
                let ctx = uc.get_data_mut();
                let hfont = ctx.alloc_handle();
                ctx.gdi_objects.insert(
                    hfont,
                    GdiObject::Font {
                        name: "Default".to_string(),
                        height: 12,
                    },
                );
                Some((1, Some(hfont as i32)))
            }
            "CreateFontA" => {
                let height = uc.read_arg(0) as i32;
                let ctx = uc.get_data_mut();
                let hfont = ctx.alloc_handle();
                ctx.gdi_objects.insert(
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
            "SetBkMode" => Some((2, Some(1))),
            "GetBkMode" => Some((1, Some(1))), // OPAQUE
            "SetBkColor" => Some((2, Some(0))),
            "SetTextColor" => Some((2, Some(0))),
            "GetTextColor" => Some((1, Some(0))),
            "CreatePen" => {
                let style = uc.read_arg(0);
                let width = uc.read_arg(1);
                let color = uc.read_arg(2);
                let ctx = uc.get_data_mut();
                let hpen = ctx.alloc_handle();
                ctx.gdi_objects.insert(
                    hpen,
                    GdiObject::Pen {
                        style,
                        width,
                        color,
                    },
                );
                Some((3, Some(hpen as i32)))
            }
            "CreateSolidBrush" => {
                let color = uc.read_arg(0);
                let ctx = uc.get_data_mut();
                let hbrush = ctx.alloc_handle();
                ctx.gdi_objects.insert(hbrush, GdiObject::Brush { color });
                Some((1, Some(hbrush as i32)))
            }
            "CreateRectRgn" => {
                let left = uc.read_arg(0) as i32;
                let top = uc.read_arg(1) as i32;
                let right = uc.read_arg(2) as i32;
                let bottom = uc.read_arg(3) as i32;
                let ctx = uc.get_data_mut();
                let hrgn = ctx.alloc_handle();
                ctx.gdi_objects.insert(
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
            "SelectClipRgn" => Some((2, Some(1))), // SIMPLEREGION
            "CombineRgn" => Some((4, Some(1))),    // SIMPLEREGION
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
            "Rectangle" => Some((5, Some(1))),
            "MoveToEx" => Some((4, Some(1))),
            "LineTo" => Some((3, Some(1))),
            "SetROP2" => Some((2, Some(1))),
            "RealizePalette" => Some((1, Some(0))),
            "SelectPalette" => Some((3, Some(0))),
            "CreatePalette" => {
                let ctx = uc.get_data_mut();
                let hpal = ctx.alloc_handle();
                ctx.gdi_objects.insert(hpal, GdiObject::Palette);
                Some((1, Some(hpal as i32)))
            }
            "GetPixel" => Some((3, Some(0))),
            "SetPixel" => Some((4, Some(0))),
            _ => {
                println!("[GDI32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
