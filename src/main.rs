extern crate minifb;
use gba_emulator::{gamepak::GamePack, gba::GBA};
use gilrs::{Button, Event, Gilrs};
use std::{fs::{File, OpenOptions}, io::prelude::*};
use std::sync::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use log::{Level, Metadata, Record, SetLoggerError, error, info};
use std::{collections::VecDeque, time::Instant};
use std::path::{Path, PathBuf};
use minifb::{Key, Window, WindowOptions, KeyRepeat, MouseButton, MouseMode};
use average::Mean;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapRb, HeapConsumer};
use muda::{Menu, Submenu, MenuItem, CheckMenuItem, MenuEvent, PredefinedMenuItem, IsMenuItem};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

mod debug_ui;
use debug_ui::DebuggerState;


const WIDTH: usize = 240;
const HEIGHT: usize = 160;
const FPS_BUFFER_SIZE: usize = 30;
const LOG_FILE_PATH: &str = "gba_emulator.log";

pub struct ConsoleLogger;

pub static LOGGER: ConsoleLogger = ConsoleLogger;
static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

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

            if let Ok(mut guard) = LOG_FILE.lock() {
                if let Some(file) = guard.as_mut() {
                    let _ = writeln!(file, "{}", record.args());
                    let _ = file.flush();
                }
            }
        }
    }

    fn flush(&self) {}
}

pub fn init_logger() -> Result<(), SetLoggerError> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(Level::Trace.to_level_filter());

    match File::create(LOG_FILE_PATH) {
        Ok(file) => {
            if let Ok(mut guard) = LOG_FILE.lock() {
                *guard = Some(file);
            }
        },
        Err(e) => {
            eprintln!("Failed to open log file {}: {}", LOG_FILE_PATH, e);
        }
    }

    Ok(())
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Opts {
    bios_file: Option<String>,
    rom_file: Option<String>,
    save_file: Option<String>,
    #[arg(short = 'b', long)]
    skip_bios: bool,
    #[arg(short = 'c', long)]
    frame_cap: Option<usize>,
    #[arg(short = 'f', long)]
    fps_counter: bool,
    #[arg(short = 's', long)]
    save_state: Option<String>,
    #[arg(long)]
    headless_frames: Option<usize>,
    #[arg(long)]
    dump_bmp: Option<String>,
    #[arg(long)]
    dump_wav: Option<String>,
    #[arg(long)]
    headless_input: Option<String>,
    #[arg(long)]
    dump_save_state: Option<String>
}

fn press_button(key_status: &mut gba_emulator::memory::key_input_registers::KeyStatus, name: &str) {
    match name {
        "up" => key_status.set_dpad_up(0),
        "down" => key_status.set_dpad_down(0),
        "left" => key_status.set_dpad_left(0),
        "right" => key_status.set_dpad_right(0),
        "a" => key_status.set_button_a(0),
        "b" => key_status.set_button_b(0),
        "l" => key_status.set_button_l(0),
        "r" => key_status.set_button_r(0),
        "start" => key_status.set_button_start(0),
        "select" => key_status.set_button_select(0),
        other => error!("Unknown --headless-input button name: {}", other),
    }
}

fn parse_headless_input(spec: &str) -> std::collections::HashMap<usize, Vec<String>> {
    let mut schedule: std::collections::HashMap<usize, Vec<String>> = std::collections::HashMap::new();
    for entry in spec.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((frame_str, button)) = entry.split_once(':') {
            match frame_str.trim().parse::<usize>() {
                Ok(frame) => schedule.entry(frame).or_insert_with(Vec::new).push(button.trim().to_string()),
                Err(_) => error!("Invalid frame number in --headless-input entry: {}", entry),
            }
        } else {
            error!("Invalid --headless-input entry (expected frame:button): {}", entry);
        }
    }
    schedule
}

fn write_wav(path: &str, samples: &[i16], sample_rate: u32) -> std::io::Result<()> {
    const CHANNELS: u16 = 2;
    const BITS_PER_SAMPLE: u16 = 16;
    let byte_rate = sample_rate * (CHANNELS as u32) * (BITS_PER_SAMPLE as u32 / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    let data_size = (samples.len() * 2) as u32;

    let mut buf: Vec<u8> = Vec::with_capacity(44 + samples.len() * 2);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&CHANNELS.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        buf.extend_from_slice(&sample.to_le_bytes());
    }

    std::fs::write(path, buf)
}

