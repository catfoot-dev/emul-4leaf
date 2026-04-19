use crate::dll::win32::{ApiHookResult, CursorFrame, GdiObject, IconFrame, Win32Context};
use crate::helper::UnicornHelper;
use goblin::pe::PE;
use std::sync::atomic::Ordering;
use unicorn_engine::Unicorn;

use super::USER32;

const SPI_GETWORKAREA: u32 = 0x0030;
const ERROR_INVALID_PARAMETER: u32 = 87;
const IDI_APPLICATION: u32 = 32512;
const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;

// API: HCURSOR LoadCursorA(HINSTANCE hInstance, LPCSTR lpCursorName)
pub(super) fn load_cursor_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let instance = uc.read_arg(0);
    let lpcursorname = uc.read_arg(1);
    let ctx = uc.get_data();

    let (res_id, name) = if lpcursorname < 0x10000 {
        (lpcursorname, None)
    } else {
        (0, Some(uc.read_string(lpcursorname as u64)))
    };

    let handle = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        handle,
        GdiObject::Cursor {
            resource_id: res_id,
            name: name.clone(),
            frames: Vec::new(),
            is_animated: false,
            display_rate_jiffies: 0,
        },
    );

    crate::emu_log!(
        "[USER32] LoadCursorA({:#x}, {}) -> HCURSOR {:#x}",
        instance,
        if let Some(n) = name {
            n
        } else {
            format!("#{}", res_id)
        },
        handle
    );
    Some(ApiHookResult::callee(2, Some(handle as i32)))
}

// API: HCURSOR LoadCursorFromFileA(LPCSTR lpFileName)
pub(super) fn load_cursor_from_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lpfilename = uc.read_arg(0);
    let filename = uc.read_string(lpfilename as u64);
    let ctx = uc.get_data();

    let filename = crate::resource_dir()
        .join(&filename)
        .to_string_lossy()
        .to_string();
    let mut frames = Vec::new();
    let mut is_animated = false;
    let mut display_rate_jiffies: u32 = 10; // ANI 기본값 (≈167ms)

    if let Ok(data) = std::fs::read(&filename) {
        if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"ACON" {
            // simple ANI/RIFF parser
            is_animated = true;
            let mut pos = 12;
            while pos + 8 <= data.len() {
                let chunk_id = &data[pos..pos + 4];
                let chunk_size =
                    u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
                pos += 8;

                // anih 청크에서 기본 표시 간격(iDispRate)을 읽음
                if chunk_id == b"anih" && chunk_size >= 32 && pos + 32 <= data.len() {
                    let rate = u32::from_le_bytes(data[pos + 28..pos + 32].try_into().unwrap());
                    if rate > 0 {
                        display_rate_jiffies = rate;
                    }
                }

                if chunk_id == b"LIST" && pos + 4 <= data.len() && &data[pos..pos + 4] == b"fram" {
                    let mut list_pos = pos + 4;
                    let list_end = pos + chunk_size;
                    while list_pos + 8 <= list_end && list_pos + 8 <= data.len() {
                        let item_id = &data[list_pos..list_pos + 4];
                        let item_size = u32::from_le_bytes(
                            data[list_pos + 4..list_pos + 8].try_into().unwrap(),
                        ) as usize;
                        list_pos += 8;
                        if item_id == b"icon"
                            && let Some(frame) =
                                parse_cur_data(&data[list_pos..list_pos + item_size])
                        {
                            frames.push(frame);
                        }
                        list_pos += (item_size + 1) & !1;
                    }
                }
                pos += (chunk_size + 1) & !1;
            }
        } else if data.len() > 6 && data[0] == 0 && data[1] == 0 && data[2] == 2 && data[3] == 0 {
            // .cur file
            if let Some(frame) = parse_cur_data(&data) {
                frames.push(frame);
            }
        }
    }

    let handle = ctx.alloc_handle();
    let frames_len = frames.len();
    ctx.gdi_objects.lock().unwrap().insert(
        handle,
        GdiObject::Cursor {
            resource_id: 0,
            name: Some(filename.clone()),
            frames,
            is_animated,
            display_rate_jiffies,
        },
    );

    crate::emu_log!(
        "[USER32] LoadCursorFromFileA(\"{}\") -> HCURSOR {:#x} (frames: {}, animated: {})",
        filename,
        handle,
        frames_len,
        is_animated
    );
    Some(ApiHookResult::callee(1, Some(handle as i32)))
}

// API: HICON LoadIconA(HINSTANCE hInstance, LPCSTR lpIconName)
pub(super) fn load_icon_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let instance = uc.read_arg(0);
    let lpiconname = uc.read_arg(1);
    let ctx = uc.get_data();

    let (res_id, name) = if lpiconname < 0x10000 {
        (lpiconname, None)
    } else {
        (0, Some(uc.read_string(lpiconname as u64)))
    };
    let frames = if instance != 0 || res_id == IDI_APPLICATION {
        load_application_icon_frames(if res_id != 0 { Some(res_id) } else { None })
    } else {
        Vec::new()
    };

    let handle = ctx.alloc_handle();
    let frames_len = frames.len();
    ctx.gdi_objects.lock().unwrap().insert(
        handle,
        GdiObject::Icon {
            resource_id: res_id,
            name: name.clone(),
            frames,
        },
    );

    crate::emu_log!(
        "[USER32] LoadIconA({:#x}, {}) -> HICON {:#x} (frames: {})",
        instance,
        if let Some(n) = name {
            n
        } else {
            format!("#{}", res_id)
        },
        handle,
        frames_len
    );
    Some(ApiHookResult::callee(2, Some(handle as i32)))
}

