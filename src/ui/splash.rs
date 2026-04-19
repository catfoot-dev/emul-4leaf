use std::thread;

use goblin::pe::PE;

pub fn load_splash_data(path: &std::path::Path) -> Option<(Vec<u32>, u32, u32)> {
    let exe_path = path.join("4Leaf.exe");

    let buffer = match std::fs::read(&exe_path) {
        Ok(b) => b,
        Err(_) => {
            println!("[Splash] 4Leaf.exe not found at {:?}", exe_path);
            return None;
        }
    };

    let pe = match PE::parse(&buffer) {
        Ok(p) => p,
        Err(e) => {
            println!("[Splash] Failed to parse PE: {:?}", e);
            return None;
        }
    };

    let mut bitmap_data: Option<&[u8]> = None;

    if let Some(opt) = pe.header.optional_header
        && let Some(resource_dir_info) = opt.data_directories.get_resource_table()
    {
        let r_rva = resource_dir_info.virtual_address;
        let r_size = resource_dir_info.size;
        let r_offset = rva_to_offset(&pe, r_rva);

        if r_offset != 0 && r_offset + r_size as usize <= buffer.len() {
            let res_section = &buffer[r_offset..r_offset + r_size as usize];

            // Manual parsing of Resource Directory
            let find_resource = |section: &[u8], type_id: u32| -> Option<u32> {
                if section.len() < 16 {
                    return None;
                }
                let named_entries = u16::from_le_bytes(section[12..14].try_into().ok()?);
                let id_entries = u16::from_le_bytes(section[14..16].try_into().ok()?);
                let total_entries = named_entries + id_entries;

                for i in 0..total_entries {
                    let entry_offset = 16 + (i as usize * 8);
                    if section.len() < entry_offset + 8 {
                        break;
                    }
                    let name_or_id = u32::from_le_bytes(
                        section[entry_offset..entry_offset + 4].try_into().ok()?,
                    );
                    let offset_to_data = u32::from_le_bytes(
                        section[entry_offset + 4..entry_offset + 8]
                            .try_into()
                            .ok()?,
                    );

                    if name_or_id == type_id || type_id == 0xFFFF_FFFF {
                        return Some(offset_to_data);
                    }
                }
                None
            };

            // Root -> Type 2 (RT_BITMAP)
            if let Some(type_entry) = find_resource(res_section, 2)
                && type_entry & 0x8000_0000 != 0
            {
                let type_dir_offset = (type_entry & 0x7FFF_FFFF) as usize;
                if type_dir_offset < res_section.len() {
                    // Type -> Name/ID (any)
                    if let Some(name_entry) =
                        find_resource(&res_section[type_dir_offset..], 0xFFFF_FFFF)
                        && name_entry & 0x8000_0000 != 0
                    {
                        let name_dir_offset = (name_entry & 0x7FFF_FFFF) as usize;
                        if name_dir_offset < res_section.len() {
                            // Name -> Language (any)
                            if let Some(lang_entry) =
                                find_resource(&res_section[name_dir_offset..], 0xFFFF_FFFF)
                                && lang_entry & 0x8000_0000 == 0
                            {
                                let data_entry_offset = lang_entry as usize;
                                if data_entry_offset + 16 <= res_section.len() {
                                    let data_rva = u32::from_le_bytes(
                                        res_section[data_entry_offset..data_entry_offset + 4]
                                            .try_into()
                                            .unwrap(),
                                    );
                                    let data_size = u32::from_le_bytes(
                                        res_section[data_entry_offset + 4..data_entry_offset + 8]
                                            .try_into()
                                            .unwrap(),
                                    );
                                    let file_offset = rva_to_offset(&pe, data_rva);
                                    if file_offset != 0
                                        && file_offset + data_size as usize <= buffer.len()
                                    {
                                        bitmap_data = Some(
                                            &buffer[file_offset..file_offset + data_size as usize],
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(data) = bitmap_data {
        decode_bitmap_resource(data)
    } else {
        None
    }
}

fn rva_to_offset(pe: &PE, rva: u32) -> usize {
    for section in &pe.sections {
        if rva >= section.virtual_address && rva < section.virtual_address + section.virtual_size {
            return (rva - section.virtual_address + section.pointer_to_raw_data) as usize;
        }
    }
    0
}

fn decode_bitmap_resource(data: &[u8]) -> Option<(Vec<u32>, u32, u32)> {
    if data.len() < 40 {
        return None;
    }

    // BITMAPINFOHEADER starting from offset 0
    let bi_size = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let width = i32::from_le_bytes(data[4..8].try_into().unwrap());
    let height = i32::from_le_bytes(data[8..12].try_into().unwrap());
    let bit_count = u16::from_le_bytes(data[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(data[16..20].try_into().unwrap());

    if bi_size < 40 || compression != 0 {
        // Support only BI_RGB for now
        return None;
    }

    let abs_width = width.unsigned_abs();
    let abs_height = height.unsigned_abs();

    let mut pixels = vec![0u32; (abs_width * abs_height) as usize];

    // Data follows the header (and optional palette)
    // For 24-bit and 32-bit, usually no palette
    let offset = bi_size as usize;

    if bit_count == 16 {
        let row_stride = (abs_width * 2 + 3) & !3; // Align to 4 bytes
        for y in 0..abs_height {
            let src_y = if height > 0 { abs_height - 1 - y } else { y };
            let src_row_start = offset + (src_y * row_stride) as usize;
            for x in 0..abs_width {
                let p = src_row_start + (x * 2) as usize;
                if p + 1 < data.len() {
                    let val = u16::from_le_bytes(data[p..p + 2].try_into().unwrap());
                    // 5-5-5 format: X RRRRR GGGGG BBBBB
                    let r5 = (val >> 10) & 0x1F;
                    let g5 = (val >> 5) & 0x1F;
                    let b5 = val & 0x1F;

                    // Scale to 8-bit: (val * 255) / 31 or simply val << 3 | val >> 2
                    let r = (r5 << 3) | (r5 >> 2);
                    let g = (g5 << 3) | (g5 >> 2);
                    let b = (b5 << 3) | (b5 >> 2);

                    pixels[(y * abs_width + x) as usize] =
                        (0xFF << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                }
            }
        }
    } else if bit_count == 24 {
        let row_stride = (abs_width * 3 + 3) & !3; // Align to 4 bytes
        for y in 0..abs_height {
            let src_y = if height > 0 { abs_height - 1 - y } else { y };
            let src_row_start = offset + (src_y * row_stride) as usize;
            for x in 0..abs_width {
                let p = src_row_start + (x * 3) as usize;
                if p + 2 < data.len() {
                    let b = data[p] as u32;
                    let g = data[p + 1] as u32;
                    let r = data[p + 2] as u32;
                    pixels[(y * abs_width + x) as usize] = (r << 16) | (g << 8) | b;
                }
            }
        }
    } else if bit_count == 32 {
        let row_stride = abs_width * 4;
        for y in 0..abs_height {
            let src_y = if height > 0 { abs_height - 1 - y } else { y };
            let src_row_start = offset + (src_y * row_stride) as usize;
            for x in 0..abs_width {
                let p = src_row_start + (x * 4) as usize;
                if p + 3 < data.len() {
                    let b = data[p] as u32;
                    let g = data[p + 1] as u32;
                    let r = data[p + 2] as u32;
                    let a = data[p + 3] as u32;
                    pixels[(y * abs_width + x) as usize] = (a << 24) | (r << 16) | (g << 8) | b;
                }
            }
        }
    } else {
        return None;
    }

    Some((pixels, abs_width, abs_height))
}

pub struct SplashPainter {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub receiver: std::sync::mpsc::Receiver<()>,
    pub should_close: bool,
}

impl crate::ui::Painter for SplashPainter {
    fn create_window(
        &self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> winit::window::Window {
        let attributes = crate::ui::apply_platform_window_attributes(
            winit::window::Window::default_attributes()
                .with_title("4Leaf Emulator")
                .with_inner_size(winit::dpi::PhysicalSize::new(self.width, self.height))
                .with_resizable(false)
                .with_decorations(false)
                .with_visible(true),
        );

        let window = event_loop.create_window(attributes).unwrap();

        // 윈도우를 화면 중앙에 배치
        if let Some(monitor) = window.primary_monitor() {
            let monitor_size = monitor.size();
            let window_size = window.outer_size();
            let x = (monitor_size.width as i32 - window_size.width as i32) / 2;
            let y = (monitor_size.height as i32 - window_size.height as i32) / 2;
            window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        }

        window
    }

    fn quit_on_close(&self) -> bool {
        false
    }

    fn should_close(&self) -> bool {
        self.should_close
    }

    fn paint(&mut self, buffer: &mut [u32], width: u32, height: u32) -> bool {
        let copy_w = width.min(self.width);
        let copy_h = height.min(self.height);

        for y in 0..copy_h {
            for x in 0..copy_w {
                buffer[(y * width + x) as usize] = self.pixels[(y * self.width + x) as usize];
            }
        }

        true
    }

    fn handle_event(
        &mut self,
        _event: &winit::event::WindowEvent,
        _event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> bool {
        false
    }

    fn tick(&mut self) -> bool {
        if !self.should_close && self.receiver.try_recv().is_ok() {
            thread::sleep(std::time::Duration::from_secs(1));
            self.should_close = true;
            return true;
        }
        false
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        if self.should_close {
            None
        } else {
            Some(std::time::Duration::from_millis(10))
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
