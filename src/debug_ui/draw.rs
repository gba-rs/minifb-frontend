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
