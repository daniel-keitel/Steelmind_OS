use core::{
    hint,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use alloc::{boxed::Box, vec::Vec};
use x86_64::instructions::hlt;

use crate::{
    acpi::ACPI,
    ass, barrier, get_boot_info,
    memory::MEMORY,
    smp::{cpu_index, get_cld},
    terminal_out::{self, PlacementInfo, TerminalWriter, WindowInfo, TERM},
};

pub fn main() {
    x86_64::instructions::interrupts::int3();

    let ap_count = ACPI.lock().ap_count;
    let id = cpu_index();

    app_test();

    crate::smp::sync_cores_barrier();
    if cpu_index() == 0 {
        MEMORY.lock().log_memory_utilization(log::Level::Debug);
    }
    crate::smp::sync_cores_barrier();

    print_logo();
    timer_test();
    print_logo();

    crate::smp::sync_cores_barrier();
    if cpu_index() == 0 {
        MEMORY.lock().log_memory_utilization(log::Level::Debug);
    }
    crate::smp::sync_cores_barrier();

    if id == 0 {
        static NEW_FRAME_REQUESTED: AtomicBool = AtomicBool::new(true);
        log::info!("Hello from core 0");

        let fb = get_boot_info().framebuffer.as_ref().unwrap();
        log::warn!("Framebuffer info: {fb:#?}");

        crate::terminal_out::switch_to_double_buffer();

        log::info!("Switched to double buffer");
        let int = || {
            NEW_FRAME_REQUESTED.store(true, Ordering::Release);
        };

        crate::apic::get_apic()
            .start_timer(30_000, true, int)
            .unwrap();

        loop {
            while !NEW_FRAME_REQUESTED.fetch_and(false, Ordering::Acquire) {
                hlt();
            }
            REFRESH_COUNTER.fetch_add(1, Ordering::Release);
            crate::terminal_out::push_to_frame_buffer();
        }
    }

    let mut local_writer =
        TerminalWriter::new(get_window_info_for_core(id as usize, ap_count as usize));

    let clear_color = terminal_out::Color::new(
        10,
        100 - (100 * id / (ap_count + 1)) as u8,
        (100 * id / (ap_count + 1)) as u8,
    );
    local_writer.clear_color = clear_color;

    local_writer.set_to_double_buffer();
    local_writer.clear(Some(terminal_out::FontSize::Size16));

    while REFRESH_COUNTER.load(Ordering::Acquire) < 2 {
        hint::spin_loop();
    }

    if id == 1 {
        TERM.lock()
            .set_window_info(get_window_info_for_core(0, ap_count as usize), None);
    }

    barrier!(ap_count);

    // // loop{hlt();}

    local_writer.clear(None);
    if id == 1 {
        let count = ACPI.lock().ap_count;

        for i in 0..100_000_000 {
            let _lock = crate::terminal_out::lock_back_buffer();
            local_writer.print(format_args!(
                "{} frame {} {}\n",
                i,
                REFRESH_COUNTER.load(Ordering::Relaxed),
                count
            ));
        }
    } else if id == 2 {
        local_writer.print(format_args!("Core 2 uses stdout\n"));
        print_logo();
    } else {
        for i in 0..200 {
            local_writer.print(format_args!("[{i}] Hello from core {id}\n"));
        }
    }

    // barrier!(ap_count+1);

    // if cpu_index() != 1 {
    //     loop {
    //         hlt();
    //     }
    // }

    // if cpu_index() == 1 {
    //     // set main term from core 0
    //     TERM.lock()
    //         .set_window_info(get_window_info_for_core(0, ap_count as usize), None)
    // }

    loop {
        // let line = { crate::serial::SERIAL.0.lock().read_line().unwrap() };
        if let Ok(c) = crate::serial::SERIAL.0.lock().read() {
            local_writer.print(format_args!("{}: {:?}\n", c, core::str::from_utf8(&[c])));
        }
    }

    // if cpu_index() == 0 {
    //     // ACPI.lock().log_proccessor_info(log::Level::Info);
    //     log::info!("\n\tSerial line echo");
    // }

    // MEMORY.lock().log_memory_utilization(log::Level::Debug);

    // loop {
    //     // let line = { crate::serial::SERIAL.0.lock().read_line().unwrap() };
    //     if let Ok(c) = crate::serial::SERIAL.0.lock().read() {
    //         log::info!("{}: {:?}", c, core::str::from_utf8(&[c]));
    //     }
    //     hint::spin_loop();
    // }
}

fn get_window_info_for_core(id: usize, count: usize) -> WindowInfo {
    let placement = PlacementInfo {
        x_div: 4,
        y_div: (count + 4 - 1) / 4,
        x_index: id % 4,
        y_index: id / 4,
        x_size: 1,
        y_size: 1,
    };

    WindowInfo::from_placement(&placement)
}

static REFRESH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn timer_interrupt() {
    get_cld().stuff.as_mut().unwrap()[0]
        .downcast_mut::<AtomicU64>()
        .unwrap()
        .fetch_add(1, Ordering::Relaxed);
}

fn allocator_fail_test() {
    for i in 1.. {
        let unit = 1000 * 1000 * 100;
        log::info!("Allocating {}MB", unit * i / 1024 / 1024);
        // crate::allocator::ALLOCATOR.log_heap_stats(log::Level::Info);
        let v = alloc::vec![0u8; unit * i];
        // crate::allocator::ALLOCATOR.log_heap_stats(log::Level::Info);
        drop(v);
        // crate::allocator::ALLOCATOR.log_heap_stats(log::Level::Info);
    }
}

fn timer_test() {
    let stuff = &mut get_cld().stuff;
    *stuff = Some(Vec::new());
    let stuff = stuff.as_mut().unwrap();

    stuff.push(Box::new(AtomicU64::new(0)));

    crate::apic::get_apic()
        .start_timer(200_000 * (cpu_index() as u32 + 1), true, timer_interrupt)
        .unwrap();

    let mut last_count = 0;
    loop {
        let counter: &AtomicU64 = stuff[0].downcast_ref().unwrap();
        let count = counter.load(Ordering::Relaxed);
        if count > last_count {
            last_count += 1;

            log::info!("+ count increased to {: >3} at {: >3}", count, cpu_index());
        }

        if count >= 5 {
            break;
        }
        hint::spin_loop();
    }
}

fn app_test() {
    let user_app = crate::ram_disk::get_file_slice(1);

    let mut resources_a = crate::loader::prepare_application(user_app);
    let mut resources_b = crate::loader::prepare_application(user_app);

    ass!(crate::loader::run(&mut resources_a), ==, 0);
    ass!(crate::loader::run(&mut resources_b), ==, 0);
    ass!(crate::loader::run(&mut resources_a), ==, 42);
}

fn print_logo() {
    let (width, pixels) = {
        let image_bytes = crate::ram_disk::get_file_slice(2);
        let mut decoder = zune_jpeg::JpegDecoder::new(image_bytes);

        let pixels = decoder.decode().unwrap();

        (decoder.dimensions().unwrap().0, pixels)
    };

    let it = pixels
        .chunks(3)
        .map(|pixel| crate::terminal_out::Color::new(pixel[0], pixel[1], pixel[2]));

    crate::terminal_out::Stdout::acquire().print_pixels(width, it);
}