fn write_bmp(path: &str, frame_buffer: &[u32], width: usize, height: usize) -> std::io::Result<()> {
    let row_size = width * 3;
    let pixel_data_size = row_size * height;
    let file_size = 54 + pixel_data_size;

    let mut buf: Vec<u8> = Vec::with_capacity(file_size);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(file_size as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(width as i32).to_le_bytes());
    buf.extend_from_slice(&(height as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..height).rev() {
        for x in 0..width {
            let pixel = frame_buffer[y * width + x];
            let r = ((pixel >> 16) & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let b = (pixel & 0xFF) as u8;
            buf.push(b);
            buf.push(g);
            buf.push(r);
        }
    }

    let mut file = File::create(path)?;
    file.write_all(&buf)
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

struct Resampler {
    input_rate: f64,
    output_rate: f64,
    position: f64,
    last_left: i16,
    last_right: i16,
}

impl Resampler {
    fn new(input_rate: u32, output_rate: u32) -> Resampler {
        Resampler {
            input_rate: input_rate as f64,
            output_rate: output_rate as f64,
            position: 0.0,
            last_left: 0,
            last_right: 0,
        }
    }

    fn process(&mut self, input: &[i16]) -> Vec<i16> {
        let frame_count = input.len() / 2;
        if frame_count == 0 {
            return Vec::new();
        }

        let mut combined: Vec<i16> = Vec::with_capacity(input.len() + 2);
        combined.push(self.last_left);
        combined.push(self.last_right);
        combined.extend_from_slice(input);
        let combined_frames = frame_count + 1;

        let ratio = self.input_rate / self.output_rate;
        let mut output = Vec::new();
        loop {
            let idx = self.position.floor() as usize;
            if idx + 1 >= combined_frames {
                break;
            }
            let frac = (self.position - idx as f64) as f32;
            let l0 = combined[idx * 2] as f32;
            let r0 = combined[idx * 2 + 1] as f32;
            let l1 = combined[(idx + 1) * 2] as f32;
            let r1 = combined[(idx + 1) * 2 + 1] as f32;
            output.push((l0 + (l1 - l0) * frac).round() as i16);
            output.push((r0 + (r1 - r0) * frac).round() as i16);
            self.position += ratio;
        }

        self.position -= frame_count as f64;
        self.last_left = input[input.len() - 2];
        self.last_right = input[input.len() - 1];

        output
    }
}

fn query_audio_output_config() -> Option<(cpal::Device, cpal::SupportedStreamConfig)> {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            error!("No audio output device available; running without sound.");
            return None;
        }
    };
    match device.default_output_config() {
        Ok(config) => Some((device, config)),
        Err(e) => {
            error!("Failed to get default audio output config: {}; running without sound.", e);
            None
        }
    }
}

fn build_and_start_audio_stream(
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    mut consumer: HeapConsumer<i16>,
    underrun_count: Arc<AtomicU64>,
) -> Option<cpal::Stream> {
    let sample_format = config.sample_format();
    let buffer_size_range = config.buffer_size().clone();
    let mut stream_config: cpal::StreamConfig = config.into();
    if let cpal::SupportedBufferSize::Range { min, max } = buffer_size_range {
        let desired = 4096u32.clamp(min, max);
        stream_config.buffer_size = cpal::BufferSize::Fixed(desired);
        info!("Requesting fixed audio buffer size of {} frames (device range {}-{})", desired, min, max);
    }

    let err_fn = |e| error!("Audio stream error: {}", e);

    let stream_result = match sample_format {
        cpal::SampleFormat::I16 => {
            let mut last_sample = 0i16;
            let underruns = underrun_count.clone();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| fill_audio_buffer(data, &mut consumer, &mut last_sample, &underruns, |s| s),
                err_fn,
                None,
            )
        },
        cpal::SampleFormat::U16 => {
            let mut last_sample = 0i16;
            let underruns = underrun_count.clone();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [u16], _| fill_audio_buffer(data, &mut consumer, &mut last_sample, &underruns, |s| (s as i32 + 32768) as u16),
                err_fn,
                None,
            )
        },
        cpal::SampleFormat::F32 => {
            let mut last_sample = 0i16;
            let underruns = underrun_count.clone();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| fill_audio_buffer(data, &mut consumer, &mut last_sample, &underruns, |s| s as f32 / 32768.0),
                err_fn,
                None,
            )
        },
        other => {
            error!("Unsupported audio sample format {:?}; running without sound.", other);
            return None;
        }
    };

    match stream_result {
        Ok(stream) => {
            if let Err(e) = stream.play() {
                error!("Failed to start audio stream: {}; running without sound.", e);
                return None;
            }
            Some(stream)
        }
        Err(e) => {
            error!("Failed to build audio output stream: {}; running without sound.", e);
            None
        }
    }
}

const UNDERRUN_DECAY: f32 = 0.995;

fn fill_audio_buffer<S: Copy>(
    data: &mut [S],
    consumer: &mut HeapConsumer<i16>,
    last_sample: &mut i16,
    underrun_count: &AtomicU64,
    convert: impl Fn(i16) -> S,
) {
    for slot in data.iter_mut() {
        let sample = match consumer.pop() {
            Some(s) => s,
            None => {
                underrun_count.fetch_add(1, Ordering::Relaxed);
                (*last_sample as f32 * UNDERRUN_DECAY) as i16
            }
        };
        *last_sample = sample;
        *slot = convert(sample);
    }
}

fn to_device_channels(stereo: &[i16], channels: u16) -> Vec<i16> {
    if channels == 2 {
        return stereo.to_vec();
    }
    let mut output = Vec::with_capacity((stereo.len() / 2) * channels as usize);
    for pair in stereo.chunks_exact(2) {
        let (left, right) = (pair[0], pair[1]);
        if channels == 1 {
            output.push(((left as i32 + right as i32) / 2) as i16);
        } else {
            output.push(left);
            output.push(right);
            for _ in 2..channels {
                output.push(0);
            }
        }
    }
    output
}

const SAVE_STATE_SLOTS: usize = 9;
const BIOS_FILENAME: &str = "gba_bios.bin";
const GBA_BIOS_SIZE: u64 = 16384;

#[cfg(target_os = "windows")]
extern "system" {
    fn FreeConsole() -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period_ms: u32) -> u32;
}

#[cfg(target_os = "windows")]
fn detach_console() {
    unsafe { FreeConsole(); }
}

#[cfg(not(target_os = "windows"))]
fn detach_console() {}

