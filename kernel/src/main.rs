#![no_std]
#![no_main]
#![feature(core_intrinsics)]
#![feature(abi_x86_interrupt)]
#![feature(naked_functions)]
#![allow(dead_code)]
// #![allow(unused_imports)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::if_not_else)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::struct_field_names)]

mod acpi;
mod allocator;
mod apic;
mod common_main;
mod constants;
mod interrupts;
mod loader;
mod logging;
mod macros;
mod memory;
mod pit;
mod ram_disk;
mod serial;
mod smp;
mod terminal_out;
mod tester;
mod tests;

extern crate alloc;

use bootloader_api::{BootInfo, BootloaderConfig};
use spin::Once;
use x86_64::instructions::hlt;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(bootloader_api::config::Mapping::Dynamic);
    config.kernel_stack_size = constants::KERNEL_STACK_SIZE;
    config.mappings.dynamic_range_start = Some(constants::v::KERNEL_DYNAMIC_START);
    config.mappings.dynamic_range_end = Some(constants::v::KERNEL_DYNAMIC_END);
    config
};

bootloader_api::entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

// initialization order:
// Set global boot info
// gdt_and_exceptions_bsp: to be able to handle exceptions (which shouldn't happen at this point)
// initialize logging (includes serial port)
// change pat so write_through + cache_disabled is write combining (workaround it would be better to use the pat bit in huge pages)
// set frame buffer to write combining (way faster than default on real hardware)
// (optional clear screen)
// (optional assert stuff we can print nice error messages)
// heap (lazily initialized) a lot of stuff needs a heap (could be optimized but the acpi currently needs a heap, and by extension the core local storage)
// acpi (needs heap, lazily initialized)
// apic creation (needs acpi, required for local interrupts)
// core local storage (needs apic (use try_get_cli in exception handlers since this initialization is so late))
// apic init (needed for apic to function)
// smp (needs apic, multi core support (initializes aps))
// enable interrupts

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    BOOT_INFO.call_once(|| boot_info as *mut _ as u64);

    interrupts::init_gdt_and_exceptions_bsp();

    logging::init_logging(log::LevelFilter::Trace, log::LevelFilter::Trace);

    memory::change_pat_so_write_through_plus_cache_disabled_is_write_combining();
    memory::set_frame_buffer_cache_to_write_combining();

    terminal_out::Stdout::acquire().clear(Some(terminal_out::FontSize::Size20));
    assert_boot_info();

    apic::create();
    smp::initialize_own_core_local_data(smp::CoreLocalData::default());
    apic::init();

    smp::init_smp();

    x86_64::instructions::interrupts::enable();

    smp::sync_cores_barrier();

    memory::MEMORY
        .lock()
        .log_memory_utilization(log::Level::Info);

    log::info!("Booted successfully");

    smp::sync_cores_barrier();

    #[cfg(feature = "testing")]
    tester::bsp_test_main();

    #[cfg(not(feature = "testing"))]
    common_main::main();

    loop {
        hlt();
    }
}

fn assert_boot_info() {
    use crate::constants::v;
    let boot_info = get_boot_info();

    boot_info
        .physical_memory_offset
        .as_mut()
        .expect("Physical memory offset not set!");

    ass!(boot_info.framebuffer.as_mut().unwrap().buffer().len(), ==, boot_info.framebuffer.as_mut().unwrap().info().byte_len);

    ass!(boot_info.framebuffer.as_mut().unwrap().buffer().as_ptr() as u64, >, v::KERNEL_DYNAMIC_START);
    ass!(*boot_info.ramdisk_addr.as_ref().unwrap_or(&0), >, v::KERNEL_DYNAMIC_START);
    ass!(*boot_info.physical_memory_offset.as_ref().unwrap_or(&0), >, v::KERNEL_DYNAMIC_START);

    ass!(boot_info.framebuffer.as_mut().unwrap().buffer().as_ptr() as u64, <, v::KERNEL_DYNAMIC_END);
    ass!(*boot_info.ramdisk_addr.as_ref().unwrap(), <, v::KERNEL_DYNAMIC_END);
    ass!(*boot_info.physical_memory_offset.as_ref().unwrap(), <, v::KERNEL_DYNAMIC_END);

    ram_disk::assert_soundness();
}

#[cfg(not(feature = "testing"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;

    terminal_out::panic_print(|term| {
        term.foreground = terminal_out::Color::new(0xFF, 0x00, 0x00);
        term.background = terminal_out::Color::new(0x70, 0x70, 0x00);
        term.write_fmt(format_args!("\n{info}\n")).unwrap();
    });
    log::error!("\n\t{info}");

    #[allow(clippy::empty_loop)]
    loop {}
}

static BOOT_INFO: Once<u64> = Once::new();

#[inline]
pub fn get_boot_info() -> &'static mut BootInfo {
    unsafe { &mut *(*BOOT_INFO.get().unwrap_unchecked() as *mut BootInfo) }
}
