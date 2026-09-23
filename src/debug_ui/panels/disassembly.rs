use gba_emulator::gba::GBA;
use gba_emulator::cpu::cpu::InstructionSet;
use crate::debug_ui::{draw, font, Rect, DebuggerState};

const LINE_HEIGHT: usize = 9;
const ROWS_BEFORE: i32 = 6;
const ROWS_AFTER: i32 = 10;

pub fn render(buf: &mut [u32], stride: usize, rect: &Rect, gba: &GBA, state: &DebuggerState) {
    let cpu = &gba.cpu;
    let pc = cpu.get_pc();
    let is_thumb = cpu.get_instruction_set() == InstructionSet::Thumb;
    let step: u32 = if is_thumb { 2 } else { 4 };

    let x = rect.x + 4;
    let mut y = rect.y + 16;

    for offset in -ROWS_BEFORE..ROWS_AFTER {
        let address = pc.wrapping_add((offset * step as i32) as u32);
        let word = if is_thumb {
            gba.memory_bus.mem_map.read_u16(address) as u32
        } else {
            gba.memory_bus.mem_map.read_u32(address)
        };
        let asm = match cpu.decode(word) {
            Ok(instr) => instr.asm(),
            Err(_) => "???".to_string(),
        };
        let is_breakpoint = state.breakpoints.contains(&address);
        let marker = if is_breakpoint { "*" } else { " " };
        let line = format!("{}{:08X} {}", marker, address, asm);

        if offset == 0 {
            draw::fill_rect(buf, stride, rect.x + 1, y - 1, rect.w - 2, LINE_HEIGHT, draw::COL_BORDER);
        }
        let color = if offset == 0 {
            draw::COL_HIGHLIGHT
        } else if is_breakpoint {
            draw::COL_BREAKPOINT
        } else {
            draw::COL_FG
        };
        font::draw_text(buf, stride, x, y, &line, color, 1);
        y += LINE_HEIGHT;
    }
}

pub fn address_for_click(rect: &Rect, gba: &GBA, mouse_x: f32, mouse_y: f32) -> Option<u32> {
    let content_y = (rect.y + 16) as f32;
    if mouse_x < rect.x as f32 || mouse_x >= (rect.x + rect.w) as f32 || mouse_y < content_y {
        return None;
    }

    let row = ((mouse_y - content_y) / LINE_HEIGHT as f32) as i32;
    let total_rows = ROWS_BEFORE + ROWS_AFTER;
    if row < 0 || row >= total_rows {
        return None;
    }

    let offset = row - ROWS_BEFORE;
    let cpu = &gba.cpu;
    let is_thumb = cpu.get_instruction_set() == InstructionSet::Thumb;
    let step: u32 = if is_thumb { 2 } else { 4 };
    Some(cpu.get_pc().wrapping_add((offset * step as i32) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gba_emulator::gba::GBA;

    fn panel() -> Rect {
        Rect { x: 240, y: 140, w: 480, h: 170 }
    }

    #[test]
    fn address_for_click_maps_the_highlighted_row_to_pc() {
        let gba = GBA::default();
        let rect = panel();
        let pc = gba.cpu.get_pc();
        let click_y = (rect.y + 16) as f32 + (ROWS_BEFORE as f32) * LINE_HEIGHT as f32 + 1.0;

        assert_eq!(address_for_click(&rect, &gba, (rect.x + 10) as f32, click_y), Some(pc));
    }

    #[test]
    fn address_for_click_outside_panel_bounds_returns_none() {
        let gba = GBA::default();
        let rect = panel();

        assert_eq!(address_for_click(&rect, &gba, (rect.x - 5) as f32, (rect.y + 20) as f32), None);
        assert_eq!(address_for_click(&rect, &gba, (rect.x + 10) as f32, (rect.y + 4) as f32), None);
    }

    #[test]
    fn address_for_click_steps_by_instruction_width() {
        let gba = GBA::default();
        let rect = panel();
        let pc = gba.cpu.get_pc();
        let is_thumb = gba.cpu.get_instruction_set() == InstructionSet::Thumb;
        let step: u32 = if is_thumb { 2 } else { 4 };
        let click_y = (rect.y + 16) as f32 + ((ROWS_BEFORE + 1) as f32) * LINE_HEIGHT as f32 + 1.0;

        assert_eq!(address_for_click(&rect, &gba, (rect.x + 10) as f32, click_y), Some(pc.wrapping_add(step)));
    }
}
