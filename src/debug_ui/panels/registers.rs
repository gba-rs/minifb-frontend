use gba_emulator::gba::GBA;
use gba_emulator::cpu::cpu::InstructionSet;
use gba_emulator::memory::memory_map::HaltState;
use crate::debug_ui::{draw, font, Rect};

const LINE_HEIGHT: usize = 9;

pub fn render(buf: &mut [u32], stride: usize, rect: &Rect, gba: &GBA) {
    let cpu = &gba.cpu;
    let x = rect.x + 4;
    let mut y = rect.y + 16;

    font::draw_text(buf, stride, x, y, &format!("PC:{:08X} MODE:{:?}", cpu.get_pc(), cpu.get_operating_mode()), draw::COL_FG, 1);
    y += LINE_HEIGHT;

    let halt_state = &gba.memory_bus.mem_map.halt_state;
    let is_thumb = cpu.get_instruction_set() == InstructionSet::Thumb;
    let halt_color = if *halt_state == HaltState::Running { draw::COL_FG } else { draw::COL_HIGHLIGHT };
    font::draw_text(buf, stride, x, y, &format!("THUMB:{} STATE:{:?}", is_thumb as u8, halt_state), halt_color, 1);
    y += LINE_HEIGHT;

    for row in 0..4 {
        let mut line = String::new();
        for col in 0..4 {
            let reg = row * 4 + col;
            line.push_str(&format!("R{:<2}{:08X} ", reg, cpu.get_register_unsafe(reg as u8)));
        }
        font::draw_text(buf, stride, x, y, &line, draw::COL_FG, 1);
        y += LINE_HEIGHT;
    }

    let flags = cpu.cpsr.flags;
    font::draw_text(
        buf, stride, x, y,
        &format!("N:{} Z:{} C:{} V:{}", flags.negative as u8, flags.zero as u8, flags.carry as u8, flags.signed_overflow as u8),
        draw::COL_FG, 1,
    );
}
