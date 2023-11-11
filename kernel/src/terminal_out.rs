use crate::{get_boot_info, serial_mark};
use alloc::vec;
use bootloader_api::info::PixelFormat;
use core::hint;
use core::sync::atomic::AtomicBool;
use core::{
    fmt::{self, Write},
    ptr, slice,
};
use lazy_static::lazy_static;
use noto_sans_mono_bitmap::{get_raster, get_raster_width, RasterizedChar};
use spin::{Mutex, Once};
use x86_64::instructions::interrupts;

pub use noto_sans_mono_bitmap::FontWeight;
pub use noto_sans_mono_bitmap::RasterHeight as FontSize;

const LINE_SPACING: usize = 0;
const LETTER_SPACING: usize = 0;
const SIDE_PADDING: usize = 0;

const TAB_SIZE: usize = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[allow(dead_code)]
impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn black() -> Self {
        Self { r: 0, g: 0, b: 0 }
    }

    pub const fn white() -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
        }
    }

    #[allow(clippy::cast_sign_loss)]
    pub const fn lerp(self, other: Self, alpha_1024: u32) -> Self {
        let r =
            (self.r as i32 + ((other.r as i32 - self.r as i32) * alpha_1024 as i32) / 1024) as u8;
        let g =
            (self.g as i32 + ((other.g as i32 - self.g as i32) * alpha_1024 as i32) / 1024) as u8;
        let b =
            (self.b as i32 + ((other.b as i32 - self.b as i32) * alpha_1024 as i32) / 1024) as u8;
        Self { r, g, b }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementInfo {
    pub x_div: usize,
    pub y_div: usize,
    pub x_index: usize,
    pub y_index: usize,
    pub x_size: usize,
    pub y_size: usize,
}

const FULL_SCREEN_PLACEMENT: PlacementInfo = PlacementInfo {
    x_div: 1,
    y_div: 1,
    x_index: 0,
    y_index: 0,
    x_size: 1,
    y_size: 1,
};

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub byte_offset: usize,
    pub width: usize,
    pub height: usize,
    pub pixel_format: PixelFormat,
    pub bytes_per_pixel: usize,
    pub stride: usize,
}

impl WindowInfo {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        serial_mark!("new window info x:{x:?} y:{y:?} width:{width:?} height:{height:?}");
        let fb_info = crate::get_boot_info().framebuffer.as_mut().unwrap().info();
        crate::ass!(x + width, <=, fb_info.width);
        crate::ass!(y + height, <=, fb_info.width);

        let pixel_offset = y * fb_info.stride + x;
        let byte_offset = pixel_offset * fb_info.bytes_per_pixel;

        Self {
            byte_offset,
            width,
            height,
            pixel_format: fb_info.pixel_format,
            bytes_per_pixel: fb_info.bytes_per_pixel,
            stride: fb_info.stride,
        }
    }

    pub fn from_placement(placement: &PlacementInfo) -> Self {
        serial_mark!("from_placement: {placement:?}");
        let fb_info = crate::get_boot_info().framebuffer.as_mut().unwrap().info();

        let start_pixel_x = placement.x_index * fb_info.width / placement.x_div;
        let stop_pixel_x = (placement.x_index + placement.x_size) * fb_info.width / placement.x_div;

        let start_pixel_y = placement.y_index * fb_info.height / placement.y_div;
        let stop_pixel_y =
            (placement.y_index + placement.y_size) * fb_info.height / placement.y_div;

        serial_mark!("start_pixel_x:{start_pixel_x} stop_pixel_x:{stop_pixel_x} start_pixel_y:{start_pixel_y} stop_pixel_y:{stop_pixel_y}");

        Self::new(
            start_pixel_x,
            start_pixel_y,
            stop_pixel_x - start_pixel_x,
            stop_pixel_y - start_pixel_y,
        )
    }
}

pub struct TerminalWriter {
    buffer_base_ptr: *mut u8,
    buffer: &'static mut [u8],
    info: WindowInfo,
    x_pos: usize,
    y_pos: usize,
    font_height: FontSize,
    line_height: usize,
    pub font_weight: FontWeight,
    pub foreground: Color,
    pub background: Color,
    pub clear_color: Color,
}

