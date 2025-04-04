extern crate minifb;
use gba_emulator::{cpu::cpu::CPU, gamepak::{self, GamePack}, gba::GBA};
use gilrs::{Button, Event, Gilrs};
use std::{fs::OpenOptions, io::prelude::*};
use log::{Level, Metadata, Record, SetLoggerError, error, info};
use std::{collections::VecDeque, time::Instant};
use minifb::{Key, Window, WindowOptions};
use average::Mean;
use clap::Parser;


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

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Opts {
    bios_file: String,
    rom_file: String,
    save_file: Option<String>,
    #[arg(short, long)]
    skip_bios: bool,
    #[arg(short, long)]
    frame_cap: Option<usize>,
    #[arg(short, long)]
    fps_counter: bool,
    #[arg(short, long)]
    save_state: Option<String>
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

fn read_save_state(save_path: &String, gba_pc: u32, game_pack: &GamePack) -> GBA {
    if let Ok(mut file) = OpenOptions::new().create(false).read(true).write(true).open(&save_path) {
        let mut binary_from_file = Vec::new();
        let _ = file.read_to_end(&mut binary_from_file).unwrap();
        let mut gba: GBA = bincode::deserialize(&binary_from_file).expect("Failed to deserialize");
        gba.register_memory();
        gba.load_bios(&game_pack.bios);
        gba.load_rom(&game_pack.rom);

        return gba;
    } else {
        error!("Failed to open {}", &save_path);
    }
    return GBA::new(gba_pc, &game_pack);
}

fn write_save_state(gba: &mut GBA, save_path: &String) {
    if let Ok(mut file) = OpenOptions::new().create(true).read(true).write(true).open(&save_path) {
        let binary = bincode::serialize(&gba).unwrap();
        let _ = file.write_all(&binary);
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
    let game_pack = GamePack::new(&opts.bios_file, &opts.rom_file);
    let mut active_gamepad = None;

    let gba_pc = if opts.skip_bios {
        0x08000000
    } else {
        0x0
    };

    let mut gba = if let Some(ref save_path) = opts.save_state {
        read_save_state(save_path, gba_pc, &game_pack) 
    } else {
        GBA::new(gba_pc, &game_pack)
    };

    if let Some(ref save_path) = opts.save_file {
        read_save_file(&mut gba, save_path);
    } else {
        info!("No save file provided");
    }

    let mut window = Window::new(
        "GBA Emulator",
        WIDTH,
        HEIGHT,
        WindowOptions{
            resize: true,
            scale: minifb::Scale::X8,
            ..WindowOptions::default()
        },
    )
    .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    // Limit frame rate
    if let Some(frame_cap) = opts.frame_cap {
        window.set_target_fps(frame_cap);
    }

    let mut fps_counter_buffer = VecDeque::new();
    let mut a: Mean = fps_counter_buffer.iter().collect();

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();
        gba.frame();

        gba.key_status.set_register(0xFFFF);

        // poll for any gamepad input events
        while let Some(Event { id, ..}) = gilrs.next_event() {
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
        
        window.get_keys().iter().for_each(|key| {
            match key {
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
        write_save_file(&mut gba, save_path);
    } else {
        info!("No save file provided");
    }

    if let Some(ref save_path) = opts.save_state {
        write_save_state(&mut gba, save_path);
    } else {
        info!("No save state file provided");
    }
}