fn load_application_icon_frames(requested_group_id: Option<u32>) -> Vec<IconFrame> {
    let exe_path = crate::resource_dir().join("4Leaf.exe");
    let Ok(buffer) = std::fs::read(&exe_path) else {
        return Vec::new();
    };
    let Ok(pe) = PE::parse(&buffer) else {
        return Vec::new();
    };
    let Some(resource_dir_info) = pe
        .header
        .optional_header
        .and_then(|opt| opt.data_directories.get_resource_table().copied())
    else {
        return Vec::new();
    };

    let r_rva = resource_dir_info.virtual_address;
    let r_size = resource_dir_info.size as usize;
    let r_offset = rva_to_offset(&pe, r_rva);
    if r_offset == 0 || r_offset.saturating_add(r_size) > buffer.len() {
        return Vec::new();
    }

    let res_section = &buffer[r_offset..r_offset + r_size];
    let Some(group_icon_data) =
        find_resource_data(res_section, &pe, &buffer, RT_GROUP_ICON, requested_group_id)
    else {
        return Vec::new();
    };
    if group_icon_data.len() < 6 {
        return Vec::new();
    }

    let count = u16::from_le_bytes([group_icon_data[4], group_icon_data[5]]) as usize;
    let mut frames = Vec::new();

    for index in 0..count {
        let entry_offset = 6 + index * 14;
        if entry_offset + 14 > group_icon_data.len() {
            break;
        }

        let bytes_in_res = u32::from_le_bytes(
            group_icon_data[entry_offset + 8..entry_offset + 12]
                .try_into()
                .unwrap(),
        );
        let icon_resource_id = u16::from_le_bytes(
            group_icon_data[entry_offset + 12..entry_offset + 14]
                .try_into()
                .unwrap(),
        ) as u32;
        let Some(icon_data) =
            find_resource_data(res_section, &pe, &buffer, RT_ICON, Some(icon_resource_id))
        else {
            continue;
        };
        let Some(ico_data) = build_ico_from_group_entry(
            &group_icon_data[entry_offset..entry_offset + 14],
            icon_data,
            bytes_in_res,
        ) else {
            continue;
        };
        if let Some(frame) = parse_icon_data(&ico_data) {
            frames.push(frame);
        }
    }

    frames
}

fn build_ico_from_group_entry(
    entry: &[u8],
    icon_data: &[u8],
    bytes_in_res: u32,
) -> Option<Vec<u8>> {
    if entry.len() < 14 {
        return None;
    }

    let icon_dir_offset: usize = 6 + 16;
    let total_len = icon_dir_offset.checked_add(icon_data.len())?;
    let mut ico = vec![0u8; total_len];
    ico[2..4].copy_from_slice(&1u16.to_le_bytes());
    ico[4..6].copy_from_slice(&1u16.to_le_bytes());
    ico[6] = entry[0];
    ico[7] = entry[1];
    ico[8] = entry[2];
    ico[9] = entry[3];
    ico[10..12].copy_from_slice(&entry[4..6]);
    ico[12..14].copy_from_slice(&entry[6..8]);
    ico[14..18].copy_from_slice(&bytes_in_res.to_le_bytes());
    ico[18..22].copy_from_slice(&(icon_dir_offset as u32).to_le_bytes());
    ico[icon_dir_offset..].copy_from_slice(icon_data);
    Some(ico)
}

fn parse_icon_data(data: &[u8]) -> Option<IconFrame> {
    let frame = parse_cur_data(data)?;
    Some(IconFrame {
        width: frame.width,
        height: frame.height,
        pixels: frame.pixels,
    })
}

fn find_resource_data<'a>(
    res_section: &'a [u8],
    pe: &PE,
    buffer: &'a [u8],
    type_id: u32,
    requested_id: Option<u32>,
) -> Option<&'a [u8]> {
    let (_, type_entry_offset) = find_resource_entry(res_section, 0, Some(type_id))?;
    let type_dir_offset = (type_entry_offset & 0x7FFF_FFFF) as usize;
    let (_, name_entry_offset) = find_resource_entry(res_section, type_dir_offset, requested_id)
        .or_else(|| find_resource_entry(res_section, type_dir_offset, None))?;
    let name_dir_offset = (name_entry_offset & 0x7FFF_FFFF) as usize;
    let (_, lang_entry_offset) = find_resource_entry(res_section, name_dir_offset, None)?;
    if (lang_entry_offset & 0x8000_0000) != 0 {
        return None;
    }

    let data_entry_offset = lang_entry_offset as usize;
    if data_entry_offset + 16 > res_section.len() {
        return None;
    }

    let data_rva = u32::from_le_bytes(
        res_section[data_entry_offset..data_entry_offset + 4]
            .try_into()
            .ok()?,
    );
    let data_size = u32::from_le_bytes(
        res_section[data_entry_offset + 4..data_entry_offset + 8]
            .try_into()
            .ok()?,
    ) as usize;
    let file_offset = rva_to_offset(pe, data_rva);
    (file_offset != 0 && file_offset.saturating_add(data_size) <= buffer.len())
        .then_some(&buffer[file_offset..file_offset + data_size])
}

