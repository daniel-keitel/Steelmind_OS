use core::alloc::GlobalAlloc;

use alloc::string::String;
use spin::{Mutex, Once};

extern crate alloc;

pub use crate::{print, println};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("User PANIC: {}", info);
    abort(42);
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::os_functions::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::os_functions::_print_fmt_contiguous(format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print_fmt(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let mut out = Out {};
    out.write_fmt(args).unwrap();
}

struct Out {}
impl core::fmt::Write for Out {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        _print(s);
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print_fmt_contiguous(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let mut out = OUT_CONTIGUOUS.lock();
    write!(out.buffer, "{}", args).unwrap();

    _print(out.buffer.as_str());

    out.buffer.clear();
    if out.buffer.capacity() > 4096 {
        out.buffer.shrink_to(4096);
    }
}

struct OutContiguous {
    buffer: String,
}
static OUT_CONTIGUOUS: Mutex<OutContiguous> = Mutex::new(OutContiguous {
    buffer: String::new(),
});

#[inline(always)]
pub fn _print(string: &str) {
    unsafe { (_FP.get().unwrap_unchecked().print)(string.as_ptr(), string.len() as u64) };
}

#[inline]
pub fn abort(exit_code: u64) -> ! {
    unsafe { (_FP.get().unwrap_unchecked().abort)(exit_code) };
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator {};

struct Allocator {}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        (_FP.get().unwrap_unchecked().alloc)(layout.size() as u64, layout.align() as u64)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        (_FP.get().unwrap_unchecked().dealloc)(ptr, layout.size() as u64, layout.align() as u64)
    }
}

#[repr(C)]
pub struct FunctionPointers {
    print: extern "C" fn(*const u8, u64),
    abort: extern "C" fn(u64) -> !,
    alloc: extern "C" fn(u64, u64) -> *mut u8,
    dealloc: extern "C" fn(*mut u8, u64, u64),
}

pub static _FP: Once<&'static FunctionPointers> = Once::new();

#[macro_export]
macro_rules! entry_point {
    ($path:path) => {
        #[doc(hidden)]
        #[export_name = "_start"]
        pub unsafe extern "C" fn __impl_start(fns: *const os_functions::FunctionPointers) -> u64 {
            $crate::os_functions::_FP.call_once(|| unsafe { &*fns });
            let f: fn() -> u64 = $path;
            f()
        }
    };
}
