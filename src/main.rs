extern crate minifb;
use gba_emulator::{cpu::cpu::CPU, gamepak::{self, GamePack}, gba::GBA};
use gilrs::{Button, Event, Gilrs};
use std::{fs::{File, OpenOptions}, io::prelude::*};
use std::sync::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use log::{Level, Metadata, Record, SetLoggerError, error, info};
use std::{collections::VecDeque, time::Instant};
use minifb::{Key, Window, WindowOptions};
use average::Mean;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapRb, HeapConsumer};


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

    // Truncates any log file from a previous run, so each run starts fresh
    // and can be reviewed on its own between sessions.
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
    bios_file: String,
    rom_file: String,
    save_file: Option<String>,
    /// Skip the BIOS boot animation and start execution at the ROM entry point.
    #[arg(short = 'b', long)]
    skip_bios: bool,
    /// Cap emulation to this many frames per second.
    #[arg(short = 'c', long)]
    frame_cap: Option<usize>,
    /// Print an FPS counter to the log.
    #[arg(short = 'f', long)]
    fps_counter: bool,
    /// Resume from a save state file instead of a cold boot.
    #[arg(short = 's', long)]
    save_state: Option<String>,
    /// Run headlessly (no window) for this many frames, dump the final
    /// framebuffer to --dump-bmp, then exit. For scripted test-ROM runs.
    #[arg(long)]
    headless_frames: Option<usize>,
    /// Output path for the --headless-frames framebuffer dump (24bpp BMP).
    #[arg(long)]
    dump_bmp: Option<String>,
    /// Output path for the --headless-frames audio dump (16-bit PCM WAV,
    /// at the Apu's fixed internal sample rate).
    #[arg(long)]
    dump_wav: Option<String>,
    /// Scripted input for --headless-frames: "frame:button,frame:button,...".
    /// Each button is pressed for exactly one frame at the given frame
    /// index (0-based), then released. Button names: up/down/left/right/
    /// a/b/l/r/start/select. Example: "60:start,120:down,126:a".
    #[arg(long)]
    headless_input: Option<String>,
    /// Write a save state to this path after --headless-frames finishes
    /// (separate from --save-state, which is only ever read from — this
    /// lets a headless run checkpoint progress without overwriting the
    /// state it resumed from).
    #[arg(long)]
    dump_save_state: Option<String>
}

/// Applies one button (named as in --headless-input) to a KeyStatus for
/// exactly the current frame.
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

/// Parses "frame:button,frame:button,..." into a lookup from frame index to
/// the list of buttons pressed on exactly that frame.
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

/// Writes interleaved-stereo i16 PCM samples as a 16-bit PCM WAV file.
/// Used by --headless-frames to inspect audio output without needing a
/// live audio device — the audio counterpart to --dump-bmp.
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
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
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

/// Writes an uncompressed 24bpp BMP from a 0RGB framebuffer (as produced by
/// gpu::frame_buffer). Used by --headless-frames to let a screenshot of a
/// test ROM's result screen be inspected without needing a live window.
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

    // BMP rows are stored bottom-up.
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

// Linear-interpolation resampler from the Apu's fixed 32768 Hz to the device's rate.
// Carries the last frame across calls so consecutive batches interpolate smoothly.
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

    // `input`/return value are interleaved stereo (L, R, L, R, ...).
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

// `consumer` is the read half of a lock-free SPSC ring buffer, filled with a
// startup cushion by the caller before this is invoked (main's warm-up loop).
fn build_and_start_audio_stream(
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    mut consumer: HeapConsumer<i16>,
    underrun_count: Arc<AtomicU64>,
) -> Option<cpal::Stream> {
    let sample_format = config.sample_format();
    let buffer_size_range = config.buffer_size().clone();
    let mut stream_config: cpal::StreamConfig = config.into();
    // Larger fixed buffer: HDMI/DisplayPort-routed audio devices are glitch-prone with WASAPI's default size.
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

// On underrun, holding the last sample (decaying toward silence) avoids the
// audible click a hard drop to zero would cause mid-waveform.
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
    let game_pack = match GamePack::load(&opts.bios_file, &opts.rom_file) {
        Ok(pack) => pack,
        Err(e) => {
            error!("{}", e);
            std::process::exit(1);
        }
    };
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

    if let Some(frame_count) = opts.headless_frames {
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

    // Pre-fill a ~200ms cushion before starting playback to absorb early jitter.
    let warmup_target_samples = (device_sample_rate as usize) * (device_channels as usize) / 5;
    while audio_producer.len() < warmup_target_samples {
        gba.frame();
        let new_samples = std::mem::take(&mut gba.apu.sample_buffer);
        let resampled = resampler.process(&new_samples);
        let device_samples = to_device_channels(&resampled, device_channels);
        audio_producer.push_slice(&device_samples);
    }

    let underrun_count = Arc::new(AtomicU64::new(0));
    let mut overrun_count: u64 = 0;

    // Dropping the Stream stops playback; None means no device or one failed to open.
    let _audio_stream = audio_config.and_then(|(device, config)| {
        build_and_start_audio_stream(device, config, audio_consumer, underrun_count.clone())
    });
    let audio_active = _audio_stream.is_some();

    // Pace emulation off real elapsed wall-clock time rather than a fixed
    // FPS target: a fixed 60fps loop can't see that a window's own
    // present/vsync cost varies (e.g. by focus state), so it silently
    // drifts from real-time and starves the audio buffer. `--frame_cap`
    // or no audio device falls back to fixed-FPS pacing.
    if !audio_active {
        window.set_target_fps(opts.frame_cap.unwrap_or(60));
    }

    const GBA_FRAME_SECONDS: f64 = 280896.0 / 16777216.0;
    const MAX_CATCHUP_FRAMES: u32 = 30;

    let mut time_accumulator = 0.0f64;
    let mut last_instant = Instant::now();
    let mut last_audio_stats_log = Instant::now();

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();

        if audio_active && opts.frame_cap.is_none() {
            let dt = now.duration_since(last_instant).as_secs_f64();
            last_instant = now;
            time_accumulator = (time_accumulator + dt).min(GBA_FRAME_SECONDS * MAX_CATCHUP_FRAMES as f64);

            let mut emulated_this_iteration = false;
            while time_accumulator >= GBA_FRAME_SECONDS {
                gba.frame();
                emulated_this_iteration = true;
                time_accumulator -= GBA_FRAME_SECONDS;

                let new_samples = std::mem::take(&mut gba.apu.sample_buffer);
                let resampled = resampler.process(&new_samples);
                let device_samples = to_device_channels(&resampled, device_channels);
                let written = audio_producer.push_slice(&device_samples);
                overrun_count += (device_samples.len() - written) as u64;
            }

            if !emulated_this_iteration {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        } else {
            gba.frame();

            let new_samples = std::mem::take(&mut gba.apu.sample_buffer);
            let resampled = resampler.process(&new_samples);
            let device_samples = to_device_channels(&resampled, device_channels);
            audio_producer.push_slice(&device_samples);
        }

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

        if audio_active && now.duration_since(last_audio_stats_log).as_secs_f64() >= 2.0 {
            last_audio_stats_log = now;
            let underruns = underrun_count.swap(0, Ordering::Relaxed);
            if underruns > 0 || overrun_count > 0 {
                info!("Audio buffer: {} underrun samples, {} overrun samples dropped (last 2s)", underruns, overrun_count);
                overrun_count = 0;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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
