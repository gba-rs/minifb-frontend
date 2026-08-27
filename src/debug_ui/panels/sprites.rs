use gba_emulator::gba::GBA;
use crate::debug_ui::Rect;

const CELL_SIZE: usize = 20;
const CELL_INNER: usize = 18;
const COLUMNS: usize = 12;
const OBJ_MODE_DISABLED: u8 = 0b10;

pub fn render(buf: &mut [u32], stride: usize, rect: &Rect, gba: &GBA) {
    let gpu = &gba.gpu;
    let mem_map = &gba.memory_bus.mem_map;
    let height = buf.len() / stride;

    for i in 0..128usize {
        let col = i % COLUMNS;
        let row = i / COLUMNS;
        let cell_x = rect.x + col * CELL_SIZE;
        let cell_y = rect.y + 16 + row * CELL_SIZE;
        if cell_y + CELL_SIZE > rect.y + rect.h {
            break;
        }

        if gpu.objects[i].attr0.get_obj_mode() == OBJ_MODE_DISABLED {
            continue;
        }

        let (obj_w, obj_h) = gpu.objects[i].size();
        if obj_w <= 0 || obj_h <= 0 {
            continue;
        }

        let (mut obj_x, mut obj_y) = gpu.objects[i].position();
        if obj_y >= 160 { obj_y -= 1 << 8; }
        if obj_x >= 240 { obj_x -= 1 << 9; }
        if obj_y + obj_h <= 0 || obj_y >= 160 || obj_x + obj_w <= 0 || obj_x >= 240 {
            continue;
        }

        let pixels = gpu.decode_object_pixels(i, mem_map);
        for cy in 0..CELL_INNER {
            let src_y = (cy * obj_h as usize) / CELL_INNER;
            for cx in 0..CELL_INNER {
                let src_x = (cx * obj_w as usize) / CELL_INNER;
                let color = pixels[src_y * obj_w as usize + src_x];
                if color.is_transparent() {
                    continue;
                }

                let px = cell_x + cx;
                let py = cell_y + cy;
                if px < stride && py < height {
                    buf[py * stride + px] = color.to_0rgb();
                }
            }
        }
    }
}
