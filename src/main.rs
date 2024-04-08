extern crate minifb;
use gba_emulator::{gamepak::GamePack, gba::GBA};
use gilrs::{Button, Event, Gilrs};
use std::{fs::OpenOptions, io::prelude::*};
use log::{Level, Metadata, Record, SetLoggerError, error, info};
use std::{collections::VecDeque, time::Instant};
use minifb::{Key, Window, WindowOptions, ScaleMode};
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
    res_width: Option<usize>,
    res_height: Option<usize>,
    #[clap(short, long)]
    skip_bios: bool,
    #[clap(short, long)]
    frame_cap: Option<i32>,
    #[clap(short, long)]
    fps_counter: bool
}

fn read_save_file(gba: &mut GBA, save_path: &String) {
    if let Ok(mut file) = OpenOptions::new().create(true).read(true).write(true).open(save_path) {
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
}

fn write_save_file(gba: &mut GBA, save_path: &String) {
    if let Ok(mut file) = OpenOptions::new().create(true).read(true).write(true).open(&save_path) {
        let _ = file.write_all(&gba.get_save_data()[..]);
    } else {
        error!("Failed to open {}", &save_path);
    }
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

    let opts: Opts = Opts::parse();
    let mut gilrs = Gilrs::new().unwrap();
    let mut game_pack = GamePack::new(&opts.bios_file, &opts.rom_file);
    game_pack.read_title();
    let mut active_gamepad = None;

    let mut gba = if opts.skip_bios {
        GBA::new(0x08000000, &game_pack)
    } else {
        GBA::new(0, &game_pack)
    };

    if let Some(ref save_path) = opts.save_file {
        read_save_file(&mut gba, save_path);
    } else {
        info!("No save file provided");
    }

    let width = if let Some(res_width) = opts.res_width {
        res_width
    } else {
        WIDTH
    };

    let height = if let Some(res_height) = opts.res_height {
        res_height
    } else {
        HEIGHT
    };

    let mut window = Window::new(
        &game_pack.title,
        width,
        height,
        WindowOptions{
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
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
        if gba.memory_bus.mem_map.new_save_data {
            info!("Got new save data, saving");
            if let Some(ref save_path) = opts.save_file {
                write_save_file(&mut gba, save_path);
            } else {
                info!("No save file provided");
            }
            gba.memory_bus.mem_map.new_save_data = false;
        }

        gba.key_status.set_register(0xFFFF);

        // poll for any gamepad input events
        while let Some(Event { id, event, time }) = gilrs.next_event() {
            active_gamepad = Some(id);
        }

        if let Some(gamepad) = active_gamepad.map(|id| gilrs.gamepad(id)) {
            if gamepad.is_pressed(Button::DPadUp) { gba.key_status.set_dpad_up(0); }
            if gamepad.is_pressed(Button::DPadDown) { gba.key_status.set_dpad_down(0); }
            if gamepad.is_pressed(Button::DPadLeft) { gba.key_status.set_dpad_left(0); }
            if gamepad.is_pressed(Button::DPadRight) { gba.key_status.set_dpad_right(0); }
            if gamepad.is_pressed(Button::South) { gba.key_status.set_button_a(0); }
            if gamepad.is_pressed(Button::East) { gba.key_status.set_button_b(0); }
            if gamepad.is_pressed(Button::RightTrigger) { gba.key_status.set_button_r(0); }
            if gamepad.is_pressed(Button::LeftTrigger) { gba.key_status.set_button_l(0); }
            if gamepad.is_pressed(Button::Select) { gba.key_status.set_button_select(0); }
            if gamepad.is_pressed(Button::Start) { gba.key_status.set_button_start(0); }
        }
        
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
    

}