fn construct_buffer(buffer_base_ptr: *mut u8, info: &WindowInfo) -> &'static mut [u8] {
    unsafe {
        slice::from_raw_parts_mut(
            buffer_base_ptr.add(info.byte_offset),
            info.stride * info.height * info.bytes_per_pixel,
        )
    }
}

impl TerminalWriter {
    pub fn new(info: WindowInfo) -> Self {
        let buffer_base_ptr = crate::get_boot_info()
            .framebuffer
            .as_mut()
            .unwrap()
            .buffer_mut()
            .as_mut_ptr();
        Self {
            buffer_base_ptr,
            buffer: construct_buffer(buffer_base_ptr, &info),
            info,
            x_pos: 0,
            y_pos: 0,
            font_height: FontSize::Size24,
            line_height: 24 + LETTER_SPACING,
            font_weight: FontWeight::Regular,
            foreground: Color::white(),
            background: Color::black(),
            clear_color: Color::new(70, 60, 60),
        }
    }

    pub fn set_to_double_buffer(&mut self) {
        loop {
            if let Some(db) = DOUBLE_BUFFER.get() {
                self.buffer_base_ptr = db.back_buffer;
                break;
            }
            hint::spin_loop();
        }
        self.buffer = construct_buffer(self.buffer_base_ptr, &self.info);
    }

    pub fn window_info(&self) -> &WindowInfo {
        &self.info
    }

    pub fn set_window_info(&mut self, info: WindowInfo, font_height: Option<FontSize>) {
        self.info = info;
        self.buffer = construct_buffer(self.buffer_base_ptr, &self.info);
        serial_mark!("Setting window info to {:?}", &self.info);
        self.clear(font_height);
    }

    pub fn clear(&mut self, font_height: Option<FontSize>) {
        self.x_pos = 0;
        self.y_pos = 0;

        for y in 0..self.info.height {
            for x in 0..self.info.width {
                self.write_pixel(x, y, self.clear_color);
            }
        }

        if let Some(font_height) = font_height {
            self.font_height = font_height;
            self.line_height = self.font_height.val() + LINE_SPACING;
        }
    }

