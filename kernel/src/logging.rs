use core::sync::atomic::AtomicU64;

use log::{Level, LevelFilter, Metadata, Record};

use core::sync::atomic::Ordering;

use crate::{smp::try_get_cld, terminal_out::Color};

static LOGGER: KernelLogger = KernelLogger;
struct KernelLogger;

const BG_COLOR: Color = Color::black();
const FG_COLOR: Color = Color::white();

impl log::Log for KernelLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let level = record.metadata().level();

        let serial_level: LevelFilter =
            unsafe { core::mem::transmute(SERIAL_LOG_LEVEL.load(Ordering::Relaxed)) };
        let graphics_level: LevelFilter =
            unsafe { core::mem::transmute(GRAPHICS_LOG_LEVEL.load(Ordering::Relaxed)) };

        if level <= serial_level {
            let _lock = SERIAL_WRITE_LOCK.lock();
            crate::serial_print!("[{:<5}", record.level());

            if let Some(ref mut cld) = try_get_cld() {
                crate::serial_print!(" {: >2}", cld.cpu_index);
            }
            if let (Some(file), Some(line)) = (record.file(), record.line()) {
                crate::serial_println!(" {}:{}] {}", file, line, record.args());
            } else {
                crate::serial_println!(" {}", record.args());
            }
        }

        if level <= graphics_level {
            let info_color = match level {
                Level::Error => Color::new(255, 50, 50),
                Level::Warn => Color::new(255, 200, 0),
                Level::Info => Color::new(220, 220, 220),
                Level::Debug => Color::new(0, 40, 255),
                Level::Trace => Color::new(130, 130, 130),
            };

            let mut stdout = crate::terminal_out::Stdout::acquire();

            let foreground = stdout.foreground();
            let background = stdout.background();
            let font = stdout.font_weight();

            stdout.set_foreground(foreground);
            stdout.set_background(background);
            stdout.set_font_weight(crate::terminal_out::FontWeight::Light);
            crate::print!(stdout; "[");
            stdout.set_foreground(info_color);
            stdout.set_font_weight(crate::terminal_out::FontWeight::Bold);
            crate::print!(stdout;"{:<5}", record.level());
            stdout.set_foreground(foreground);
            if let Some(ref mut cld) = try_get_cld() {
                crate::print!(stdout;" {: >2}", cld.cpu_index);
            }
            stdout.set_font_weight(crate::terminal_out::FontWeight::Light);

            if let (Some(file), Some(line)) = (record.file(), record.line()) {
                if file.len() > 50 {
                    crate::print!(stdout;" |{}:{}", &file[file.len() - 50..], line);
                } else {
                    crate::print!(stdout;" {}:{}", file, line);
                }
            }

            crate::print!(stdout;"] - ");
            stdout.set_font_weight(crate::terminal_out::FontWeight::Regular);

            crate::println!(stdout;"{}", record.args());

            stdout.set_foreground(foreground);
            stdout.set_background(background);
            stdout.set_font_weight(font);
        }
    }

    fn flush(&self) {
        crate::serial::SERIAL.1.lock().flush();
    }
}

pub fn init_logging(serial_level: LevelFilter, graphics_level: LevelFilter) {
    set_serial_log_level(serial_level);
    set_graphics_log_level(graphics_level);

    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Trace))
        .expect("Logger setup failed");

    log::debug!("Logging initialized");
}

pub fn set_serial_log_level(level: LevelFilter) {
    SERIAL_LOG_LEVEL.store(level as u64, Ordering::Release);
}

pub fn set_graphics_log_level(level: LevelFilter) {
    GRAPHICS_LOG_LEVEL.store(level as u64, Ordering::Release);
}

static SERIAL_LOG_LEVEL: AtomicU64 = AtomicU64::new(0);
static GRAPHICS_LOG_LEVEL: AtomicU64 = AtomicU64::new(0);

static SERIAL_WRITE_LOCK: spin::Mutex<()> = spin::Mutex::new(());
