use core::ptr::NonNull;

use acpi::{AcpiTables, PhysicalMapping};
use spin::Mutex;

use crate::{get_boot_info, memory::physical_memory_offset};

#[derive(Clone)]
pub struct AcpiHandler {}

impl acpi::AcpiHandler for AcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let offset = physical_memory_offset().as_u64();
        let ptr = NonNull::new((physical_address + offset as usize) as *mut T).unwrap();
        PhysicalMapping::new(physical_address, ptr, size, size, self.clone())
    }

    fn unmap_physical_region<T>(_region: &acpi::PhysicalMapping<Self, T>) {}
}

pub struct Acpi {
    pub acpi_tables: AcpiTables<AcpiHandler>,
    pub local_apic_ptr: *mut (),
    pub ap_count: u64,
}

impl Acpi {
    pub fn new() -> Self {
        log::info!("Initializing ACPI");
        let handler = AcpiHandler {};
        let rsdp_addr = *get_boot_info().rsdp_addr.as_ref().unwrap();
        let acpi_tables = unsafe { AcpiTables::from_rsdp(handler, rsdp_addr as usize) }.unwrap();

        let platform_info = acpi_tables.platform_info().unwrap();

        let local_apic_address =
            if let acpi::InterruptModel::Apic(ref apic) = platform_info.interrupt_model {
                apic.local_apic_address
            } else {
                panic!("Apic not supported");
            };

        let ap_count = platform_info
            .processor_info
            .as_ref()
            .unwrap()
            .application_processors
            .iter()
            .filter(|e| e.state != acpi::platform::ProcessorState::Disabled && e.is_ap)
            .count() as u64;

        core::mem::drop(platform_info);

        let mut s = Self {
            acpi_tables,
            local_apic_ptr: (physical_memory_offset().as_u64() + local_apic_address) as *mut (),
            ap_count,
        };
        s.log_proccessor_info(log::Level::Trace);
        s
    }

    pub fn log_proccessor_info(&mut self, level: log::Level) {
        let platform_info = self.acpi_tables.platform_info().unwrap();

        for entry in &*platform_info.processor_info.unwrap().application_processors {
            log::log!(level, "Found processor: {:#?}", entry);
        }
    }
}

unsafe impl Send for Acpi {}
unsafe impl Sync for Acpi {}

lazy_static::lazy_static! {
    pub static ref ACPI: Mutex<Acpi> = Mutex::new(Acpi::new());
}