fn find_resource_entry(
    section: &[u8],
    dir_offset: usize,
    wanted_id: Option<u32>,
) -> Option<(u32, u32)> {
    if dir_offset + 16 > section.len() {
        return None;
    }

    let named_entries =
        u16::from_le_bytes(section[dir_offset + 12..dir_offset + 14].try_into().ok()?);
    let id_entries = u16::from_le_bytes(section[dir_offset + 14..dir_offset + 16].try_into().ok()?);
    let total_entries = named_entries + id_entries;
    let mut fallback = None;

    for index in 0..total_entries {
        let entry_offset = dir_offset + 16 + index as usize * 8;
        if entry_offset + 8 > section.len() {
            break;
        }
        let name_or_id =
            u32::from_le_bytes(section[entry_offset..entry_offset + 4].try_into().ok()?);
        let offset_to_data = u32::from_le_bytes(
            section[entry_offset + 4..entry_offset + 8]
                .try_into()
                .ok()?,
        );

        if fallback.is_none() {
            fallback = Some((name_or_id, offset_to_data));
        }

        if let Some(wanted_id) = wanted_id
            && (name_or_id & 0x8000_0000) == 0
            && name_or_id == wanted_id
        {
            return Some((name_or_id, offset_to_data));
        }
    }

    fallback
}

fn rva_to_offset(pe: &PE, rva: u32) -> usize {
    for section in &pe.sections {
        if rva >= section.virtual_address && rva < section.virtual_address + section.virtual_size {
            return (rva - section.virtual_address + section.pointer_to_raw_data) as usize;
        }
    }
    0
}

pub(super) fn dib_row_stride(width: u32, bits_per_pixel: u16) -> Option<usize> {
    let row_bits = (width as usize).checked_mul(bits_per_pixel as usize)?;
    let aligned_dwords = row_bits.checked_add(31)? / 32;
    aligned_dwords.checked_mul(4)
}

