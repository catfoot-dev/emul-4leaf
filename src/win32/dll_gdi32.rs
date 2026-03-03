use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllGDI32 {}

impl DllGDI32 {
    pub fn text_out_a() -> Option<(usize, Option<i32>)>{
        println!("text_out_a");
        Some((0, None))
    }

    pub fn get_text_metrics_a() -> Option<(usize, Option<i32>)>{
        println!("get_text_metrics_a");
        Some((0, None))
    }

    pub fn select_object() -> Option<(usize, Option<i32>)>{
        println!("select_object");
        Some((0, None))
    }

    pub fn set_bk_mode() -> Option<(usize, Option<i32>)>{
        println!("set_bk_mode");
        Some((0, None))
    }

    pub fn get_text_extent_point32_a() -> Option<(usize, Option<i32>)>{
        println!("get_text_extent_point32_a");
        Some((0, None))
    }

    pub fn get_stock_object() -> Option<(usize, Option<i32>)>{
        println!("get_stock_object");
        Some((0, None))
    }

    pub fn delete_object() -> Option<(usize, Option<i32>)>{
        println!("delete_object");
        Some((0, None))
    }

    pub fn get_device_caps() -> Option<(usize, Option<i32>)>{
        println!("get_device_caps");
        Some((0, None))
    }

    pub fn create_font_indirect_a() -> Option<(usize, Option<i32>)>{
        println!("create_font_indirect_a");
        Some((0, None))
    }

    pub fn create_font_a() -> Option<(usize, Option<i32>)>{
        println!("create_font_a");
        Some((0, None))
    }

    pub fn create_dib_section() -> Option<(usize, Option<i32>)>{
        println!("create_dib_section");
        Some((0, None))
    }

    pub fn create_compatible_dc() -> Option<(usize, Option<i32>)>{
        println!("create_compatible_dc");
        Some((0, None))
    }

    pub fn create_pen() -> Option<(usize, Option<i32>)>{
        println!("create_pen");
        Some((0, None))
    }

    pub fn select_clip_rgn() -> Option<(usize, Option<i32>)>{
        println!("select_clip_rgn");
        Some((0, None))
    }

    pub fn create_rect_rgn() -> Option<(usize, Option<i32>)>{
        println!("create_rect_rgn");
        Some((0, None))
    }

    pub fn set_di_bits_to_device() -> Option<(usize, Option<i32>)>{
        println!("set_di_bits_to_device");
        Some((0, None))
    }

    pub fn set_stretch_blt_mode() -> Option<(usize, Option<i32>)>{
        println!("set_stretch_blt_mode");
        Some((0, None))
    }

    pub fn realize_palette() -> Option<(usize, Option<i32>)>{
        println!("realize_palette");
        Some((0, None))
    }

    pub fn select_palette() -> Option<(usize, Option<i32>)>{
        println!("select_palette");
        Some((0, None))
    }

    pub fn create_solid_brush() -> Option<(usize, Option<i32>)>{
        println!("create_solid_brush");
        Some((0, None))
    }

    pub fn rectangle() -> Option<(usize, Option<i32>)>{
        println!("rectangle");
        Some((0, None))
    }

    pub fn move_to_ex() -> Option<(usize, Option<i32>)>{
        println!("move_to_ex");
        Some((0, None))
    }

    pub fn line_to() -> Option<(usize, Option<i32>)>{
        println!("line_to");
        Some((0, None))
    }

    pub fn create_compatible_bitmap() -> Option<(usize, Option<i32>)>{
        println!("create_compatible_bitmap");
        Some((0, None))
    }

    pub fn equal_rgn() -> Option<(usize, Option<i32>)>{
        println!("equal_rgn");
        Some((0, None))
    }

    pub fn get_text_color() -> Option<(usize, Option<i32>)>{
        println!("get_text_color");
        Some((0, None))
    }

    pub fn get_bk_mode() -> Option<(usize, Option<i32>)>{
        println!("get_bk_mode");
        Some((0, None))
    }

    pub fn set_rop2() -> Option<(usize, Option<i32>)>{
        println!("set_rop2");
        Some((0, None))
    }

    pub fn get_char_width_a() -> Option<(usize, Option<i32>)>{
        println!("get_char_width_a");
        Some((0, None))
    }