    fn newline(&mut self) {
        self.y_pos += self.line_height;
        self.carriage_return();

        if self.y_pos + self.line_height > self.info.height {
            self.y_pos = 0;
            self.clear_color =
                Color::new(self.clear_color.b, self.clear_color.r, self.clear_color.g);
        }

        for y in self.y_pos..(self.y_pos + self.line_height * 2).min(self.info.height) {
            for x in 0..self.info.width {
                let color = if (x + y) & 1 == 0 {
                    self.clear_color
                } else {
                    Color::new(
                        self.clear_color.r / 2,
                        self.clear_color.g / 2,
                        self.clear_color.b / 2,
                    )
                };
                self.write_pixel(x, y, color);
            }
        }
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    fn tab(&mut self) {
        let new_spaces = if self.x_pos == 0 {
            4
        } else {
            let char_width = get_raster_width(self.font_weight, self.font_height) + LETTER_SPACING;
            let char_count = (self.x_pos - SIDE_PADDING) / char_width;
            let final_char_count = ((char_count + 1 + TAB_SIZE - 1) / TAB_SIZE) * TAB_SIZE;
            final_char_count - char_count
        };

        for _ in 0..new_spaces {
            self.write_raw_char(' ');
        }
    }

    pub fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            '\t' => self.tab(),
            c => self.write_raw_char(c),
        }
    }

    #[inline]
    fn get_rasterized_char(&self, c: char) -> (RasterizedChar, usize) {
        (
            get_raster(c, self.font_weight, self.font_height)
                .unwrap_or_else(|| get_raster('�', self.font_weight, self.font_height).unwrap()),
            get_raster_width(self.font_weight, self.font_height),
        )
    }

    fn write_raw_char(&mut self, c: char) {
        let (rasterized_char, char_width) = self.get_rasterized_char(c);
        if self.x_pos == 0 {
            self.x_pos = SIDE_PADDING;
        }
        if self.x_pos + char_width >= self.info.width - SIDE_PADDING {
            self.newline();
        }

        self.print_rasterized_char(&rasterized_char);

        self.x_pos += char_width + LETTER_SPACING;
    }

    fn print_rasterized_char(&mut self, rasterized_char: &RasterizedChar) {
        for (y, row) in rasterized_char.raster().iter().enumerate() {
            for (x, byte) in row.iter().enumerate() {
                let color = self
                    .background
                    .lerp(self.foreground, *byte as u32 * 1024 / 255);
                self.write_pixel(self.x_pos + x, self.y_pos + y, color);
            }
        }
    }

    pub fn print_pixels(&mut self, width: usize, mut colors: impl Iterator<Item = Color>) {
        let x_offset = self.x_pos;
        let mut y_offset = 0;

        'outer: loop {
            for x in 0..width {
                if let Some(color) = colors.next() {
                    let x = x + x_offset;
                    if x >= self.info.width {
                        continue;
                    }
                    let y = self.y_pos + y_offset;
                    self.write_pixel(x, y, color);
                } else {
                    break 'outer;
                }
            }
            y_offset += 1;
            if y_offset >= self.line_height {
                self.newline();
                y_offset = 0;
            }
        }
        if y_offset != 0 {
            self.newline();
        }
    }

    #[inline]
    #[allow(clippy::cast_ptr_alignment)]
    fn write_pixel(&mut self, x: usize, y: usize, color: Color) {
        let pixel_offset = y * self.info.stride + x;
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let byte_offset = pixel_offset * bytes_per_pixel;

        match self.info.pixel_format {
            PixelFormat::Rgb => {
                self.buffer[byte_offset] = color.r;
                self.buffer[byte_offset + 1] = color.g;
                self.buffer[byte_offset + 2] = color.b;
            }
            PixelFormat::Bgr => {
                self.buffer[byte_offset] = color.b;
                self.buffer[byte_offset + 1] = color.g;
                self.buffer[byte_offset + 2] = color.r;
            }
            _ => {
                self.buffer[byte_offset] =
                    ((color.r as u16 + color.g as u16 + color.b as u16) / 3) as u8;
            }
        };
    }

    pub fn print(&mut self, args: fmt::Arguments) {
        self.write_fmt(args).unwrap();
    }
}

impl fmt::Write for TerminalWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}

unsafe impl Send for TerminalWriter {}
unsafe impl Sync for TerminalWriter {}

lazy_static! {
    pub static ref TERM: Mutex<TerminalWriter> = Mutex::new(TerminalWriter::new(
        WindowInfo::from_placement(&FULL_SCREEN_PLACEMENT)
    ));
    static ref EMERGENCY_PANIC_TERM: Mutex<TerminalWriter> = Mutex::new(TerminalWriter::new(
        WindowInfo::from_placement(&FULL_SCREEN_PLACEMENT)
    ));
}

static SWAP_LOCK: spin::Mutex<()> = spin::Mutex::new(());
static PANICKED_STOP_PRINTING: AtomicBool = AtomicBool::new(false);
static DOUBLE_BUFFER: Once<DoubleBuffer> = Once::new();
static BACK_BUFFER_LOCK: spin::Mutex<()> = spin::Mutex::new(()); // Todo change to fair multi write single read lock

struct DoubleBuffer {
    back_buffer: *mut u8,
    front_buffer: *mut u8,
    length: usize,
}

unsafe impl Send for DoubleBuffer {}
unsafe impl Sync for DoubleBuffer {}

pub fn switch_to_double_buffer() {
    DOUBLE_BUFFER.call_once(|| {
        let frame_buffer = get_boot_info().framebuffer.as_mut().unwrap();
        let info = frame_buffer.info();
        // bootloader bug mitigation (the size of the framebuffer is the whole vram)
        let frame_buffer_size = info
            .byte_len
            .min(info.stride * info.bytes_per_pixel * info.height);

        log::trace!("Initializing double buffer (size{})", frame_buffer_size);

        let new_back_buffer = vec![0; frame_buffer_size].leak();
        {
            let term = &mut TERM.lock();
            term.buffer_base_ptr = new_back_buffer.as_mut_ptr();
            term.buffer = construct_buffer(term.buffer_base_ptr, &term.info);
            DoubleBuffer {
                back_buffer: &mut term.buffer[0],
                front_buffer: &mut frame_buffer.buffer_mut()[0],
                length: frame_buffer_size,
            }
        }
    });
}