pub(super) fn parse_cur_data(data: &[u8]) -> Option<CursorFrame> {
    if data.len() < 22 {
        return None;
    }
    let count = u16::from_le_bytes(data[4..6].try_into().ok()?) as usize;
    if count == 0 {
        return None;
    }

    // Take the first directory entry
    let entry_offset = 6;
    let mut width = data[entry_offset] as u32;
    let mut height = data[entry_offset + 1] as u32;
    let hotspot_x =
        u16::from_le_bytes(data[entry_offset + 4..entry_offset + 6].try_into().ok()?) as i32;
    let hotspot_y =
        u16::from_le_bytes(data[entry_offset + 6..entry_offset + 8].try_into().ok()?) as i32;
    let size =
        u32::from_le_bytes(data[entry_offset + 8..entry_offset + 12].try_into().ok()?) as usize;
    let offset =
        u32::from_le_bytes(data[entry_offset + 12..entry_offset + 16].try_into().ok()?) as usize;

    if offset + size > data.len() {
        return None;
    }

    let bmp_data = &data[offset..offset + size];
    if bmp_data.len() < 40 {
        return None;
    }

    let bi_size = u32::from_le_bytes(bmp_data[0..4].try_into().ok()?);
    let bi_width = i32::from_le_bytes(bmp_data[4..8].try_into().ok()?);
    let bi_height = i32::from_le_bytes(bmp_data[8..12].try_into().ok()?);
    let bi_bit_count = u16::from_le_bytes(bmp_data[14..16].try_into().ok()?);
    let bi_clr_used = u32::from_le_bytes(bmp_data[32..36].try_into().ok()?);

    if bi_size < 40 || bi_width == 0 || bi_height == 0 {
        return None;
    }

    if width == 0 {
        width = bi_width.unsigned_abs();
    }
    if height == 0 {
        height = (bi_height.abs() / 2) as u32;
    } // CUR height in BMP is double (XOR + AND)

    let pixel_count = (width as usize).checked_mul(height as usize)?;
    let mut pixels = vec![0u32; pixel_count];
    let palette_entry_count = match bi_bit_count {
        1 | 4 | 8 => {
            if bi_clr_used != 0 {
                bi_clr_used as usize
            } else {
                1usize << bi_bit_count
            }
        }
        _ => 0,
    };
    let palette_offset = bi_size as usize;
    let palette_len = palette_entry_count.checked_mul(4)?;
    let pixel_data_offset = palette_offset.checked_add(palette_len)?;
    let xor_stride = dib_row_stride(width, bi_bit_count)?;

    if pixel_data_offset > bmp_data.len() {
        return None;
    }
    if palette_offset
        .checked_add(palette_len)
        .is_none_or(|end| end > bmp_data.len())
    {
        return None;
    }

    let mut palette = Vec::with_capacity(palette_entry_count);
    for index in 0..palette_entry_count {
        let offset = palette_offset + index * 4;
        let b = bmp_data[offset] as u32;
        let g = bmp_data[offset + 1] as u32;
        let r = bmp_data[offset + 2] as u32;
        palette.push(0xFF00_0000 | (r << 16) | (g << 8) | b);
    }

    let xor_len = xor_stride.checked_mul(height as usize)?;
    if pixel_data_offset
        .checked_add(xor_len)
        .is_none_or(|end| end > bmp_data.len())
    {
        return None;
    }

    let bottom_up = bi_height > 0;
    let mut has_explicit_alpha = false;

    for y in 0..height as usize {
        let src_y = if bottom_up {
            height as usize - 1 - y
        } else {
            y
        };
        let row_offset = pixel_data_offset + src_y * xor_stride;
        for x in 0..width as usize {
            let color = match bi_bit_count {
                1 => {
                    let byte = *bmp_data.get(row_offset + x / 8)?;
                    let bit = 7 - (x % 8);
                    let palette_idx = ((byte >> bit) & 0x01) as usize;
                    *palette.get(palette_idx)?
                }
                4 => {
                    let byte = *bmp_data.get(row_offset + x / 2)?;
                    let palette_idx = if x % 2 == 0 { byte >> 4 } else { byte & 0x0F };
                    *palette.get(palette_idx as usize)?
                }
                8 => {
                    let palette_idx = *bmp_data.get(row_offset + x)? as usize;
                    *palette.get(palette_idx)?
                }
                24 => {
                    let offset = row_offset + x * 3;
                    let b = *bmp_data.get(offset)? as u32;
                    let g = *bmp_data.get(offset + 1)? as u32;
                    let r = *bmp_data.get(offset + 2)? as u32;
                    0xFF00_0000 | (r << 16) | (g << 8) | b
                }
                32 => {
                    let offset = row_offset + x * 4;
                    let b = *bmp_data.get(offset)? as u32;
                    let g = *bmp_data.get(offset + 1)? as u32;
                    let r = *bmp_data.get(offset + 2)? as u32;
                    let a = *bmp_data.get(offset + 3)? as u32;
                    if a != 0 {
                        has_explicit_alpha = true;
                    }
                    (a << 24) | (r << 16) | (g << 8) | b
                }
                _ => {
                    crate::emu_log!(
                        "[USER32] parse_cur_data: unsupported cursor bit depth {}",
                        bi_bit_count
                    );
                    return None;
                }
            };
            pixels[y * width as usize + x] = color;
        }
    }

    // 고전 CUR 포맷은 XOR 비트맵 뒤에 1bpp AND 마스크를 두므로,
    // 팔레트/24bpp 커서는 이 마스크로 투명도를 만들고 32bpp도 필요 시 보정합니다.
    let mask_stride = dib_row_stride(width, 1)?;
    let mask_offset = pixel_data_offset.checked_add(xor_len)?;
    let mask_len = mask_stride.checked_mul(height as usize)?;
    if mask_offset
        .checked_add(mask_len)
        .is_some_and(|end| end <= bmp_data.len())
    {
        for y in 0..height as usize {
            let src_y = if bottom_up {
                height as usize - 1 - y
            } else {
                y
            };
            let row_offset = mask_offset + src_y * mask_stride;
            for x in 0..width as usize {
                let byte = *bmp_data.get(row_offset + x / 8)?;
                let bit = 7 - (x % 8);
                let transparent = ((byte >> bit) & 0x01) != 0;
                let pixel = &mut pixels[y * width as usize + x];
                if transparent {
                    *pixel &= 0x00FF_FFFF;
                } else if bi_bit_count < 32 || !has_explicit_alpha {
                    *pixel |= 0xFF00_0000;
                }
            }
        }
    } else if bi_bit_count < 32 {
        for pixel in &mut pixels {
            *pixel |= 0xFF00_0000;
        }
    }

    Some(CursorFrame {
        width,
        height,
        hotspot_x,
        hotspot_y,
        pixels,
    })
}

// API: HCURSOR SetCursor(HCURSOR hCursor)
pub(super) fn set_cursor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hcursor = uc.read_arg(0);
    let ctx = uc.get_data();
    let old = ctx
        .current_cursor
        .swap(hcursor, std::sync::atomic::Ordering::SeqCst);

    // UI 스레드에도 커서 변경 알림을 보내되, 현재 wndproc 문맥의 HWND를 우선 사용합니다.
    let hwnd = USER32::resolve_cursor_target_hwnd(ctx);
    if hwnd != 0 {
        ctx.win_event
            .lock()
            .unwrap()
            .send_ui_command(crate::ui::UiCommand::SetCursor { hwnd, hcursor });
    }

    crate::emu_log!("[USER32] SetCursor({:#x}) -> HCURSOR {:#x}", hcursor, old);
    Some(ApiHookResult::callee(1, Some(old as i32)))
}

