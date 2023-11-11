use core::{
    any::Any,
    cell::OnceCell,
    num::NonZeroU64,
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};

use alloc::{boxed::Box, vec::Vec};
use spin::{Barrier, Once};
use x86_64::{
    align_up,
    instructions::hlt,
    structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB},
    PhysAddr, VirtAddr,
};

use crate::{
    acpi::ACPI,
    apic::{
        get_apic,
        ipi::{create_send_init_cmd, create_startup_cmd},
    },
    ass,
    constants::{v, KERNEL_STACK_SIZE, MAX_CORES},
    interrupts,
    memory::MEMORY,
};

static AP_CORE_COUNTER: AtomicU64 = AtomicU64::new(0);
static AP_STARTUP_DONE_COUNTER: AtomicU64 = AtomicU64::new(0);

const CODE: &[u8] = include_bytes!("../smp_trampoline/ap.bin");
pub fn init_smp() {
    log::info!("Initializing smp...");
    // ; 0x0A00 u32 address of the l4 page table
    // ; 0x0B00 u64 address of the atomic core counter
    // ; 0x0B10 u64 stride of the stack
    // ; 0x0B20 u64 base address of the stack
    // ; 0x0B30 u64 address of the entry point for rust kernel ap function
    allocate_stacks();

    let l4_page_table_phys_addr = crate::memory::active_level_4_table_phys_addr();
    ass!(l4_page_table_phys_addr, <, 0xffff_ffff, "page table is not addressable with 32bits");

    let atomic_core_counter_addr = AP_CORE_COUNTER.as_ptr() as u64;

    let stack_stride = calc_stack_stride();

    let stack_base = v::KERNEL_AP_STACKS + stack_stride - 1;

    let entry_function_addr = (ap_entry_fn as *const ()) as u64;

    log::debug!("Mapping smp trampoline");

    // SAFETY WARNING null ptr mapped: dereferencing a null ptr is now allowed
    {
        let mut mem = MEMORY.lock();
        unsafe {
            mem.map_frame(
                Page::<Size4KiB>::from_start_address(VirtAddr::new(0)).unwrap(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                PhysFrame::from_start_address(PhysAddr::new(0)).unwrap(),
            );
        };
    }
    unsafe {
        #[allow(clippy::zero_ptr)]
        ptr::copy_nonoverlapping(CODE.as_ptr(), 0 as *mut u8, CODE.len());

        *(0x0A00 as *mut u64) = l4_page_table_phys_addr;
        *(0x0B00 as *mut u64) = atomic_core_counter_addr;
        *(0x0B10 as *mut u64) = stack_stride;
        *(0x0B20 as *mut u64) = stack_base;
        *(0x0B30 as *mut u64) = entry_function_addr;
    }

    startup_aps();

    let ap_core_count = ACPI.lock().ap_count;
    let mut time_out = 1_000_000;

    while AP_STARTUP_DONE_COUNTER.load(Ordering::Acquire) < ap_core_count && time_out > 0 {
        core::hint::spin_loop();
        time_out -= 1;
    }

    {
        let mut mem = MEMORY.lock();
        unsafe { mem.unmap(Page::<Size4KiB>::from_start_address(VirtAddr::new(0)).unwrap()) };
    }

    if time_out == 0 {
        panic!("AP startup timed out");
    } else {
        log::info!("All aps started");
    }
}

fn allocate_stacks() {
    log::trace!("Allocating stacks for APs");
    let ap_core_count = ACPI.lock().ap_count;
    let mut mem = MEMORY.lock();

    let pages_per_core = align_up(KERNEL_STACK_SIZE, 4096) / 4096;

    let mut virt_addr = v::KERNEL_AP_STACKS;

    for _ in 0..ap_core_count {
        virt_addr += 4096; // add guard page
        for _ in 0..pages_per_core {
            let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(virt_addr)).unwrap();
            mem.map_ram_kernel(page, PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE);
            virt_addr += 4096;
        }
    }
    log::trace!("Stacks allocated for aps");
}

const fn calc_stack_stride() -> u64 {
    let pages_per_core = align_up(KERNEL_STACK_SIZE, 4096) / 4096;
    (pages_per_core + 1) * 4096
}

