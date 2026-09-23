pub mod draw;
pub mod font;
pub mod panels;

use gba_emulator::gba::GBA;
use std::collections::HashSet;

pub const TOTAL_WIDTH: usize = 960;
pub const TOTAL_HEIGHT: usize = 576;
pub const SCALE: minifb::Scale = minifb::Scale::X2;

pub const GAME_X: usize = 0;
pub const GAME_Y: usize = 0;

pub const CPU_PANEL: Rect = Rect { x: 240, y: 0, w: 480, h: 140 };
pub const DISASM_PANEL: Rect = Rect { x: 240, y: 140, w: 480, h: 170 };
pub const MEMORY_PANEL: Rect = Rect { x: 240, y: 310, w: 480, h: 170 };
pub const SPRITES_PANEL: Rect = Rect { x: 0, y: 160, w: 240, h: 320 };
pub const TILES_PANEL: Rect = Rect { x: 720, y: 0, w: 240, h: 240 };
pub const BACKGROUNDS_PANEL: Rect = Rect { x: 720, y: 240, w: 240, h: 240 };
pub const SOUND_PANEL: Rect = Rect { x: 0, y: 496, w: 960, h: 68 };

pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

pub struct DebuggerState {
    pub enabled: bool,
    pub breakpoints: HashSet<u32>,
    pub memory_base: u32,
    pub memory_region: usize,
    pub recent_audio_samples: Vec<i16>,
}

impl Default for DebuggerState {
    fn default() -> Self {
        DebuggerState {
            enabled: false,
            breakpoints: HashSet::new(),
            memory_base: panels::memory::REGIONS[panels::memory::DEFAULT_REGION].1,
            memory_region: panels::memory::DEFAULT_REGION,
            recent_audio_samples: Vec::new(),
        }
    }
}

fn draw_panel(buf: &mut [u32], stride: usize, rect: &Rect, title: &str) {
    draw::fill_rect(buf, stride, rect.x, rect.y, rect.w, rect.h, draw::COL_PANEL_BG);
    draw::draw_rect(buf, stride, rect.x, rect.y, rect.w, rect.h, draw::COL_BORDER);
    font::draw_text(buf, stride, rect.x + 4, rect.y + 4, title, draw::COL_TITLE, 1);
}

pub fn render(buf: &mut [u32], stride: usize, state: &DebuggerState, gba: Option<&GBA>) {
    draw::fill_rect(buf, stride, 0, 0, stride, buf.len() / stride, draw::COL_BG);
    draw::fill_rect(buf, stride, GAME_X, GAME_Y, 240, 160, 0x00000000);
    draw_panel(buf, stride, &CPU_PANEL, "CPU");
    draw_panel(buf, stride, &DISASM_PANEL, "DISASSEMBLY");
    draw_panel(buf, stride, &MEMORY_PANEL, "MEMORY");
    draw_panel(buf, stride, &SPRITES_PANEL, "SPRITES");
    draw_panel(buf, stride, &TILES_PANEL, "TILES");
    draw_panel(buf, stride, &BACKGROUNDS_PANEL, "BACKGROUNDS");
    draw_panel(buf, stride, &SOUND_PANEL, "SOUND");

    if let Some(gba) = gba {
        panels::registers::render(buf, stride, &CPU_PANEL, gba);
        panels::disassembly::render(buf, stride, &DISASM_PANEL, gba, state);
        let region_name = panels::memory::REGIONS[state.memory_region].0;
        panels::memory::render(buf, stride, &MEMORY_PANEL, gba, state.memory_base, region_name);
        panels::sprites::render(buf, stride, &SPRITES_PANEL, gba);
        panels::tiles::render(buf, stride, &TILES_PANEL, gba);
        panels::backgrounds::render(buf, stride, &BACKGROUNDS_PANEL, gba);
    }
    panels::sound::render(buf, stride, &SOUND_PANEL, &state.recent_audio_samples);

    let hint = "F5:CONTINUE  F9:BREAKPOINT  F10:STEP  CLICK DISASM:BREAKPOINT  []:REGION  UP/DN:SCROLL  PGUP/PGDN:PAGE";
    font::draw_text(buf, stride, 4, TOTAL_HEIGHT - 12, hint, draw::COL_DIM, 1);
}