// API: BOOL DestroyCursor(HCURSOR hCursor)
pub(super) fn destroy_cursor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hcursor = uc.read_arg(0);
    let ctx = uc.get_data();
    ctx.gdi_objects.lock().unwrap().remove(&hcursor);
    crate::emu_log!("[USER32] DestroyCursor({:#x}) -> BOOL 1", hcursor);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: int MapWindowPoints(HWND hWndFrom, HWND hWndTo, LPPOINT lpPoints, UINT cPoints)
pub(super) fn map_window_points(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd_from = uc.read_arg(0);
    let hwnd_to = uc.read_arg(1);
    let lp_points = uc.read_arg(2);
    let c_points = uc.read_arg(3);

    let (from_x, from_y) = if hwnd_from == 0 {
        (0, 0)
    } else {
        let win_event = uc.get_data().win_event.lock().unwrap();
        win_event.client_screen_origin(hwnd_from).unwrap_or((0, 0))
    };

    let (to_x, to_y) = if hwnd_to == 0 {
        (0, 0)
    } else {
        let win_event = uc.get_data().win_event.lock().unwrap();
        win_event.client_screen_origin(hwnd_to).unwrap_or((0, 0))
    };

    let dx = from_x - to_x;
    let dy = from_y - to_y;

    for i in 0..c_points {
        let offset = (i as u64) * 8;
        let x = uc.read_u32(lp_points as u64 + offset) as i32;
        let y = uc.read_u32(lp_points as u64 + offset + 4) as i32;
        uc.write_u32(lp_points as u64 + offset, (x + dx) as u32);
        uc.write_u32(lp_points as u64 + offset + 4, (y + dy) as u32);
    }

    // Low word of return value is pixels horizontal, high word is pixels vertical
    let ret = (dx as u16 as u32) | ((dy as u16 as u32) << 16);
    crate::emu_log!(
        "[USER32] MapWindowPoints({:#x}, {:#x}, {:#x}, {:#x}) -> int {}",
        hwnd_from,
        hwnd_to,
        lp_points,
        c_points,
        ret
    );
    Some(ApiHookResult::callee(4, Some(ret as i32)))
}

// API: BOOL SystemParametersInfoA(UINT uiAction, UINT uiParam, PVOID pvParam, UINT fWinIni)
pub(super) fn system_parameters_info_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ui_action = uc.read_arg(0);
    let ui_param = uc.read_arg(1);
    let pv_param = uc.read_arg(2);
    let f_win_ini = uc.read_arg(3);

    let ret = match ui_action {
        SPI_GETWORKAREA => {
            if pv_param == 0 {
                uc.get_data()
                    .last_error
                    .store(ERROR_INVALID_PARAMETER, Ordering::SeqCst);
                crate::emu_log!(
                    "[USER32] SystemParametersInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 0 (pvParam is NULL)",
                    ui_action,
                    ui_param,
                    pv_param,
                    f_win_ini
                );
                0
            } else {
                let (left, top, right, bottom) = uc.get_data().work_area_rect();

                // 게스트가 넘긴 RECT 버퍼에 현재 가상 작업 영역을 그대로 기록합니다.
                uc.write_u32(pv_param as u64, left as u32);
                uc.write_u32(pv_param as u64 + 4, top as u32);
                uc.write_u32(pv_param as u64 + 8, right as u32);
                uc.write_u32(pv_param as u64 + 12, bottom as u32);
                uc.get_data().last_error.store(0, Ordering::SeqCst);
                crate::emu_log!(
                    "[USER32] SystemParametersInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1, work_area=({}, {}, {}, {})",
                    ui_action,
                    ui_param,
                    pv_param,
                    f_win_ini,
                    left,
                    top,
                    right,
                    bottom
                );
                1
            }
        }
        _ => {
            crate::emu_log!(
                "[USER32] SystemParametersInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1 (stub)",
                ui_action,
                ui_param,
                pv_param,
                f_win_ini
            );
            1
        }
    };

    Some(ApiHookResult::callee(4, Some(ret)))
}

// API: BOOL GetCursorPos(LPPOINT lpPoint)
pub(super) fn get_cursor_pos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let pt_addr = uc.read_arg(0);
    let ctx = uc.get_data();
    let x = ctx.mouse_x.load(std::sync::atomic::Ordering::SeqCst);
    let y = ctx.mouse_y.load(std::sync::atomic::Ordering::SeqCst);
    uc.write_u32(pt_addr as u64, x);
    uc.write_u32(pt_addr as u64 + 4, y);
    crate::emu_log!("[USER32] GetCursorPos({:#x}) -> BOOL 1", pt_addr);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL PtInRect(const RECT* lprc, POINT pt)
