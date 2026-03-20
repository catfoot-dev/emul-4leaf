pub mod common;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{Window, WindowId};

use anyhow::Result;
use std::num::NonZeroU32;
use std::sync::mpsc::{Receiver, Sender};

// 그래픽 관련
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10},
    pixelcolor::Rgb888,
    prelude::*,
    text::Text,
};
use softbuffer::{Context as SoftContext, Surface};

use crate::debug::common::{CpuContext, DebugCommand};

struct FrameBuffer<'a> {
    buffer: &'a mut [u32],
    width: u32,
}
impl<'a> DrawTarget for FrameBuffer<'a> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;
    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(Point { x, y }, color) in pixels.into_iter() {
            if x >= 0 && x < self.width as i32 {
                let offset = (y as u32 * self.width + x as u32) as usize;
                if offset < self.buffer.len() {
                    self.buffer[offset] =
                        ((color.r() as u32) << 16) | ((color.g() as u32) << 8) | (color.b() as u32);
                }
            }
        }
        Ok(())
    }
}
impl<'a> OriginDimensions for FrameBuffer<'a> {
    fn size(&self) -> Size {
        Size::new(self.width, (self.buffer.len() as u32) / self.width)
    }
}

#[derive(Default)]
pub struct Debug {
    cmd_tx: Option<Sender<DebugCommand>>,
    state_rx: Option<Receiver<CpuContext>>,
    cpu_state: Option<CpuContext>,
    waiting_for_step: bool,
    auto_running: bool,
    window: Option<Window>,
}

impl Debug {
    pub fn new(cmd_tx: Sender<DebugCommand>, state_rx: Receiver<CpuContext>) -> Self {
        Debug {
            cmd_tx: Some(cmd_tx),
            state_rx: Some(state_rx),
            cpu_state: None,
            waiting_for_step: false,
            auto_running: false,
            window: None,
        }
    }
}

impl ApplicationHandler for Debug {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.window = Some(
            event_loop
                .create_window(Window::default_attributes())
                .unwrap(),
        );
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(rx) = self.state_rx.as_ref() {
            if let Ok(new_state) = rx.try_recv() {
                self.cpu_state = Some(new_state);
                self.waiting_for_step = false;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                let window = self.window.as_ref().unwrap();
                if id != window.id() {
                    return;
                }

                let (width, height) = {
                    let size = window.inner_size();
                    (size.width, size.height)
                };

                let window = self.window.as_ref().unwrap();
                let display_handle = window.display_handle().unwrap();
                let context = SoftContext::new(display_handle).unwrap();
                let mut surface = Surface::new(&context, window).unwrap();
                if let (Some(nw), Some(nh)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
                    surface.resize(nw, nh).unwrap();
                }

                let mut buffer = surface.buffer_mut().unwrap();
                buffer.fill(0); // 화면 지우기

                // 그리기 도구 준비
                let mut display = FrameBuffer {
                    buffer: &mut buffer,
                    width,
                };
                let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE);
                let label_style = MonoTextStyle::new(&FONT_6X10, Rgb888::YELLOW);
                let hl_style = MonoTextStyle::new(&FONT_6X10, Rgb888::CYAN);
                let style_w = MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE);
                let style_y = MonoTextStyle::new(&FONT_6X10, Rgb888::YELLOW);

                if let Some(state) = self.cpu_state.as_ref() {
                    // 그리기 로직
                    // 레지스터 출력
                    let reg_names = [
                        "EAX", "EBX", "ECX", "EDX", "ESI", "EDI", "EBP", "ESP", "EIP",
                    ];
                    let mut y = 20;

                    Text::new("REGISTERS", Point::new(10, y), label_style)
                        .draw(&mut display)
                        .ok();
                    y += 15;

                    for (i, val) in state.regs.iter().enumerate() {
                        let color = if i == 8 { hl_style } else { text_style }; // EIP 강조
                        let text = format!("{}: 0x{:08x}", reg_names[i], val);
                        Text::new(&text, Point::new(10, y), color)
                            .draw(&mut display)
                            .ok();
                        y += 12;
                    }

                    // 다음 명령어 출력
                    y += 10;
                    Text::new("NEXT OP:", Point::new(10, y), label_style)
                        .draw(&mut display)
                        .ok();
                    y += 15;
                    Text::new(&state.next_instr, Point::new(10, y), hl_style)
                        .draw(&mut display)
                        .ok();

                    // 스택 뷰 출력 (오른쪽)
                    let stack_x = 200;
                    let mut stack_y = 20;
                    Text::new("STACK (TOP 10)", Point::new(stack_x, stack_y), label_style)
                        .draw(&mut display)
                        .ok();
                    stack_y += 15;

                    for (addr, val) in &state.stack {
                        let mark = if *addr == state.regs[7] { "<- ESP" } else { "" };
                        let text = format!("0x{:08x}: 0x{:08x} {}", addr, val, mark);
                        Text::new(&text, Point::new(stack_x, stack_y), text_style)
                            .draw(&mut display)
                            .ok();
                        stack_y += 12;
                    }
                    let mode_str = if self.auto_running {
                        "[AUTO-RUN]"
                    } else {
                        "[STEP]"
                    };
                    let mode_color = if self.auto_running {
                        MonoTextStyle::new(&FONT_6X10, Rgb888::new(0, 255, 128))
                    } else {
                        style_y
                    };
                    Text::new(
                        &format!(
                            "Mode: {}  |  F5: Run/Pause  |  F10: Step  |  ESC: Quit",
                            mode_str
                        ),
                        Point::new(10, 400),
                        mode_color,
                    )
                    .draw(&mut display)
                    .ok();
                } else {
                    Text::new("Waiting...", Point::new(10, 20), style_w)
                        .draw(&mut display)
                        .ok();
                }
                buffer.present().unwrap();
                // self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                if PhysicalKey::Code(KeyCode::F10) == event.physical_key && !self.waiting_for_step {
                    if let Some(tx) = self.cmd_tx.as_ref() {
                        let _ = tx.send(DebugCommand::Step);
                        self.waiting_for_step = true;
                    }
                }
                if PhysicalKey::Code(KeyCode::F5) == event.physical_key {
                    if let Some(tx) = self.cmd_tx.as_ref() {
                        if self.auto_running {
                            self.auto_running = false;
                            let _ = tx.send(DebugCommand::Pause);
                            println!("[DEBUG] UI: Pause requested");
                        } else {
                            self.auto_running = true;
                            let _ = tx.send(DebugCommand::Run);
                            println!("[DEBUG] UI: Run requested");
                        }
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
                if PhysicalKey::Code(KeyCode::Escape) == event.physical_key {
                    event_loop.exit();
                }
            }
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }
            _ => (),
        }
    }
}

pub fn create_debug_window(cmd_tx: Sender<DebugCommand>, state_rx: Receiver<CpuContext>) {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = Debug::new(cmd_tx, state_rx);
    event_loop.run_app(&mut app).unwrap();
}
