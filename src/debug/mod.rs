pub mod common;

use winit::application::ApplicationHandler;
use winit::error::EventLoopError;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, DisplayHandle};
use std::sync::Arc;
use winit::window::{Window, WindowId};

use anyhow::{Context, Result};
use goblin::pe::PE;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

// 그래픽 관련
use softbuffer::{Context as SoftContext, Surface};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    text::Text,
    primitives::{PrimitiveStyle, Rectangle},
};

use crate::debug::common::{CpuContext, DebugCommand};

struct FrameBuffer<'a> {
    buffer: &'a mut [u32],
    width: u32,
}
impl<'a> DrawTarget for FrameBuffer<'a> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;
    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where I: IntoIterator<Item = Pixel<Self::Color>> {
        for Pixel(Point { x, y }, color) in pixels.into_iter() {
            if x >= 0 && x < self.width as i32 {
                let offset = (y as u32 * self.width + x as u32) as usize;
                if offset < self.buffer.len() {
                    self.buffer[offset] = ((color.r() as u32) << 16) | ((color.g() as u32) << 8) | (color.b() as u32);
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
    window: Option<Window>,
}

impl Debug {
    pub fn new(cmd_tx: Sender<DebugCommand>, state_rx: Receiver<CpuContext>) -> Self {
        Debug {
            cmd_tx: Some(cmd_tx),
            state_rx: Some(state_rx),
            cpu_state: None,
            waiting_for_step: false,
            window: None,
        }
    }
}

impl ApplicationHandler for Debug {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.window = Some(event_loop.create_window(Window::default_attributes()).unwrap());
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Ok(new_state) = self.state_rx.as_ref().unwrap().try_recv() {
            self.cpu_state = Some(new_state);
            self.waiting_for_step = false;
            self.window.as_ref().unwrap().request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                let window = self.window.as_ref().unwrap();
                if id != window.id() { return; }

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
                let mut display = FrameBuffer { buffer: &mut buffer, width };
                let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE);
                let label_style = MonoTextStyle::new(&FONT_6X10, Rgb888::YELLOW);
                let hl_style = MonoTextStyle::new(&FONT_6X10, Rgb888::CYAN);
                let style_w = MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE);
                let style_y = MonoTextStyle::new(&FONT_6X10, Rgb888::YELLOW);

                if let Some(state) = self.cpu_state.as_ref() {
                    // 그리기 로직
                    // 레지스터 출력
                    let reg_names = ["EAX", "EBX", "ECX", "EDX", "ESI", "EDI", "EBP", "ESP", "EIP"];
                    let mut y = 20;
                    
                    Text::new("REGISTERS", Point::new(10, y), label_style).draw(&mut display).ok();
                    y += 15;

                    for (i, val) in state.regs.iter().enumerate() {
                        let color = if i == 8 { hl_style } else { text_style }; // EIP 강조
                        let text = format!("{}: 0x{:08x}", reg_names[i], val);
                        Text::new(&text, Point::new(10, y), color).draw(&mut display).ok();
                        y += 12;
                    }

                    // 다음 명령어 출력
                    y += 10;
                    Text::new("NEXT OP:", Point::new(10, y), label_style).draw(&mut display).ok();
                    y += 15;
                    Text::new(&state.next_instr, Point::new(10, y), hl_style).draw(&mut display).ok();

                    // 스택 뷰 출력 (오른쪽)
                    let stack_x = 200;
                    let mut stack_y = 20;
                    Text::new("STACK (TOP 10)", Point::new(stack_x, stack_y), label_style).draw(&mut display).ok();
                    stack_y += 15;

                    for (addr, val) in &state.stack {
                        let mark = if *addr == state.regs[7] { "<- ESP" } else { "" };
                        let text = format!("0x{:08x}: 0x{:08x} {}", addr, val, mark);
                        Text::new(&text, Point::new(stack_x, stack_y), text_style).draw(&mut display).ok();
                        stack_y += 12;
                    }
                    Text::new("Press F10 to Step", Point::new(10, 400), style_y).draw(&mut display).ok();
                } else {
                    Text::new("Waiting...", Point::new(10, 20), style_w).draw(&mut display).ok();
                }
                buffer.present().unwrap();
                // self.window.as_ref().unwrap().request_redraw();
            },
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed { return; }
                if PhysicalKey::Code(KeyCode::F10) == event.physical_key && !self.waiting_for_step {
                    self.cmd_tx.as_ref().unwrap().send(DebugCommand::Step).unwrap();
                    self.waiting_for_step = true;
                }
                if PhysicalKey::Code(KeyCode::Escape) == event.physical_key {
                    event_loop.exit();
                }
            }
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            },
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