pub(super) fn pt_in_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let rect_addr = uc.read_arg(0);
    let pt_x = uc.read_arg(1) as i32;
    let pt_y = uc.read_arg(2) as i32;
    let left = uc.read_u32(rect_addr as u64) as i32;
    let top = uc.read_u32(rect_addr as u64 + 4) as i32;
    let right = uc.read_u32(rect_addr as u64 + 8) as i32;
    let bottom = uc.read_u32(rect_addr as u64 + 12) as i32;
    let inside = pt_x >= left && pt_x < right && pt_y >= top && pt_y < bottom;
    let ret = if inside { 1 } else { 0 };
    crate::emu_log!(
        "[USER32] PtInRect({:#x}, {{x:{}, y:{}}}) -> BOOL {}",
        rect_addr,
        pt_x,
        pt_y,
        ret
    );
    Some(ApiHookResult::callee(3, Some(ret)))
}

// API: BOOL SetRect(LPRECT lprc, int xLeft, int yTop, int xRight, int yBottom)
pub(super) fn set_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let rect_addr = uc.read_arg(0);
    let left = uc.read_arg(1) as i32;
    let top = uc.read_arg(2) as i32;
    let right = uc.read_arg(3) as i32;
    let bottom = uc.read_arg(4) as i32;
    uc.write_u32(rect_addr as u64, left as u32);
    uc.write_u32(rect_addr as u64 + 4, top as u32);
    uc.write_u32(rect_addr as u64 + 8, right as u32);
    uc.write_u32(rect_addr as u64 + 12, bottom as u32);
    // crate::emu_log!(
    //     "[USER32] SetRect({:#x}, {}, {}, {}, {}) -> BOOL 1",
    //     rect_addr,
    //     left,
    //     top,
    //     right,
    //     bottom
    // );
    Some(ApiHookResult::callee(5, Some(1)))
}

// API: BOOL EqualRect(const RECT* lprc1, const RECT* lprc2)
pub(super) fn equal_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let r1 = uc.read_arg(0);
    let r2 = uc.read_arg(1);
    let mut eq = true;
    for i in 0..4 {
        if uc.read_u32(r1 as u64 + i * 4) != uc.read_u32(r2 as u64 + i * 4) {
            eq = false;
            break;
        }
    }
    let ret = if eq { 1 } else { 0 };
    crate::emu_log!("[USER32] EqualRect({:#x}, {:#x}) -> BOOL {}", r1, r2, ret);
    Some(ApiHookResult::callee(2, Some(ret)))
}

// API: BOOL UnionRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
pub(super) fn union_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let src1 = uc.read_arg(1);
    let src2 = uc.read_arg(2);
    let l1 = uc.read_u32(src1 as u64) as i32;
    let t1 = uc.read_u32(src1 as u64 + 4) as i32;
    let r1 = uc.read_u32(src1 as u64 + 8) as i32;
    let b1 = uc.read_u32(src1 as u64 + 12) as i32;
    let l2 = uc.read_u32(src2 as u64) as i32;
    let t2 = uc.read_u32(src2 as u64 + 4) as i32;
    let r2 = uc.read_u32(src2 as u64 + 8) as i32;
    let b2 = uc.read_u32(src2 as u64 + 12) as i32;
    let l = l1.min(l2);
    let t = t1.min(t2);
    let r = r1.max(r2);
    let b = b1.max(b2);
    uc.write_u32(dst as u64, l as u32);
    uc.write_u32(dst as u64 + 4, t as u32);
    uc.write_u32(dst as u64 + 8, r as u32);
    uc.write_u32(dst as u64 + 12, b as u32);
    crate::emu_log!(
        "[USER32] UnionRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
        dst,
        src1,
        src2
    );
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: BOOL IntersectRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
pub(super) fn intersect_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let src1 = uc.read_arg(1);
    let src2 = uc.read_arg(2);
    let l1 = uc.read_u32(src1 as u64) as i32;
    let t1 = uc.read_u32(src1 as u64 + 4) as i32;
    let r1 = uc.read_u32(src1 as u64 + 8) as i32;
    let b1 = uc.read_u32(src1 as u64 + 12) as i32;
    let l2 = uc.read_u32(src2 as u64) as i32;
    let t2 = uc.read_u32(src2 as u64 + 4) as i32;
    let r2 = uc.read_u32(src2 as u64 + 8) as i32;
    let b2 = uc.read_u32(src2 as u64 + 12) as i32;
    let l = l1.max(l2);
    let t = t1.max(t2);
    let r = r1.min(r2);
    let b = b1.min(b2);
    let ret = if l < r && t < b {
        uc.write_u32(dst as u64, l as u32);
        uc.write_u32(dst as u64 + 4, t as u32);
        uc.write_u32(dst as u64 + 8, r as u32);
        uc.write_u32(dst as u64 + 12, b as u32);
        1
    } else {
        uc.write_u32(dst as u64, 0);
        uc.write_u32(dst as u64 + 4, 0);
        uc.write_u32(dst as u64 + 8, 0);
        uc.write_u32(dst as u64 + 12, 0);
        0
    };
    crate::emu_log!(
        "[USER32] IntersectRect({:#x}, {:#x}, {:#x}) -> BOOL {}",
        dst,
        src1,
        src2,
        ret
    );
    Some(ApiHookResult::callee(3, Some(ret)))
}

