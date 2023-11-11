use core::{
    num::NonZeroU64,
    ptr::{self},
};

use x86_64::instructions::interrupts;

use bitfield::bitfield;

use crate::{ass, smp::get_cld};

pub struct Apic {
    local_apic_ptr: *mut u8,
}

impl Apic {
    const fn new(local_apic_ptr: *mut u8) -> Self {
        Self { local_apic_ptr }
    }

    fn init(&mut self) {
        log::debug!("initializing APIC");
        self.write(Offset::TaskPriority, 0); // set task priority to 0 (accept all interrupts)
                                             // self.write(Offset::SpuriousInterruptVector, 0xFF); // set spurious interrupt vector to 0xFF
                                             // self.write(Offset::SpuriousInterruptVector, 0x100); // enable apic
        self.write(Offset::SpuriousInterruptVector, 0x11FF); // disable focus processor checking
        self.init_timer();
    }

    pub fn signal_end_of_interrupt(&mut self) {
        self.write(Offset::EndOfInterrupt, 0);
    }

    pub fn id(&mut self) -> u8 {
        (self.read(Offset::Id) >> 24) as u8
    }

    pub fn write_interrupt_command(&mut self, cmd: ipi::InterruptCommand) {
        self.write(Offset::InterruptCommandHigh, cmd.upper() as u32);
        self.write(Offset::InterruptCommandLow, cmd.lower() as u32); //Order is important writing to low triggers command
    }

    pub fn read_interrupt_command(&mut self) -> ipi::InterruptCommand {
        let high = self.read(Offset::InterruptCommandHigh);
        let low = self.read(Offset::InterruptCommandLow);
        let mut cmd = ipi::InterruptCommand(0);
        cmd.set_upper(high as u64);
        cmd.set_lower(low as u64);
        cmd
    }

    pub fn read(&mut self, offset: Offset) -> u32 {
        unsafe { ptr::read_volatile(self.offset(offset)) }
    }

    pub fn write(&mut self, offset: Offset, value: u32) {
        unsafe { ptr::write_volatile(self.offset(offset), value) }
    }

    unsafe fn offset<T>(&mut self, offset: Offset) -> *mut T {
        self.local_apic_ptr
            .wrapping_add(offset as usize)
            .cast::<T>()
    }

    pub fn init_timer(&mut self) {
        let _lock = AP_TIMER_INIT_LOCK.lock();

        self.write(Offset::TimerDivideConfiguration, 0x3); // divider 16
        self.write(Offset::TimerInitialCount, 0xFFFF_FFFF);

        crate::pit::delay(50_000).unwrap(); // ~max pit delay

        self.write(Offset::TimerLocalVectorTableEntry, 1 << 16); // stop timer

        let ticks_in_50ms = 0xFFFF_FFFF - self.read(Offset::TimerCurrentCount);
        let ticks_per_second = ticks_in_50ms as u64 * 20;
        get_cld().apic_timer_ticks_per_second =
            Some(NonZeroU64::new(ticks_per_second).expect("apic timer initialization failed"));
    }

    pub fn start_timer(
        &mut self,
        interval_us: u32,
        periodic: bool,
        interrupt: fn(),
    ) -> Result<(), ()> {
        get_cld().apic_timer_interrupt_function = Some(interrupt);
        let ticks_per_second = get_cld().apic_timer_ticks_per_second.unwrap().get();
        let ticks_in_interval: u64 = (ticks_per_second * interval_us as u64) / 1_000_000;
        let ticks_in_interval: u32 = ticks_in_interval.try_into().map_err(|_| ())?;
        if periodic {
            ass!(ticks_in_interval > 0);
        }

        self.write(Offset::TimerLocalVectorTableEntry, 1 << 16);
        // // Start timer as periodic on IRQ 0, divider 16, with the number of ticks we counted
        self.write(
            Offset::TimerLocalVectorTableEntry,
            32 | if periodic { 0x20000 } else { 0x40000 },
        );
        self.write(Offset::TimerDivideConfiguration, 0x3);
        self.write(Offset::TimerInitialCount, ticks_in_interval);
        Ok(())
    }

    pub fn stop_timer(&mut self) {
        self.write(Offset::TimerLocalVectorTableEntry, 1 << 16);
    }
}

pub mod ipi {
    use super::bitfield;