#[cfg(target_os = "windows")]
fn raise_timer_resolution() {
    unsafe { timeBeginPeriod(1); }
}

#[cfg(not(target_os = "windows"))]
fn raise_timer_resolution() {}

fn is_valid_bios_size(byte_len: u64) -> bool {
    byte_len == GBA_BIOS_SIZE
}

fn find_local_bios() -> Option<PathBuf> {
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join(BIOS_FILENAME);
    let metadata = std::fs::metadata(&path).ok()?;
    if is_valid_bios_size(metadata.len()) {
        Some(path)
    } else {
        error!(
            "{} exists but is {} bytes (expected {}); ignoring it",
            path.display(), metadata.len(), GBA_BIOS_SIZE
        );
        None
    }
}

fn save_path_for_rom(rom_path: &Path) -> PathBuf {
    rom_path.with_extension("sav")
}

fn save_state_path_for_slot(rom_path: &Path, slot: usize) -> PathBuf {
    rom_path.with_extension(format!("state{}", slot))
}

struct RunningGame {
    gba: GBA,
    game_pack: GamePack,
    gba_pc: u32,
    rom_path: PathBuf,
    save_path: PathBuf,
}

fn open_rom(bios_path: &str, rom_path: &Path, skip_bios: bool) -> Option<RunningGame> {
    let rom_path_string = rom_path.to_string_lossy().to_string();
    let game_pack = match GamePack::load(bios_path, &rom_path_string) {
        Ok(pack) => pack,
        Err(e) => {
            error!("{}", e);
            return None;
        }
    };

    let gba_pc = if skip_bios { 0x08000000 } else { 0x0 };
    let mut gba = GBA::new(gba_pc, &game_pack);

    let save_path = save_path_for_rom(rom_path);
    if save_path.exists() {
        read_save_file(&mut gba, &save_path.to_string_lossy().to_string());
    } else {
        info!("No existing save file for {}", rom_path.display());
    }

    Some(RunningGame {
        gba,
        game_pack,
        gba_pc,
        rom_path: rom_path.to_path_buf(),
        save_path,
    })
}

struct AppMenu {
    #[allow(dead_code)]
    menu: Menu,
    open_rom: MenuItem,
    exit: MenuItem,
    pause: MenuItem,
    reset: MenuItem,
    select_save_file: MenuItem,
    save_state_slots: Vec<MenuItem>,
    load_state_slots: Vec<MenuItem>,
    emulation_menu: Submenu,
    save_menu: Submenu,
    #[allow(dead_code)]
    settings_menu: Submenu,
    debug: CheckMenuItem,
    debug_step: MenuItem,
    debug_continue: MenuItem,
    debug_clear_breakpoints: MenuItem,
}

impl AppMenu {
    fn set_game_loaded(&self, loaded: bool) {
        self.emulation_menu.set_enabled(loaded);
        self.save_menu.set_enabled(loaded);
    }

    fn refresh_state_slots(&self, rom_path: Option<&Path>) {
        for (i, item) in self.load_state_slots.iter().enumerate() {
            let exists = rom_path
                .map(|p| save_state_path_for_slot(p, i + 1).exists())
                .unwrap_or(false);
            item.set_enabled(exists);
        }
    }
}

fn build_menu(window: &Window, debugger_enabled: bool) -> AppMenu {
    let menu = Menu::new();

    let open_rom = MenuItem::new("Open ROM...", true, None);
    let exit = MenuItem::new("Exit", true, None);
    let file_menu = Submenu::with_items(
        "File",
        true,
        &[
            &open_rom,
            &PredefinedMenuItem::separator(),
            &exit,
        ],
    )
    .expect("failed to build File menu");

    let pause = MenuItem::new("Pause", true, None);
    let reset = MenuItem::new("Reset", true, None);
    let emulation_menu = Submenu::with_items("Emulation", true, &[&pause, &reset])
        .expect("failed to build Emulation menu");

    let save_state_slots: Vec<MenuItem> = (1..=SAVE_STATE_SLOTS)
        .map(|i| MenuItem::new(format!("Slot {}", i), true, None))
        .collect();
    let load_state_slots: Vec<MenuItem> = (1..=SAVE_STATE_SLOTS)
        .map(|i| MenuItem::new(format!("Slot {}", i), true, None))
        .collect();

    let save_state_refs: Vec<&dyn IsMenuItem> = save_state_slots
        .iter()
        .map(|item| item as &dyn IsMenuItem)
        .collect();
    let load_state_refs: Vec<&dyn IsMenuItem> = load_state_slots
        .iter()
        .map(|item| item as &dyn IsMenuItem)
        .collect();

    let save_state_submenu = Submenu::with_items("Save State", true, &save_state_refs)
        .expect("failed to build Save State submenu");
    let load_state_submenu = Submenu::with_items("Load State", true, &load_state_refs)
        .expect("failed to build Load State submenu");
    let select_save_file = MenuItem::new("Select Save File...", true, None);

    let save_menu = Submenu::with_items(
        "Save",
        true,
        &[
            &save_state_submenu,
            &load_state_submenu,
            &PredefinedMenuItem::separator(),
            &select_save_file,
        ],
    )
    .expect("failed to build Save menu");

    let settings_menu = Submenu::new("Settings", true);

    let debug = CheckMenuItem::new("Show Debugger", true, debugger_enabled, None);
    let debug_step = MenuItem::new("Step Instruction (F10)", true, None);
    let debug_continue = MenuItem::new("Continue (F5)", true, None);
    let debug_clear_breakpoints = MenuItem::new("Clear Breakpoints", true, None);
    let debug_menu = Submenu::with_items(
        "Debug",
        true,
        &[
            &debug,
            &PredefinedMenuItem::separator(),
            &debug_step,
            &debug_continue,
            &debug_clear_breakpoints,
        ],
    )
    .expect("failed to build Debug menu");

    menu.append(&file_menu).expect("failed to append File menu");
    menu.append(&emulation_menu).expect("failed to append Emulation menu");
    menu.append(&save_menu).expect("failed to append Save menu");
    menu.append(&settings_menu).expect("failed to append Settings menu");
    menu.append(&debug_menu).expect("failed to append Debug menu");

    #[cfg(target_os = "windows")]
    {
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                unsafe {
                    menu.init_for_hwnd(win32_handle.hwnd.get())
                        .expect("failed to attach menu bar to window");
                }
            }
        }
    }

    let app_menu = AppMenu {
        menu,
        open_rom,
        exit,
        pause,
        reset,
        select_save_file,
        save_state_slots,
        load_state_slots,
        emulation_menu,
        save_menu,
        settings_menu,
        debug,
        debug_step,
        debug_continue,
        debug_clear_breakpoints,
    };
    app_menu.set_game_loaded(false);
    app_menu
}