// API: HANDLE GetClipboardData(UINT uFormat)
pub(super) fn get_clipboard_data(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let format = uc.read_arg(0);
    if format == 1 {
        let (ptr, data) = {
            let ctx = uc.get_data();
            let cb = ctx.clipboard_data.lock().unwrap();
            if cb.is_empty() {
                (0, Vec::new())
            } else {
                let ptr = ctx.alloc_heap_block(cb.len() + 1).unwrap_or(0);
                (ptr, cb.clone())
            }
        };
        if ptr != 0 {
            uc.mem_write(ptr as u64, &data).unwrap();
            uc.mem_write(ptr as u64 + data.len() as u64, &[0]).unwrap();
            crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int {:#x}", format, ptr);
            return Some(ApiHookResult::callee(1, Some(ptr as i32)));
        }
    }
    crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int 0", format);
    Some(ApiHookResult::callee(1, Some(0)))
}

// API: BOOL OpenClipboard(HWND hWndNewOwner)
pub(super) fn open_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let opened = ctx
        .clipboard_open
        .swap(1, std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!(
        "[USER32] OpenClipboard({:#x}) -> BOOL {}",
        hwnd,
        if opened == 0 { 1 } else { 0 }
    );
    Some(ApiHookResult::callee(
        1,
        Some(if opened == 0 { 1 } else { 0 }),
    ))
}

// API: BOOL CloseClipboard(void)
pub(super) fn close_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    ctx.clipboard_open
        .store(0, std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] CloseClipboard() -> BOOL 1");
    Some(ApiHookResult::callee(0, Some(1)))
}

// API: BOOL EmptyClipboard(void)
pub(super) fn empty_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    ctx.clipboard_data.lock().unwrap().clear();
    crate::emu_log!("[USER32] EmptyClipboard() -> BOOL 1");
    Some(ApiHookResult::callee(0, Some(1)))
}

// API: HANDLE SetClipboardData(UINT uFormat, HANDLE hMem)
pub(super) fn set_clipboard_data(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let format = uc.read_arg(0);
    let hmem = uc.read_arg(1);
    if format == 1 && hmem != 0 {
        let mut buf = Vec::new();
        let mut curr = hmem as u64;
        loop {
            let mut tmp = [0u8; 1];
            uc.mem_read(curr, &mut tmp).unwrap();
            if tmp[0] == 0 {
                break;
            }
            buf.push(tmp[0]);
            curr += 1;
        }
        let ctx = uc.get_data();
        *ctx.clipboard_data.lock().unwrap() = buf;
        crate::emu_log!(
            "[USER32] SetClipboardData({:#x}) -> HANDLE {:#x}",
            format,
            hmem
        );
        return Some(ApiHookResult::callee(2, Some(hmem as i32)));
    }
    Some(ApiHookResult::callee(2, Some(0)))
}

// API: BOOL IsClipboardFormatAvailable(UINT format)
pub(super) fn is_clipboard_format_available(
    uc: &mut Unicorn<Win32Context>,
) -> Option<ApiHookResult> {
    let format = uc.read_arg(0);
    let available = if format == 1 {
        let ctx = uc.get_data();
        if ctx.clipboard_data.lock().unwrap().is_empty() {
            0
        } else {
            1
        }
    } else {
        0
    };
    crate::emu_log!(
        "[USER32] IsClipboardFormatAvailable({:#x}) -> BOOL {}",
        format,
        available
    );
    Some(ApiHookResult::callee(1, Some(available)))
}

