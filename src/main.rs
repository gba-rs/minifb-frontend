extern crate minifb;
use gba_emulator::gba::GBA;
use std::env;
use std::io::prelude::*;
use std::fs::File;
use log::{Record, Level, Metadata, SetLoggerError};
use log::info;
use std::time::Instant;
use std::collections::VecDeque;
use average::Mean;


use minifb::{Key, Window, WindowOptions};

const WIDTH: usize = 240;
const HEIGHT: usize = 160;
const FPS_BUFFER_SIZE: usize = 30;

pub struct ConsoleLogger;

pub static LOGGER: ConsoleLogger = ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Trace
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {

            let target = if record.target().len() > 0 {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            };

            println!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

pub fn init_logger() -> Result<(), SetLoggerError> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(Level::Trace.to_level_filter());
    Ok(())
}

fn main() {

    match init_logger() {
        Ok(_) => {
            info!("Logger initialized succesfully");
        },
        Err(_) => {
            info!("Logger failed to initialize");
        }
    }

    let args: Vec<String> = env::args().collect();

    let rom = File::open(&args[2]);
    let mut rom_bytes = Vec::new();
    rom.unwrap().read_to_end(&mut rom_bytes);

    let bios = File::open(&args[1]);
    let mut bios_bytes = Vec::new();
    bios.unwrap().read_to_end(&mut bios_bytes);

    let mut window = Window::new(
        "GBA Emulator",
        WIDTH,
        HEIGHT,
        WindowOptions{
            resize: true,
            ..WindowOptions::default()
        },
    )
    .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    // let mut gba: GBA = GBA::new(0x08000000, &bios_bytes, &rom_bytes);
    let mut gba: GBA = GBA::new(0, &bios_bytes, &rom_bytes);

    // Limit to max ~60 fps update rate
    window.limit_update_rate(Some(std::time::Duration::from_micros(16600)));

    let mut fps_counter_buffer = VecDeque::new();
    let mut a: Mean = fps_counter_buffer.iter().collect();


    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();
        gba.frame();

        gba.key_status.set_register(0xFFFF);

        window.get_keys().map(|keys| {
            for t in keys {
                match t {
                    Key::W => gba.key_status.set_dpad_up(0),
                    Key::S => gba.key_status.set_dpad_down(0),
                    Key::A => gba.key_status.set_dpad_left(0),
                    Key::D => gba.key_status.set_dpad_right(0),
                    Key::H => gba.key_status.set_button_a(0),
                    Key::J => gba.key_status.set_button_b(0),
                    Key::K => gba.key_status.set_button_r(0),
                    Key::L => gba.key_status.set_button_l(0),
                    Key::Enter => gba.key_status.set_button_start(0),
                    Key::Backspace => gba.key_status.set_button_select(0),
                    _ => ()
                }
            }
        });

        window
            .update_with_buffer(&gba.gpu.frame_buffer, WIDTH, HEIGHT)
            .unwrap();

        fps_counter_buffer.push_back(1f64 / now.elapsed().as_secs_f64());
        if fps_counter_buffer.len() == FPS_BUFFER_SIZE {
            a = fps_counter_buffer.drain(0..FPS_BUFFER_SIZE).collect();
        }

        window.set_title(&format!("GBA Emu: {} FPS", a.mean()));
    }
}