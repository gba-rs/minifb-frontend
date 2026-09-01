use crate::debug_ui::{draw, Rect};

const COL_LEFT: u32 = 0x0060D0FF;
const COL_RIGHT: u32 = 0x00FF8040;
const COL_CENTER: u32 = 0x00303840;

pub fn render(buf: &mut [u32], stride: usize, rect: &Rect, samples: &[i16]) {
    let plot_x = rect.x + 2;
    let plot_y = rect.y + 16;
    let plot_w = rect.w.saturating_sub(4);
    let plot_h = rect.h.saturating_sub(20);
    if plot_w == 0 || plot_h == 0 {
        return;
    }

    let mid_y = plot_y + plot_h / 2;
    draw::draw_line(buf, stride, plot_x as i32, mid_y as i32, (plot_x + plot_w) as i32, mid_y as i32, COL_CENTER);

    let frame_count = samples.len() / 2;
    if frame_count < 2 {
        return;
    }

    plot_channel(buf, stride, samples, 0, frame_count, plot_x, plot_y, plot_w, plot_h, COL_LEFT);
    plot_channel(buf, stride, samples, 1, frame_count, plot_x, plot_y, plot_w, plot_h, COL_RIGHT);
}

fn plot_channel(
    buf: &mut [u32],
    stride: usize,
    samples: &[i16],
    channel_offset: usize,
    frame_count: usize,
    plot_x: usize,
    plot_y: usize,
    plot_w: usize,
    plot_h: usize,
    color: u32,
) {
    let half_h = (plot_h / 2) as f32;
    let mut prev: Option<(i32, i32)> = None;
    for col in 0..plot_w {
        let frame_idx = col * frame_count / plot_w;
        let sample = samples[frame_idx * 2 + channel_offset] as f32;
        let normalized = sample / (i16::MAX as f32);
        let x = (plot_x + col) as i32;
        let y = (plot_y as f32 + half_h - normalized * half_h) as i32;
        if let Some((px, py)) = prev {
            draw::draw_line(buf, stride, px, py, x, y, color);
        }
        prev = Some((x, y));
    }
}