// API: HWND SetCapture(HWND hWnd)
pub(super) fn set_capture(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let old = ctx
        .capture_hwnd
        .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] SetCapture({:#x}) -> HWND {:#x}", hwnd, old);
    Some(ApiHookResult::callee(1, Some(old as i32)))
}

// API: HWND GetCapture(void)
pub(super) fn get_capture(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let hwnd = ctx.capture_hwnd.load(std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] GetCapture() -> HWND {:#x}", hwnd);
    Some(ApiHookResult::callee(0, Some(hwnd as i32)))
}

// API: BOOL ReleaseCapture(void)
pub(super) fn release_capture(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    ctx.capture_hwnd
        .store(0, std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] ReleaseCapture() -> BOOL 1");
    Some(ApiHookResult::callee(0, Some(1)))
}

// API: BOOL ScreenToClient(HWND hWnd, LPPOINT lpPoint)
pub(super) fn screen_to_client(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let pt_addr = uc.read_arg(1);
    let (win_x, win_y) = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        win_event.client_screen_origin(hwnd).unwrap_or((0, 0))
    };
    let x = uc.read_u32(pt_addr as u64) as i32;
    let y = uc.read_u32(pt_addr as u64 + 4) as i32;
    uc.write_u32(pt_addr as u64, (x - win_x) as u32);
    uc.write_u32(pt_addr as u64 + 4, (y - win_y) as u32);
    crate::emu_log!(
        "[USER32] ScreenToClient({:#x}, {:#x}) -> BOOL 1",
        hwnd,
        pt_addr
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL ClientToScreen(HWND hWnd, LPPOINT lpPoint)
pub(super) fn client_to_screen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let pt_addr = uc.read_arg(1);
    let (win_x, win_y) = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        win_event.client_screen_origin(hwnd).unwrap_or((0, 0))
    };
    let x = uc.read_u32(pt_addr as u64) as i32;
    let y = uc.read_u32(pt_addr as u64 + 4) as i32;
    uc.write_u32(pt_addr as u64, (x + win_x) as u32);
    uc.write_u32(pt_addr as u64 + 4, (y + win_y) as u32);
    crate::emu_log!("[USER32] ClientToScreen({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL CreateCaret(HWND hWnd, HBITMAP hBitmap, int nWidth, int nHeight)
pub(super) fn create_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let hbitmap = uc.read_arg(1);
    let nwidth = uc.read_arg(2);
    let nheight = uc.read_arg(3);
    crate::emu_log!(
        "[USER32] CreateCaret({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        hwnd,
        hbitmap,
        nwidth,
        nheight
    );
    Some(ApiHookResult::callee(4, Some(1)))
}

// API: BOOL DestroyCaret(void)
pub(super) fn destroy_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] DestroyCaret({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(0, Some(1)))
}

// API: BOOL ShowCaret(HWND hWnd)
pub(super) fn show_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] ShowCaret({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL HideCaret(HWND hWnd)
pub(super) fn hide_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] HideCaret({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL SetCaretPos(int X, int Y)
pub(super) fn set_caret_pos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let x = uc.read_arg(0);
    let y = uc.read_arg(1);
    crate::emu_log!("[USER32] SetCaretPos({:#x}, {:#x}) -> BOOL 1", x, y);
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: SHORT GetAsyncKeyState(int vKey)
pub(super) fn get_async_key_state(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let vkey = uc.read_arg(0) as usize;
    let ctx = uc.get_data();
    let ks = ctx.key_states.lock().unwrap();
    let mut state: i32 = 0;
    if vkey < 256 && ks[vkey] {
        state = -32768; // 0x8000
    }
    crate::emu_log!(
        "[USER32] GetAsyncKeyState({:#x}) -> SHORT {:#x}",
        vkey,
        state
    );
    Some(ApiHookResult::callee(1, Some(state)))
}

// API: SHORT GetKeyState(int nVirtKey)
pub(super) fn get_key_state(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let vkey = uc.read_arg(0) as usize;
    let ctx = uc.get_data();
    let ks = ctx.key_states.lock().unwrap();
    let mut state: i32 = 0;
    if vkey < 256 && ks[vkey] {
        state = -32768; // 0x8000
    }
    crate::emu_log!("[USER32] GetKeyState({:#x}) -> SHORT {:#x}", vkey, state);
    Some(ApiHookResult::callee(1, Some(state)))
}

// API: DWORD GetSysColor(int nIndex)
pub(super) fn get_sys_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let index = uc.read_arg(0);
    let color = match index {
        5 => 0x00FFFFFF,  // COLOR_WINDOW
        8 => 0x00000000,  // COLOR_WINDOWTEXT
        15 => 0x00C0C0C0, // COLOR_BTNFACE
        _ => 0x00808080,
    };
    crate::emu_log!("[USER32] GetSysColor({:#x}) -> COLOR {:#x}", index, color);
    Some(ApiHookResult::callee(1, Some(color)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dll::win32::StackCleanup;
    use unicorn_engine::{Arch, Mode, RegisterX86, Unicorn};

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
    fn system_parameters_info_getworkarea_writes_current_work_area() {
        let mut uc = new_test_uc();
        uc.get_data().set_work_area(-8, 16, 1912, 1048);
        let rect_addr = uc.malloc(16) as u32;

        write_call_frame(&mut uc, &[SPI_GETWORKAREA, 0, rect_addr, 0]);
        let result = system_parameters_info_a(&mut uc).unwrap();

        assert_eq!(result.cleanup, StackCleanup::Callee(4));
        assert_eq!(result.return_value, Some(1));
        assert_eq!(uc.read_u32(rect_addr as u64) as i32, -8);
        assert_eq!(uc.read_u32(rect_addr as u64 + 4) as i32, 16);
        assert_eq!(uc.read_u32(rect_addr as u64 + 8) as i32, 1912);
        assert_eq!(uc.read_u32(rect_addr as u64 + 12) as i32, 1048);
        assert_eq!(uc.get_data().last_error.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn system_parameters_info_getworkarea_rejects_null_rect_pointer() {
        let mut uc = new_test_uc();

        write_call_frame(&mut uc, &[SPI_GETWORKAREA, 0, 0, 0]);
        let result = system_parameters_info_a(&mut uc).unwrap();

        assert_eq!(result.cleanup, StackCleanup::Callee(4));
        assert_eq!(result.return_value, Some(0));
        assert_eq!(
            uc.get_data().last_error.load(Ordering::SeqCst),
            ERROR_INVALID_PARAMETER
        );
    }
}
