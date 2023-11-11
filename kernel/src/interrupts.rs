use core::sync::atomic::AtomicU64;

use alloc::{boxed::Box, vec::Vec};
use lazy_static::lazy_static;
use x86_64::{
    instructions::{port::Port, tables::load_tss},
    registers::segmentation::{Segment, CS, DS, SS},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
        tss::TaskStateSegment,
    },
    VirtAddr,
};

use crate::{
    apic::get_apic,
    constants::MAX_CORES,
    serial_println,
    smp::{get_cld, try_get_cld},
};

macro_rules! interrupt_handler___ {
    ($idt:ident, $x:ident) => {{
        extern "x86-interrupt" fn handler(stack_frame: InterruptStackFrame) {
            let cld = try_get_cld();
            panic!(
                "EXCEPTION: {}\n{:#?}\nCore local data: {:x?}",
                stringify!($x),
                stack_frame,
                cld
            );
        }

        $idt.$x.set_handler_fn(handler);
    }};
}

macro_rules! interrupt_handler_ec {
    ($idt:ident, $x:ident) => {{
        extern "x86-interrupt" fn handler(stack_frame: InterruptStackFrame, ec: u64) {
            let cld = try_get_cld();
            panic!(
                "EXCEPTION: {} error code:{}\n{:#?}\nCore local data: {:x?}",
                stringify!($x),
                ec,
                stack_frame,
                cld
            );
        }

        $idt.$x.set_handler_fn(handler);
    }};
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        interrupt_handler_ec!(idt, alignment_check);
        interrupt_handler___!(idt, bound_range_exceeded);
        interrupt_handler_ec!(idt, cp_protection_exception);
        interrupt_handler___!(idt, device_not_available);
        interrupt_handler___!(idt, divide_error);
        interrupt_handler_ec!(idt, general_protection_fault);
        interrupt_handler___!(idt, hv_injection_exception);
        interrupt_handler___!(idt, invalid_opcode);
        interrupt_handler_ec!(idt, invalid_tss);
        interrupt_handler___!(idt, non_maskable_interrupt);
        interrupt_handler___!(idt, overflow);
        interrupt_handler_ec!(idt, security_exception);
        interrupt_handler_ec!(idt, segment_not_present);
        interrupt_handler___!(idt, simd_floating_point);
        interrupt_handler_ec!(idt, stack_segment_fault);
        interrupt_handler___!(idt, virtualization);
        interrupt_handler_ec!(idt, vmm_communication_exception);
        interrupt_handler___!(idt, x87_floating_point);

        idt.page_fault.set_handler_fn(page_fault_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(0);
        }
        idt[32].set_handler_fn(timer_interrupt);
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt
    };
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[0] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

            let stack_start = VirtAddr::from_ptr(unsafe { &STACK });
            stack_start + STACK_SIZE
        };
        tss
    };
    static ref AP_TSS: [TaskStateSegment; MAX_CORES as usize - 1] = {
        let mut tss_arr = [TaskStateSegment::new(); MAX_CORES as usize - 1];
        for tss in &mut tss_arr {
            tss.interrupt_stack_table[0] = {
                const STACK_SIZE: usize = 4096 * 5;
                let stack = Box::leak(Box::new([0u8; STACK_SIZE]));

                let stack_start = VirtAddr::from_ptr(stack.as_ptr());
                stack_start + STACK_SIZE
            };
        }
        tss_arr
    };
    static ref GDT: (GlobalDescriptorTable, SegmentSelector, SegmentSelector) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
        let tss_selector = gdt.add_entry(Descriptor::tss_segment(&TSS));
        (gdt, code_selector, tss_selector)
    };
    static ref AP_GDT: Vec<(GlobalDescriptorTable, SegmentSelector, SegmentSelector)> = {
        AP_TSS
            .iter()
            .map(|tss| {
                let mut gdt = GlobalDescriptorTable::new();
                let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
                let tss_selector = gdt.add_entry(Descriptor::tss_segment(tss));
                (gdt, code_selector, tss_selector)
            })
            .collect()
    };
}

fn remap_and_disable_pic(offset1: u8, offset2: u8) {
    let mut pic1_cmd: Port<u8> = Port::new(0x20);
    let mut pic1_data: Port<u8> = Port::new(0x21);
    let mut pic2_cmd: Port<u8> = Port::new(0xA0);
    let mut pic2_data: Port<u8> = Port::new(0xA1);
    let mut wait_port: Port<u8> = Port::new(0x80);

    unsafe {
        pic1_cmd.write(0x11); // starts the initialization sequence (in cascade mode)
        wait_port.write(0);
        pic2_cmd.write(0x11);
        wait_port.write(0);
        pic1_data.write(offset1); // ICW2: Master PIC vector offset
        wait_port.write(0);
        pic2_data.write(offset2); // ICW2: Slave PIC vector offset
        wait_port.write(0);
        pic1_data.write(4); // ICW3: tell Master PIC that there is a slave PIC at IRQ2 (0000 0100)
        wait_port.write(0);
        pic2_data.write(2); // ICW3: tell Slave PIC its cascade identity (0000 0010)
        wait_port.write(0);

        pic1_data.write(0x01); // ICW4: have the PICs use 8086 mode (and not 8080 mode)
        wait_port.write(0);
        pic2_data.write(0x01);
        wait_port.write(0);

        pic1_data.write(0xff);
        pic2_data.write(0xff);
    }
}

pub fn init_gdt_and_exceptions_ap(ap_index: u64) {
    let gdt = &AP_GDT[ap_index as usize];
    gdt.0.load();
    unsafe {
        SS::set_reg(SegmentSelector(0));
        DS::set_reg(SegmentSelector(0));
        CS::set_reg(gdt.1);
        load_tss(gdt.2);
    }
    IDT.load();
}

pub fn init_gdt_and_exceptions_bsp() {
    GDT.0.load();
    unsafe {
        SS::set_reg(SegmentSelector(0));
        DS::set_reg(SegmentSelector(0));
        CS::set_reg(GDT.1);
        load_tss(GDT.2);
    }
    IDT.load();
    remap_and_disable_pic(32, 32 + 8);
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let cld = try_get_cld();
    let cr2 = x86_64::registers::control::Cr2::read();
    panic!(
        "EXCEPTION: PAGE FAULT\n{:#?}\n{:?}\n{:x?}\nCore local data: {:x?}",
        stack_frame, error_code, cr2, cld
    );
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    let cld = try_get_cld();
    panic!(
        "EXCEPTION: DOUBLE FAULT\n{:#?}\nCore local data: {:x?}",
        stack_frame, cld
    );
}

pub static TIMER_COUNTER: AtomicU64 = AtomicU64::new(0);

extern "x86-interrupt" fn timer_interrupt(_stack_frame: InterruptStackFrame) {
    if let Some(callback) = get_cld().apic_timer_interrupt_function {
        callback();
    }
    get_apic().signal_end_of_interrupt();
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {
    let cld = try_get_cld();
    serial_println!("break_point cld:{:?}", cld); // TODO: lock free
}
