use gba_emulator::gba::GBA;
use gba_emulator::memory::memory_map::{PALETTE_RAM_START, PALETTE_RAM_SIZE};
use gba_emulator::memory::lcd_io_registers::PixelFormat;
use gba_emulator::gpu::rgb15::Rgb15;
use crate::debug_ui::Rect;

const TILE_SIZE: usize = 8;

pub fn render(buf: &mut [u32], stride: usize, rect: &Rect, gba: &GBA) {
    let gpu = &gba.gpu;
    let mem_map = &gba.memory_bus.mem_map;
    let height = buf.len() / stride;

    let bg_number = (0..4)
        .find(|&i| gpu.display_control.should_display(i))
        .unwrap_or(0);
    let control = &gpu.backgrounds[bg_number as usize].control;
    let tileset_location = control.get_tileset_location();
    let pixel_format = control.get_pixel_format();
    let bytes_per_tile = control.get_tilesize();

    let origin_x = rect.x + 2;
    let origin_y = rect.y + 16;
    let columns = (rect.w.saturating_sub(4)) / TILE_SIZE;
    let rows = (rect.h.saturating_sub(18)) / TILE_SIZE;
    if columns == 0 || rows == 0 {
        return;
    }

    let vram_char_block_size = 0x4000u32;
    let tile_count = (vram_char_block_size / bytes_per_tile).min((columns * rows) as u32);

    for tile_index in 0..tile_count {
        let col = (tile_index as usize) % columns;
        let row = (tile_index as usize) / columns;
        if row >= rows {
            break;
        }
        let cell_x = origin_x + col * TILE_SIZE;
        let cell_y = origin_y + row * TILE_SIZE;

        let tile_address = tileset_location + tile_index * bytes_per_tile;
        for py in 0..TILE_SIZE {
            for px in 0..TILE_SIZE {
                let pixel_index = match pixel_format {
                    PixelFormat::EightBit => {
                        let addr = tile_address + (8 * py as u32 + px as u32);
                        mem_map.memory[addr as usize].get()
                    }
                    PixelFormat::FourBit => {
                        let addr = tile_address + (4 * py as u32 + (px as u32 / 2));
                        let value = mem_map.memory[addr as usize].get();
                        if px & 1 != 0 { value >> 4 } else { value & 0xf }
                    }
                } as u32;

                let color = if pixel_index == 0 {
                    Rgb15::new(0x8000)
                } else {
                    let palette_ram_index = 2 * pixel_index;
                    let raw_addr = palette_ram_index + 0x500_0000u32;
                    let masked_addr = (raw_addr & PALETTE_RAM_SIZE) + PALETTE_RAM_START;
                    let idx = masked_addr as usize;
                    let value = u16::from_le_bytes([mem_map.memory[idx].get(), mem_map.memory[idx + 1].get()]);
                    Rgb15::new(value)
                };

                if color.is_transparent() {
                    continue;
                }

                let out_x = cell_x + px;
                let out_y = cell_y + py;
                if out_x < stride && out_y < height {
                    buf[out_y * stride + out_x] = color.to_0rgb();
                }
            }
        }
    }
}
