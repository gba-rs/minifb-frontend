pub const COL_BG: u32 = 0x00101018;
pub const COL_PANEL_BG: u32 = 0x00181822;
pub const COL_BORDER: u32 = 0x00404858;
pub const COL_TITLE: u32 = 0x0080C0FF;
pub const COL_FG: u32 = 0x00D0D8E0;
pub const COL_HIGHLIGHT: u32 = 0x00FFD060;
pub const COL_BREAKPOINT: u32 = 0x00FF4040;
pub const COL_DIM: u32 = 0x00707880;

pub fn fill_rect(buf: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    let height = buf.len() / stride;
    for row in y..(y + h).min(height) {
        let start = row * stride + x;
        let end = (row * stride + x + w).min(row * stride + stride);
        if start < end {
            buf[start..end].fill(color);
        }
    }
}

pub fn draw_rect(buf: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    fill_rect(buf, stride, x, y, w, 1, color);
    fill_rect(buf, stride, x, y + h - 1, w, 1, color);
    fill_rect(buf, stride, x, y, 1, h, color);
    fill_rect(buf, stride, x + w - 1, y, 1, h, color);
}

pub fn set_pixel(buf: &mut [u32], stride: usize, x: i32, y: i32, color: u32) {
    let height = (buf.len() / stride) as i32;
    if x >= 0 && y >= 0 && x < stride as i32 && y < height {
        buf[(y as usize) * stride + (x as usize)] = color;
    }
}

pub fn draw_line(buf: &mut [u32], stride: usize, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
    let (mut x0, mut y0) = (x0, y0);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        set_pixel(buf, stride, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