    pub fn get_text_extent_point_a() -> Option<(usize, Option<i32>)>{
        println!("get_text_extent_point_a");
        Some((0, None))
    }

    pub fn set_bk_color() -> Option<(usize, Option<i32>)>{
        println!("set_bk_color");
        Some((0, None))
    }

    pub fn set_text_color() -> Option<(usize, Option<i32>)>{
        println!("set_text_color");
        Some((0, None))
    }

    pub fn get_rgn_box() -> Option<(usize, Option<i32>)>{
        println!("get_rgn_box");
        Some((0, None))
    }

    pub fn combine_rgn() -> Option<(usize, Option<i32>)>{
        println!("combine_rgn");
        Some((0, None))
    }

    pub fn create_palette() -> Option<(usize, Option<i32>)>{
        println!("create_palette");
        Some((0, None))
    }

    pub fn delete_dc() -> Option<(usize, Option<i32>)>{
        println!("delete_dc");
        Some((0, None))
    }

    pub fn stretch_di_bits() -> Option<(usize, Option<i32>)>{
        println!("stretch_di_bits");
        Some((0, None))
    }

    pub fn get_pixel() -> Option<(usize, Option<i32>)>{
        println!("get_pixel");
        Some((0, None))
    }

    pub fn set_pixel() -> Option<(usize, Option<i32>)>{
        println!("set_pixel");
        Some((0, None))
    }

    pub fn bit_blt() -> Option<(usize, Option<i32>)>{
        println!("bit_blt");
        Some((0, None))
    }


    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "TextOutA" => DllGDI32::text_out_a(),
            "GetTextMetricsA" => DllGDI32::get_text_metrics_a(),
            "SelectObject" => DllGDI32::select_object(),
            "SetBkMode" => DllGDI32::set_bk_mode(),
            "GetTextExtentPoint32A" => DllGDI32::get_text_extent_point32_a(),
            "GetStockObject" => DllGDI32::get_stock_object(),
            "DeleteObject" => DllGDI32::delete_object(),
            "GetDeviceCaps" => DllGDI32::get_device_caps(),
            "CreateFontIndirectA" => DllGDI32::create_font_indirect_a(),
            "CreateFontA" => DllGDI32::create_font_a(),
            "CreateDIBSection" => DllGDI32::create_dib_section(),
            "CreateCompatibleDC" => DllGDI32::create_compatible_dc(),
            "CreatePen" => DllGDI32::create_pen(),
            "SelectClipRgn" => DllGDI32::select_clip_rgn(),
            "CreateRectRgn" => DllGDI32::create_rect_rgn(),
            "SetDIBitsToDevice" => DllGDI32::set_di_bits_to_device(),
            "SetStretchBltMode" => DllGDI32::set_stretch_blt_mode(),
            "RealizePalette" => DllGDI32::realize_palette(),
            "SelectPalette" => DllGDI32::select_palette(),
            "CreateSolidBrush" => DllGDI32::create_solid_brush(),
            "Rectangle" => DllGDI32::rectangle(),
            "MoveToEx" => DllGDI32::move_to_ex(),
            "LineTo" => DllGDI32::line_to(),
            "CreateCompatibleBitmap" => DllGDI32::create_compatible_bitmap(),
            "EqualRgn" => DllGDI32::equal_rgn(),
            "GetTextColor" => DllGDI32::get_text_color(),
            "GetBkMode" => DllGDI32::get_bk_mode(),
            "SetROP2" => DllGDI32::set_rop2(),
            "GetCharWidthA" => DllGDI32::get_char_width_a(),
            "GetTextExtentPointA" => DllGDI32::get_text_extent_point_a(),
            "SetBkColor" => DllGDI32::set_bk_color(),
            "SetTextColor" => DllGDI32::set_text_color(),
            "GetRgnBox" => DllGDI32::get_rgn_box(),
            "CombineRgn" => DllGDI32::combine_rgn(),
            "CreatePalette" => DllGDI32::create_palette(),
            "DeleteDC" => DllGDI32::delete_dc(),
            "StretchDIBits" => DllGDI32::stretch_di_bits(),
            "GetPixel" => DllGDI32::get_pixel(),
            "SetPixel" => DllGDI32::set_pixel(),
            "BitBlt" => DllGDI32::bit_blt(),
            _ => None
        }
    }
}