    pub fn create_send_init_cmd() -> InterruptCommand {
        let mut ic = InterruptCommand(0);
        ic.set_interupt_vector(0);
        ic.set_delivery_mode(5);
        ic.set_destination_mode_logical(false);
        ic.set_de_assert(false);
        ic.set_not_de_assert(true);
        ic.set_destination_type(3);
        // ic.set_destination_type(0);
        // ic.set_apic_id(apic_id as u64);
        ic
    }

    pub fn create_startup_cmd(vector: u8) -> InterruptCommand {
        let mut ic = InterruptCommand(0);
        ic.set_interupt_vector(vector as u64);
        ic.set_delivery_mode(6);
        ic.set_destination_mode_logical(false);
        ic.set_de_assert(false);
        ic.set_not_de_assert(true);
        ic.set_destination_type(3);
        // ic.set_destination_type(0);
        // ic.set_apic_id(apic_id as u64);
        ic
    }

    bitfield! {
        #[derive(Clone, Copy)]
        pub struct InterruptCommand(u64);
        impl Debug;
        pub interupt_vector, set_interupt_vector: 7, 0;
        pub delivery_mode, set_delivery_mode: 10, 8;
        pub destination_mode_logical, set_destination_mode_logical: 11;
        pub delivery_status, _: 12;
        pub not_de_assert, set_not_de_assert: 14;
        pub de_assert, set_de_assert: 15;
        pub destination_type, set_destination_type: 19, 18;
        pub apic_id, set_apic_id: (32+27), (32+24);
        pub lower, set_lower : 31, 0;
        pub upper, set_upper : 63, 32;
    }
}

#[repr(usize)]
pub enum Offset {
    Id = 0x20,
    Version = 0x30,
    TaskPriority = 0x80,
    ArbitrationPriority = 0x90,
    ProcessorPriority = 0xa0,
    EndOfInterrupt = 0xb0,
    RemoteRead = 0xc0,
    LocalDestination = 0xd0,
    DestinationFormat = 0xe0,
    SpuriousInterruptVector = 0xf0,
    InService = 0x100,
    TriggerMode = 0x180,
    InterruptRequest = 0x200,
    ErrorStatus = 0x280,
    InterruptCommandLow = 0x300,
    InterruptCommandHigh = 0x310,
    TimerLocalVectorTableEntry = 0x320,
    ThermalLocalVectorTableEntry = 0x330,
    PerformanceCounterLocalVectorTableEntry = 0x340,
    LocalInterrupt0VectorTableEntry = 0x350,
    LocalInterrupt1VectorTableEntry = 0x360,
    ErrorVectorTableEntry = 0x370,
    TimerInitialCount = 0x380,
    TimerCurrentCount = 0x390,
    TimerDivideConfiguration = 0x3e0,
    ExtendedApicFeature = 0x400,
    ExtendedApicControl = 0x410,
    SpecificEndOfInterrupt = 0x420,
    InterruptEnable = 0x480,
    ExtendedInterruptLocalVectorTable = 0x500,
}

static LOCAL_APIC_PTR: spin::Once<u64> = spin::Once::new();

#[inline]
pub fn get_apic() -> Apic {
    let local_apic_ptr = (*LOCAL_APIC_PTR.get().expect("APIC not initialized")) as *mut u8;
    Apic::new(local_apic_ptr)
}

// to be used in exception interrupts (since the underlying ACPI may not be initialized yet)
#[inline]
pub fn try_get_apic() -> Option<Apic> {
    let local_apic_ptr = LOCAL_APIC_PTR.get().map(|p| *p as *mut u8)?;
    Some(Apic::new(local_apic_ptr))
}

// needs to be called be called once (only bsp) prior to first initialization (requires heap)
pub fn create() {
    interrupts::without_interrupts(|| {
        LOCAL_APIC_PTR.call_once(|| {
            log::info!("Creating APIC");
            let acpi = crate::acpi::ACPI.lock();
            acpi.local_apic_ptr as u64
        });
    });
}

// needs to be called by every core exactly once to use apic (after gdt is initialized)
pub fn init() {
    interrupts::without_interrupts(|| {
        get_apic().init();
    });
}

static AP_TIMER_INIT_LOCK: spin::Mutex<()> = spin::Mutex::new(());
