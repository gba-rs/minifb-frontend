use gba_emulator::gba::GBA;
use gba_emulator::memory::memory_map::{
    ON_BOARD_WRAM_START, ON_CHIP_WRAM_START, PALETTE_RAM_START,
    VIDEO_RAM_START, OBJECT_ATTRIBUTES_START, ROM_START, SRAM_START,
};
use crate::debug_ui::{draw, font, Rect};

pub const ROW_BYTES: u32 = 16;
pub const VISIBLE_ROWS: u32 = 16;
pub const DEFAULT_REGION: usize = 6;

pub const REGIONS: [(&str, u32); 8] = [
    ("BIOS", 0x0000_0000),
    ("EWRAM", ON_BOARD_WRAM_START),
    ("IWRAM", ON_CHIP_WRAM_START),
    ("PALETTE", PALETTE_RAM_START),
    ("VRAM", VIDEO_RAM_START),
    ("OAM", OBJECT_ATTRIBUTES_START),
    ("ROM", ROM_START),
    ("SRAM", SRAM_START),
];

const LINE_HEIGHT: usize = 9;

pub fn render(buf: &mut [u32], stride: usize, rect: &Rect, gba: &GBA, base: u32, region_name: &str) {
    let mem = &gba.memory_bus.mem_map;
    let x = rect.x + 4;
    let mut y = rect.y + 16;

    font::draw_text(buf, stride, x, y, &format!("[{}] BASE:{:08X}", region_name, base), draw::COL_TITLE, 1);
    y += LINE_HEIGHT;

    for row in 0..VISIBLE_ROWS {
        let row_addr = base.wrapping_add(row * ROW_BYTES);
        let mut line = format!("{:08X}: ", row_addr);
        let mut ascii = String::new();
        for col in 0..ROW_BYTES {
            let byte = mem.read_u8(row_addr.wrapping_add(col));
            line.push_str(&format!("{:02X} ", byte));
            let c = byte as char;
            ascii.push(if c.is_ascii_graphic() { c } else { '.' });
        }
        line.push(' ');
        line.push_str(&ascii);

        font::draw_text(buf, stride, x, y, &line, draw::COL_FG, 1);
        y += LINE_HEIGHT;
    }
}
