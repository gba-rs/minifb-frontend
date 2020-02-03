extern crate minifb;
use gba_emulator::gba::GBA;
use std::env;
use std::io::prelude::*;
use std::fs::File;

use minifb::{Key, Window, WindowOptions};

const WIDTH: usize = 240;
const HEIGHT: usize = 160;

fn main() {
    let args: Vec<String> = env::args().collect();

    let rom = File::open(&args[2]);
    let mut rom_bytes = Vec::new();
    rom.unwrap().read_to_end(&mut rom_bytes);

    let bios = File::open(&args[1]);
    let mut bios_bytes = Vec::new();
    bios.unwrap().read_to_end(&mut bios_bytes);

    let mut buffer: Vec<u32> = vec![0; WIDTH * HEIGHT];

    let mut window = Window::new(
        "GBA Emulator",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )
    .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    let mut gba: GBA = GBA::new(0x08000000, &bios_bytes, &rom_bytes);

    // Limit to max ~60 fps update rate
    window.limit_update_rate(Some(std::time::Duration::from_micros(16600)));

    while window.is_open() && !window.is_key_down(Key::Escape) {
        gba.frame();

        // We unwrap here as we want this code to exit if it fails. Real applications may want to handle this in a different way
        window
            .update_with_buffer(&gba.gpu.frame_buffer, WIDTH, HEIGHT)
            .unwrap();
    }
}