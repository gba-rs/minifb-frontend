extern crate minifb;
use gba_emulator::{gamepak::GamePack, gba::GBA};
use std::{fs::OpenOptions, io::prelude::*};
use log::{Level, Metadata, Record, SetLoggerError, error, info};
use std::{collections::VecDeque, time::Instant};
use minifb::{Key, Window, WindowOptions};
use clap::{AppSettings, Clap};
use average::Mean;

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

#[derive(Clap)]
#[clap(version = "1.0", author = "gba-rs team <https://github.com/gba-rs/>")]
#[clap(setting = AppSettings::ColoredHelp)]
struct Opts {
    bios_file: String,
    rom_file: String,
    save_file: Option<String>,
    #[clap(short, long)]
    skip_bios: bool,
    #[clap(short, long)]
    frame_cap: Option<i32>,
    #[clap(short, long)]
    fps_counter: bool
}

fn main() {
    let opts: Opts = Opts::parse();
    let game_pack = GamePack::new(&opts.bios_file, &opts.rom_file);

    let mut gba = if opts.skip_bios {
        GBA::new(0x08000000, &game_pack)
    } else {
        GBA::new(0, &game_pack)
    };

    if let Some(ref save_path) = opts.save_file {
        if let Ok(mut file) = OpenOptions::new().create(true).read(true).write(true).open(&save_path) {
            let mut save_data: Vec<u8> = Vec::new();
            let read_result = file.read_to_end(&mut save_data);
            match read_result {
                Ok(_) => {
                    gba.load_save_file(&save_data);
                    info!("Loaded save file {}", &save_path);
                },
                Err(_) => error!("Error reading {} to end", &save_path),
            }
        } else {
            error!("Failed to open {}", &save_path);
        }
    } else {
        info!("No save file provided, will create");
    }

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

    // Limit frame rate
    if let Some(frame_cap) = opts.frame_cap {
        let frame_time = (1.0 / (frame_cap as f32)) * 1_000_000.0;
        window.limit_update_rate(Some(std::time::Duration::from_micros(frame_time as u64)));
    }

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
                    Key::R => gba.key_status.set_button_r(0),
                    Key::Q => gba.key_status.set_button_l(0),
                    Key::Enter => gba.key_status.set_button_start(0),
                    Key::Backspace => gba.key_status.set_button_select(0),
                    _ => ()
                }
            }
        });

        window
            .update_with_buffer(&gba.gpu.frame_buffer, WIDTH, HEIGHT)
            .unwrap();

        if opts.fps_counter {
            fps_counter_buffer.push_back(1f64 / now.elapsed().as_secs_f64());
            if fps_counter_buffer.len() == FPS_BUFFER_SIZE {
                a = fps_counter_buffer.drain(0..FPS_BUFFER_SIZE).collect();
            }
            window.set_title(&format!("GBA Emu: {} FPS", a.mean()));
        }
    }
    

    if let Some(ref save_path) = opts.save_file {
        if let Ok(mut file) = OpenOptions::new().create(true).read(true).write(true).open(&save_path) {
            let _ = file.write_all(&gba.get_save_data()[..]);
        } else {
            error!("Failed to open {}", &save_path);
        }
    }
}