fn startup_aps() {
    log::debug!("Starting APs: sending init and startup commands");

    let mut apic = get_apic();

    let init_cmd = create_send_init_cmd();
    let startup_cmd = create_startup_cmd(0);
    // println!("init_cmd: {:#x} {:#?}", init_cmd.0, init_cmd);
    // println!("startup_cmd: {:#x} {:#?}", startup_cmd.0, startup_cmd);

    apic.write_interrupt_command(init_cmd);
    crate::pit::delay(10_000).unwrap();
    apic.write_interrupt_command(startup_cmd);
    crate::pit::delay(200).unwrap();
    apic.write_interrupt_command(startup_cmd);

    log::debug!("Starting APs: commands sent");
}

unsafe extern "C" fn ap_entry_fn(ap_index: u64) -> ! {
    AP_STARTUP_DONE_COUNTER.fetch_add(1, core::sync::atomic::Ordering::AcqRel);

    log::info!(
        "Core started: index({}) apic_id({})",
        ap_index + 1,
        get_apic().id()
    );

    interrupts::init_gdt_and_exceptions_ap(ap_index);

    log::debug!(
        "Exceptions setup: index({}) apic_id({})",
        ap_index + 1,
        get_apic().id()
    );

    initialize_own_core_local_data(CoreLocalData {
        cpu_index: ap_index + 1,
        ..Default::default()
    });

    log::debug!(
        "Core local data initialized: index({}) apic_id({})",
        ap_index + 1,
        get_apic().id()
    );

    crate::apic::init();

    x86_64::instructions::interrupts::enable();

    log::info!(
        "Core initialized: index({}) apic_id({})",
        ap_index + 1,
        get_apic().id()
    );

    sync_cores_barrier();
    sync_cores_barrier();

    #[cfg(not(feature = "testing"))]
    crate::common_main::main();

    #[cfg(feature = "testing")]
    crate::tester::ap_test_main();

    loop {
        hlt();
    }
}

pub fn sync_cores_barrier() {
    log::trace!(
        "syncing all aps and bsp (core with apic id {} arrived)",
        get_apic().id()
    );
    let barrier =
        SYNC_STARTUP_BARRIER.call_once(|| Barrier::new(ACPI.lock().ap_count as usize + 1));

    if barrier.wait().is_leader() {
        log::info!("All cores synced");
    }
}

static SYNC_STARTUP_BARRIER: Once<Barrier> = Once::new();

// must be called by each core
pub fn initialize_own_core_local_data(core_local_data: CoreLocalData) {
    let apic_id = get_apic().id();
    unsafe {
        CORE_LOCAL[apic_id as usize].get_or_init(|| core_local_data);
    }
}

// undefined behavior id initialize_own_core_local_data was not called by the calling core
#[inline]
pub fn get_cld() -> &'static mut CoreLocalData {
    let apic_id = get_apic().id();
    unsafe {
        CORE_LOCAL[apic_id as usize]
            .get_mut()
            .expect("core local data not initialized")
    }
}

#[inline]
pub fn cpu_index() -> u64 {
    get_cld().cpu_index
}

// to be used in exception interrupts (since the underlying APIC and ACPI may not be initialized yet)
#[inline]
pub fn try_get_cld() -> Option<&'static mut CoreLocalData> {
    let apic_id = crate::apic::try_get_apic()?.id();
    unsafe { CORE_LOCAL[apic_id as usize].get_mut() }
}

#[derive(Debug, Default)]
// Needs to stay small since it is in static memory for each potential core
pub struct CoreLocalData {
    pub running_application_data: Option<crate::loader::RunningApplicationCLD>,
    pub cpu_index: u64, //None for bsp
    pub apic_timer_ticks_per_second: Option<NonZeroU64>,
    pub apic_timer_interrupt_function: Option<fn()>,
    pub stuff: Option<Vec<Box<dyn Any>>>,
}

#[allow(clippy::declare_interior_mutable_const)]
const CORE_LOCAL_ENTRY_INIT: OnceCell<CoreLocalData> = OnceCell::new();
static mut CORE_LOCAL: [OnceCell<CoreLocalData>; MAX_CORES as usize] =
    [CORE_LOCAL_ENTRY_INIT; MAX_CORES as usize];