fn recreate_window(old: &Window, debugger_shown: bool, opts: &Opts, audio_active: bool) -> (Window, AppMenu) {
    let position = old.get_position();
    let title = if debugger_shown { "GBA Emulator (Debug)" } else { "GBA Emulator" };

    let (width, height, scale) = if debugger_shown {
        (debug_ui::TOTAL_WIDTH, debug_ui::TOTAL_HEIGHT, debug_ui::SCALE)
    } else {
        (WIDTH, HEIGHT, minifb::Scale::X8)
    };

    let mut new_window = Window::new(
        title,
        width,
        height,
        WindowOptions {
            resize: true,
            scale,
            ..WindowOptions::default()
        },
    )
    .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    new_window.set_position(position.0, position.1);
    if !audio_active {
        new_window.set_target_fps(opts.frame_cap.unwrap_or(60));
    }

    let new_menu = build_menu(&new_window, debugger_shown);
    (new_window, new_menu)
}

const MAX_STEPS_PER_SLICE: u32 = 300_000;

fn emulate_frame(gba: &mut GBA, debugger_state: &DebuggerState) -> bool {
    if debugger_state.enabled && !debugger_state.breakpoints.is_empty() {
        gba.frame_until_breakpoint(&debugger_state.breakpoints, MAX_STEPS_PER_SLICE)
    } else {
        gba.frame();
        true
    }
}