pub fn push_to_frame_buffer() {
    unsafe {
        if PANICKED_STOP_PRINTING.load(core::sync::atomic::Ordering::Acquire) {
            return;
        }
        let fb_info = get_boot_info().framebuffer.as_ref().unwrap().info();
        let db = DOUBLE_BUFFER.get().unwrap(); // Required for panic handling
        let _term = TERM.lock(); // Lock terminal to prevent tearing from main out
        let _back_buffer = BACK_BUFFER_LOCK.lock(); // Lock back buffer to prevent tearing from other cores (opt in)
        interrupts::without_interrupts(|| {
            let _swap = SWAP_LOCK.lock();
            for y in 0..fb_info.height {
                let offset = y * fb_info.stride * fb_info.bytes_per_pixel;
                // more efficient if stride is big (may be in the thousands)
                ptr::copy_nonoverlapping(
                    db.back_buffer.add(offset),
                    db.front_buffer.add(offset),
                    fb_info.width * fb_info.bytes_per_pixel,
                );
                // let _ =  ptr::read_volatile(db.front_buffer.add(offset));
            }
            // ptr::copy_nonoverlapping(db.front_buffer, db.back_buffer, db.length);
            //volatile_copy_nonoverlapping_memory(db.front_buffer, db.back_buffer, db.length);
        });
    }
}

// lock back buffer to write to it without tearing (The buffer is multi write single read)
pub fn lock_back_buffer() -> spin::MutexGuard<'static, ()> {
    BACK_BUFFER_LOCK.lock()
}

pub fn panic_print(callback: impl FnOnce(&mut TerminalWriter)) {
    static PANIC_PRINT_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    PANICKED_STOP_PRINTING.store(true, core::sync::atomic::Ordering::Release);
    let _lock = PANIC_PRINT_LOCK.lock();

    if let Some(db) = DOUBLE_BUFFER.get() {
        let mut time_out = 1_000_000;
        let _swap = loop {
            let x = SWAP_LOCK.try_lock();
            if x.is_some() {
                break x;
            }
            if time_out == 0 {
                break None;
            }
            time_out -= 1;
            hint::spin_loop();
        };
        let term = &mut EMERGENCY_PANIC_TERM.lock();
        term.buffer = unsafe { slice::from_raw_parts_mut(db.front_buffer, db.length) };
        callback(term);
    } else if let Some(ref mut term) = TERM.try_lock() {
        callback(term);
    } else {
        let term = &mut EMERGENCY_PANIC_TERM.lock();
        callback(term);
    }
}

pub struct Stdout {
    inner: spin::MutexGuard<'static, TerminalWriter>,
}

impl Stdout {
    #[inline]
    pub fn acquire() -> Self {
        Self { inner: TERM.lock() }
    }

    pub fn print(&mut self, args: fmt::Arguments) {
        if PANICKED_STOP_PRINTING.load(core::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.inner.write_fmt(args).unwrap();
    }

    pub fn print_pixels(&mut self, width: usize, colors: impl Iterator<Item = Color>) {
        if PANICKED_STOP_PRINTING.load(core::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.inner.print_pixels(width, colors);
    }

    pub fn clear(&mut self, font_height: Option<FontSize>) {
        if PANICKED_STOP_PRINTING.load(core::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.inner.clear(font_height);
    }

    pub fn set_font_weight(&mut self, weight: FontWeight) {
        self.inner.font_weight = weight;
    }

    pub fn font_weight(&self) -> FontWeight {
        self.inner.font_weight
    }

    pub fn set_foreground(&mut self, color: Color) {
        self.inner.foreground = color;
    }

    pub fn foreground(&self) -> Color {
        self.inner.foreground
    }

    pub fn set_background(&mut self, color: Color) {
        self.inner.background = color;
    }

    pub fn background(&self) -> Color {
        self.inner.background
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.inner.clear_color = color;
    }

    pub fn clear_color(&self) -> Color {
        self.inner.clear_color
    }
}