fn present(window: &mut Window, debugger_state: &DebuggerState, debug_buf: &mut [u32], game_buf: &[u32], gba: Option<&GBA>) {
    if debugger_state.enabled {
        debug_ui::render(debug_buf, debug_ui::TOTAL_WIDTH, debugger_state, gba);
        for row in 0..HEIGHT {
            let src = row * WIDTH;
            let dst = row * debug_ui::TOTAL_WIDTH;
            debug_buf[dst..dst + WIDTH].copy_from_slice(&game_buf[src..src + WIDTH]);
        }
        window
            .update_with_buffer(debug_buf, debug_ui::TOTAL_WIDTH, debug_ui::TOTAL_HEIGHT)
            .unwrap();
    } else {
        window.update_with_buffer(game_buf, WIDTH, HEIGHT).unwrap();
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

    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {}", info);
    }));

    let opts: Opts = Opts::parse();
    let mut gilrs = Gilrs::new().unwrap();
    let mut active_gamepad = None;

    if let Some(frame_count) = opts.headless_frames {
        let bios_file = opts.bios_file.clone().unwrap_or_else(|| {
            error!("--headless-frames requires a BIOS file argument");
            std::process::exit(1);
        });
        let rom_file = opts.rom_file.clone().unwrap_or_else(|| {
            error!("--headless-frames requires a ROM file argument");
            std::process::exit(1);
        });
        let game_pack = match GamePack::load(&bios_file, &rom_file) {
            Ok(pack) => pack,
            Err(e) => {
                error!("{}", e);
                std::process::exit(1);
            }
        };

        let gba_pc = if opts.skip_bios { 0x08000000 } else { 0x0 };

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

        let input_schedule = opts.headless_input.as_deref().map(parse_headless_input).unwrap_or_default();
        for frame_index in 0..frame_count {
            gba.key_status.set_register(0xFFFF);
            if let Some(buttons) = input_schedule.get(&frame_index) {
                for button in buttons {
                    press_button(&mut gba.key_status, button);
                }
            }
            gba.frame();
        }
        if let Some(ref dump_path) = opts.dump_bmp {
            match write_bmp(dump_path, &gba.gpu.frame_buffer, WIDTH, HEIGHT) {
                Ok(_) => info!("Wrote headless framebuffer dump to {}", dump_path),
                Err(e) => error!("Failed to write BMP dump: {}", e),
            }
        }
        if let Some(ref dump_path) = opts.dump_wav {
            match write_wav(dump_path, &gba.apu.sample_buffer, gba_emulator::apu::OUTPUT_SAMPLE_RATE as u32) {
                Ok(_) => info!("Wrote headless audio dump to {}", dump_path),
                Err(e) => error!("Failed to write WAV dump: {}", e),
            }
        }
        if let Some(ref save_path) = opts.dump_save_state {
            write_save_state(&mut gba, save_path);
            info!("Wrote headless save state to {}", save_path);
        }
        return;
    }

    detach_console();
    raise_timer_resolution();

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

    let mut app_menu = build_menu(&window, false);
    let local_bios_path = find_local_bios();
    app_menu.open_rom.set_enabled(local_bios_path.is_some());
    if local_bios_path.is_none() {
        let bios_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        error!(
            "No {} found in {}; Open ROM is disabled until it's placed there",
            BIOS_FILENAME, bios_dir.display()
        );
        rfd::MessageDialog::new()
            .set_title("BIOS not found")
            .set_description(format!(
                "Place a valid {} ({} bytes) in:\n{}\n\nOpen ROM is disabled until then.",
                BIOS_FILENAME, GBA_BIOS_SIZE, bios_dir.display()
            ))
            .set_level(rfd::MessageLevel::Warning)
            .show();
    }
    let blank_buffer = vec![0u32; WIDTH * HEIGHT];
    let mut paused = false;
    let mut should_exit = false;
    let mut debugger_state = DebuggerState::default();
    let mut debug_buf = vec![0u32; debug_ui::TOTAL_WIDTH * debug_ui::TOTAL_HEIGHT];

    let mut current: Option<RunningGame> = if let (Some(bios_file), Some(rom_file)) =
        (opts.bios_file.clone(), opts.rom_file.clone())
    {
        let rom_path = PathBuf::from(&rom_file);
        let gba_pc = if opts.skip_bios { 0x08000000 } else { 0x0 };
        match GamePack::load(&bios_file, &rom_file) {
            Ok(game_pack) => {
                let mut gba = if let Some(ref save_path) = opts.save_state {
                    read_save_state(save_path, gba_pc, &game_pack)
                } else {
                    GBA::new(gba_pc, &game_pack)
                };
                let save_path = match &opts.save_file {
                    Some(path) => PathBuf::from(path),
                    None => save_path_for_rom(&rom_path),
                };
                if save_path.exists() {
                    read_save_file(&mut gba, &save_path.to_string_lossy().to_string());
                }
                Some(RunningGame { gba, game_pack, gba_pc, rom_path, save_path })
            }
            Err(e) => {
                error!("{}", e);
                None
            }
        }
    } else {
        None
    };
    app_menu.set_game_loaded(current.is_some());
    app_menu.refresh_state_slots(current.as_ref().map(|g| g.rom_path.as_path()));

    let mut fps_counter_buffer = VecDeque::new();
    let mut a: Mean = fps_counter_buffer.iter().collect();

    let audio_config = query_audio_output_config();
    let (device_sample_rate, device_channels) = match &audio_config {
        Some((_, config)) => (config.sample_rate().0, config.channels()),
        None => (gba_emulator::apu::OUTPUT_SAMPLE_RATE as u32, 2),
    };
    match &audio_config {
        Some((device, config)) => info!(
            "Audio output: {:?} @ {} Hz, {} channel(s), {:?}",
            device.name(), config.sample_rate().0, config.channels(), config.sample_format()
        ),
        None => info!("No audio output device active; running silently."),
    }
    let mut resampler = Resampler::new(gba_emulator::apu::OUTPUT_SAMPLE_RATE as u32, device_sample_rate);
    let max_buffered_samples = (device_sample_rate as usize) * (device_channels as usize) * 2;

    let ring = HeapRb::<i16>::new(max_buffered_samples);
    let (mut audio_producer, audio_consumer) = ring.split();

    let warmup_target_samples = (device_sample_rate as usize) * (device_channels as usize) / 5;
    if let Some(game) = current.as_mut() {
        while audio_producer.len() < warmup_target_samples {
            game.gba.frame();
            let new_samples = std::mem::take(&mut game.gba.apu.sample_buffer);
            let resampled = resampler.process(&new_samples);
            let device_samples = to_device_channels(&resampled, device_channels);
            audio_producer.push_slice(&device_samples);
        }
    }

    let underrun_count = Arc::new(AtomicU64::new(0));
    let mut overrun_count: u64 = 0;

    let _audio_stream = audio_config.and_then(|(device, config)| {
        build_and_start_audio_stream(device, config, audio_consumer, underrun_count.clone())
    });
    let audio_active = _audio_stream.is_some();

    if !audio_active {
        window.set_target_fps(opts.frame_cap.unwrap_or(60));
    }

    const GBA_FRAME_SECONDS: f64 = 280896.0 / 16777216.0;
    const MAX_CATCHUP_FRAMES: u32 = 30;
    const TURBO_MULTIPLIER: f64 = 4.0;
    const SLEEP_SAFETY_MARGIN_SECONDS: f64 = 0.002;

    let mut time_accumulator = 0.0f64;
    let mut last_instant = Instant::now();
    let mut last_audio_stats_log = Instant::now();
    let mut last_turbo = false;
    let mut last_mouse_down = false;

    while window.is_open() && !window.is_key_down(Key::Escape) && !should_exit {
        let now = Instant::now();

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == app_menu.open_rom.id() {
                if let Some(bios_path) = &local_bios_path {
                    if let Some(rom_path) = rfd::FileDialog::new().add_filter("GBA ROM", &["gba"]).pick_file() {
                        if let Some(mut game) = current.take() {
                            write_save_file(&mut game.gba, &game.save_path.to_string_lossy().to_string());
                        }
                        if let Some(new_game) = open_rom(&bios_path.to_string_lossy(), &rom_path, opts.skip_bios) {
                            app_menu.refresh_state_slots(Some(&new_game.rom_path));
                            app_menu.set_game_loaded(true);
                            paused = false;
                            current = Some(new_game);
                        } else {
                            app_menu.set_game_loaded(false);
                        }
                    }
                }
            } else if event.id == app_menu.exit.id() {
                should_exit = true;
            } else if event.id == app_menu.pause.id() {
                paused = !paused;
                let _ = app_menu.pause.set_text(if paused { "Resume" } else { "Pause" });
            } else if event.id == app_menu.reset.id() {
                if let Some(game) = current.as_mut() {
                    write_save_file(&mut game.gba, &game.save_path.to_string_lossy().to_string());
                    let mut fresh_gba = GBA::new(game.gba_pc, &game.game_pack);
                    if game.save_path.exists() {
                        read_save_file(&mut fresh_gba, &game.save_path.to_string_lossy().to_string());
                    }
                    game.gba = fresh_gba;
                    paused = false;
                }
            } else if event.id == app_menu.select_save_file.id() {
                if let Some(game) = current.as_mut() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("GBA save file", &["sav"]).pick_file() {
                        game.save_path = path;
                        read_save_file(&mut game.gba, &game.save_path.to_string_lossy().to_string());
                    }
                }
            } else if let Some(slot) = app_menu.save_state_slots.iter().position(|item| event.id == item.id()) {
                if let Some(game) = current.as_mut() {
                    let slot_path = save_state_path_for_slot(&game.rom_path, slot + 1);
                    write_save_state(&mut game.gba, &slot_path.to_string_lossy().to_string());
                    app_menu.refresh_state_slots(Some(&game.rom_path));
                }
            } else if let Some(slot) = app_menu.load_state_slots.iter().position(|item| event.id == item.id()) {
                if let Some(game) = current.as_mut() {
                    let slot_path = save_state_path_for_slot(&game.rom_path, slot + 1);
                    if slot_path.exists() {
                        game.gba = read_save_state(&slot_path.to_string_lossy().to_string(), game.gba_pc, &game.game_pack);
                        paused = false;
                    }
                }
            } else if event.id == app_menu.debug.id() {
                debugger_state.enabled = !debugger_state.enabled;
                let (new_window, new_menu) = recreate_window(&window, debugger_state.enabled, &opts, audio_active);
                window = new_window;
                app_menu = new_menu;
                app_menu.set_game_loaded(current.is_some());
                app_menu.refresh_state_slots(current.as_ref().map(|g| g.rom_path.as_path()));
                if paused {
                    let _ = app_menu.pause.set_text("Resume");
                }
            } else if event.id == app_menu.debug_step.id() {
                if debugger_state.enabled {
                    if let Some(game) = current.as_mut() {
                        game.gba.single_step();
                        let _ = std::mem::take(&mut game.gba.apu.sample_buffer);
                        paused = true;
                        let _ = app_menu.pause.set_text("Resume");
                    }
                }
            } else if event.id == app_menu.debug_continue.id() {
                paused = false;
                let _ = app_menu.pause.set_text("Pause");
            } else if event.id == app_menu.debug_clear_breakpoints.id() {
                debugger_state.breakpoints.clear();
            }
        }

        let turbo = window.is_key_down(Key::Space)
            || active_gamepad.map(|id| gilrs.gamepad(id).is_pressed(Button::Select)).unwrap_or(false);
        if turbo != last_turbo && !opts.fps_counter {
            window.set_title(if turbo { "GBA Emulator (Turbo)" } else { "GBA Emulator" });
        }
        last_turbo = turbo;

        if debugger_state.enabled {
            if window.is_key_pressed(Key::F10, KeyRepeat::No) {
                if let Some(game) = current.as_mut() {
                    game.gba.single_step();
                    let _ = std::mem::take(&mut game.gba.apu.sample_buffer);
                    paused = true;
                    let _ = app_menu.pause.set_text("Resume");
                }
            }
            if window.is_key_pressed(Key::F5, KeyRepeat::No) {
                paused = false;
                let _ = app_menu.pause.set_text("Pause");
            }
            if window.is_key_pressed(Key::F9, KeyRepeat::No) {
                if let Some(game) = current.as_ref() {
                    let pc = game.gba.cpu.get_pc();
                    if !debugger_state.breakpoints.remove(&pc) {
                        debugger_state.breakpoints.insert(pc);
                    }
                }
            }

            if window.is_key_pressed(Key::LeftBracket, KeyRepeat::No) {
                debugger_state.memory_region = (debugger_state.memory_region + debug_ui::panels::memory::REGIONS.len() - 1) % debug_ui::panels::memory::REGIONS.len();
                debugger_state.memory_base = debug_ui::panels::memory::REGIONS[debugger_state.memory_region].1;
            }
            if window.is_key_pressed(Key::RightBracket, KeyRepeat::No) {
                debugger_state.memory_region = (debugger_state.memory_region + 1) % debug_ui::panels::memory::REGIONS.len();
                debugger_state.memory_base = debug_ui::panels::memory::REGIONS[debugger_state.memory_region].1;
            }
            if window.is_key_pressed(Key::Up, KeyRepeat::Yes) {
                debugger_state.memory_base = debugger_state.memory_base.wrapping_sub(debug_ui::panels::memory::ROW_BYTES);
            }
            if window.is_key_pressed(Key::Down, KeyRepeat::Yes) {
                debugger_state.memory_base = debugger_state.memory_base.wrapping_add(debug_ui::panels::memory::ROW_BYTES);
            }
            if window.is_key_pressed(Key::PageUp, KeyRepeat::Yes) {
                debugger_state.memory_base = debugger_state.memory_base.wrapping_sub(debug_ui::panels::memory::ROW_BYTES * debug_ui::panels::memory::VISIBLE_ROWS);
            }
            if window.is_key_pressed(Key::PageDown, KeyRepeat::Yes) {
                debugger_state.memory_base = debugger_state.memory_base.wrapping_add(debug_ui::panels::memory::ROW_BYTES * debug_ui::panels::memory::VISIBLE_ROWS);
            }

            let mouse_down = window.get_mouse_down(MouseButton::Left);
            let mouse_clicked = mouse_down && !last_mouse_down;
            last_mouse_down = mouse_down;
            if mouse_clicked {
                if let Some(game) = current.as_ref() {
                    if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Clamp) {
                        if let Some(addr) = debug_ui::panels::disassembly::address_for_click(&debug_ui::DISASM_PANEL, &game.gba, mx, my) {
                            if !debugger_state.breakpoints.remove(&addr) {
                                debugger_state.breakpoints.insert(addr);
                            }
                        }
                    }
                }
            }
        }

        let mut frames_emulated: u32 = 0;

        match current.as_mut() {
            Some(game) if !paused => {
                if audio_active && opts.frame_cap.is_none() {
                    let mut dt = now.duration_since(last_instant).as_secs_f64();
                    last_instant = now;
                    if turbo {
                        dt *= TURBO_MULTIPLIER;
                    }
                    time_accumulator = (time_accumulator + dt).min(GBA_FRAME_SECONDS * MAX_CATCHUP_FRAMES as f64);

                    while time_accumulator >= GBA_FRAME_SECONDS {
                        let completed = emulate_frame(&mut game.gba, &debugger_state);
                        frames_emulated += 1;
                        time_accumulator -= GBA_FRAME_SECONDS;

                        let new_samples = std::mem::take(&mut game.gba.apu.sample_buffer);
                        if !turbo {
                            let resampled = resampler.process(&new_samples);
                            let device_samples = to_device_channels(&resampled, device_channels);
                            let written = audio_producer.push_slice(&device_samples);
                            overrun_count += (device_samples.len() - written) as u64;
                        }

                        if !completed {
                            paused = true;
                            let _ = app_menu.pause.set_text("Resume");
                            break;
                        }
                    }

                    if !turbo {
                        let remaining = GBA_FRAME_SECONDS - time_accumulator;
                        let sleep_secs = (remaining - SLEEP_SAFETY_MARGIN_SECONDS).max(0.0);
                        if sleep_secs > 0.0 {
                            std::thread::sleep(std::time::Duration::from_secs_f64(sleep_secs));
                        }
                    }
                } else {
                    let frames_this_iteration = if turbo { TURBO_MULTIPLIER as usize } else { 1 };
                    for _ in 0..frames_this_iteration {
                        let completed = emulate_frame(&mut game.gba, &debugger_state);
                        frames_emulated += 1;

                        let new_samples = std::mem::take(&mut game.gba.apu.sample_buffer);
                        if !turbo {
                            let resampled = resampler.process(&new_samples);
                            let device_samples = to_device_channels(&resampled, device_channels);
                            audio_producer.push_slice(&device_samples);
                        }

                        if !completed {
                            paused = true;
                            let _ = app_menu.pause.set_text("Resume");
                            break;
                        }
                    }
                }

                game.gba.key_status.set_register(0xFFFF);

                while let Some(Event { id, ..}) = gilrs.next_event() {
                    active_gamepad = Some(id);
                }

                if let Some(gamepad) = active_gamepad.map(|id| gilrs.gamepad(id)) {
                    if gamepad.is_pressed(Button::DPadUp) { game.gba.key_status.set_dpad_up(0); }
                    if gamepad.is_pressed(Button::DPadDown) { game.gba.key_status.set_dpad_down(0); }
                    if gamepad.is_pressed(Button::DPadLeft) { game.gba.key_status.set_dpad_left(0); }
                    if gamepad.is_pressed(Button::DPadRight) { game.gba.key_status.set_dpad_right(0); }
                    if gamepad.is_pressed(Button::South) { game.gba.key_status.set_button_a(0); }
                    if gamepad.is_pressed(Button::East) { game.gba.key_status.set_button_b(0); }
                    if gamepad.is_pressed(Button::RightTrigger) { game.gba.key_status.set_button_r(0); }
                    if gamepad.is_pressed(Button::LeftTrigger) { game.gba.key_status.set_button_l(0); }
                    if gamepad.is_pressed(Button::Select) { game.gba.key_status.set_button_select(0); }
                    if gamepad.is_pressed(Button::Start) { game.gba.key_status.set_button_start(0); }
                }

                window.get_keys().iter().for_each(|key| {
                    match key {
                        Key::W => game.gba.key_status.set_dpad_up(0),
                        Key::S => game.gba.key_status.set_dpad_down(0),
                        Key::A => game.gba.key_status.set_dpad_left(0),
                        Key::D => game.gba.key_status.set_dpad_right(0),
                        Key::H => game.gba.key_status.set_button_a(0),
                        Key::J => game.gba.key_status.set_button_b(0),
                        Key::R => game.gba.key_status.set_button_r(0),
                        Key::Q => game.gba.key_status.set_button_l(0),
                        Key::Enter => game.gba.key_status.set_button_start(0),
                        Key::Backspace => game.gba.key_status.set_button_select(0),
                        _ => ()
                    }
                });

                present(&mut window, &debugger_state, &mut debug_buf, &game.gba.gpu.frame_buffer, Some(&game.gba));

                if opts.fps_counter && frames_emulated > 0 {
                    fps_counter_buffer.push_back(1f64 / now.elapsed().as_secs_f64());
                    if fps_counter_buffer.len() == FPS_BUFFER_SIZE {
                        a = fps_counter_buffer.drain(0..FPS_BUFFER_SIZE).collect();
                    }
                    window.set_title(&format!("GBA Emu: {} FPS", a.mean()));
                }
            }
            Some(game) => {
                present(&mut window, &debugger_state, &mut debug_buf, &game.gba.gpu.frame_buffer, Some(&game.gba));
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            None => {
                present(&mut window, &debugger_state, &mut debug_buf, &blank_buffer, None);
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }

        if audio_active && now.duration_since(last_audio_stats_log).as_secs_f64() >= 2.0 {
            last_audio_stats_log = now;
            let underruns = underrun_count.swap(0, Ordering::Relaxed);
            if underruns > 0 || overrun_count > 0 {
                info!("Audio buffer: {} underrun samples, {} overrun samples dropped (last 2s)", underruns, overrun_count);
                overrun_count = 0;
            }
        }
    }

    if let Some(game) = current.as_mut() {
        write_save_file(&mut game.gba, &game.save_path.to_string_lossy().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_path_for_rom_replaces_extension_with_sav() {
        let rom = Path::new("C:/roms/Some Game (USA).gba");
        assert_eq!(save_path_for_rom(rom), PathBuf::from("C:/roms/Some Game (USA).sav"));
    }

    #[test]
    fn save_state_path_for_slot_numbers_each_slot() {
        let rom = Path::new("C:/roms/game.gba");
        assert_eq!(save_state_path_for_slot(rom, 1), PathBuf::from("C:/roms/game.state1"));
        assert_eq!(save_state_path_for_slot(rom, 9), PathBuf::from("C:/roms/game.state9"));
    }

    #[test]
    fn is_valid_bios_size_accepts_exactly_16kb() {
        assert!(is_valid_bios_size(16384));
    }

    #[test]
    fn is_valid_bios_size_rejects_a_rom_sized_file() {
        assert!(!is_valid_bios_size(8 * 1024 * 1024));
    }

    #[test]
    fn is_valid_bios_size_rejects_truncated_or_padded_files() {
        assert!(!is_valid_bios_size(0));
        assert!(!is_valid_bios_size(16383));
        assert!(!is_valid_bios_size(16385));
    }

    #[test]
    fn resampler_passes_through_at_equal_rates() {
        let mut r = Resampler::new(32768, 32768);
        let input = vec![100, -100, 200, -200, 300, -300];
        let output = r.process(&input);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn resampler_upsamples_to_more_frames() {
        let mut r = Resampler::new(32768, 48000);
        let input = vec![0i16; 2000];
        let output = r.process(&input);
        assert!(output.len() > input.len());
    }

    #[test]
    fn resampler_preserves_phase_across_calls() {
        let mut r = Resampler::new(32768, 48000);
        let mut total_in = 0;
        let mut total_out = 0;
        for _ in 0..10 {
            let input = vec![1i16; 200];
            total_in += input.len() / 2;
            total_out += r.process(&input).len() / 2;
        }
        let expected = (total_in as f64 * 48000.0 / 32768.0) as usize;
        assert!(total_out.abs_diff(expected) <= 1);
    }

    #[test]
    fn to_device_channels_stereo_passthrough() {
        let stereo = vec![1, 2, 3, 4];
        assert_eq!(to_device_channels(&stereo, 2), stereo);
    }

    #[test]
    fn to_device_channels_mono_averages() {
        let stereo = vec![10, 20, -10, 10];
        assert_eq!(to_device_channels(&stereo, 1), vec![15, 0]);
    }

    #[test]
    fn to_device_channels_multichannel_pads_silence() {
        let stereo = vec![10, 20];
        assert_eq!(to_device_channels(&stereo, 4), vec![10, 20, 0, 0]);
    }

    #[test]
    fn fill_audio_buffer_passes_through_available_samples() {
        let ring = HeapRb::<i16>::new(8);
        let (mut producer, mut consumer) = ring.split();
        producer.push_slice(&[100, -100, 200]);
        let mut last_sample = 0i16;
        let underruns = AtomicU64::new(0);
        let mut data = [0i16; 3];
        fill_audio_buffer(&mut data, &mut consumer, &mut last_sample, &underruns, |s| s);
        assert_eq!(data, [100, -100, 200]);
        assert_eq!(underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fill_audio_buffer_decays_instead_of_dropping_to_zero_on_underrun() {
        let ring = HeapRb::<i16>::new(8);
        let (_producer, mut consumer) = ring.split();
        let mut last_sample = 1000i16;
        let underruns = AtomicU64::new(0);
        let mut data = [0i16; 3];
        fill_audio_buffer(&mut data, &mut consumer, &mut last_sample, &underruns, |s| s);
        assert!(data[0] > 0 && data[0] < 1000, "expected a decayed non-zero sample, got {}", data[0]);
        assert!(data[1] < data[0]);
        assert!(data[2] < data[1]);
        assert_eq!(underruns.load(Ordering::Relaxed), 3);
    }
}
