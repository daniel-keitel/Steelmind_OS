use core::alloc::GlobalAlloc;

use alloc::alloc::Layout;
use buddy_system_allocator::LockedHeapWithRescue;

use x86_64::{
    align_up,
    structures::paging::{Page, PageTableFlags},
    VirtAddr,
};

use crate::constants::v;

#[global_allocator]
pub static ALLOCATOR: KernelAllocatorWrapper = KernelAllocatorWrapper {};

lazy_static::lazy_static! {
    pub static ref INNER_KERNEL_ALLOC: LockedHeapWithRescue<38> = {
        log::info!("Initializing kernel heap");
        LockedHeapWithRescue::new(|heap, layout|{
            let old_size = heap.stats_total_bytes() as u64;
            let min_size_to_add = (align_up(old_size, layout.align() as u64) - old_size + layout.size() as u64).next_power_of_two();

            let new_total_size = (min_size_to_add + old_size).next_power_of_two().max(min_size_to_add * 2);
            let new_added_size = new_total_size - old_size;

            let start = v::KERNEL_HEAP_START + heap.stats_total_bytes() as u64;

            let page_range = {
                let mapping_start = VirtAddr::new(start);
                let mapping_end = mapping_start + new_added_size - 1u64;
                let mapping_start_page = Page::containing_address(mapping_start);
                let mapping_end_page = Page::containing_address(mapping_end);
                Page::range_inclusive(mapping_start_page, mapping_end_page)
            };

            let allocation_page_count = page_range.count();
            log::trace!(
                "Kernel heap grows by {} pages ({}MB)",
                 allocation_page_count, allocation_page_count * 4096 / 1024 / 1024);

            {
                let mut memory = crate::memory::MEMORY.lock();
                for page in page_range {
                    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
                    memory.map_ram_kernel(page, flags);
                }
                memory.log_memory_utilization(log::Level::Trace);
                log::trace!("Kernel heap stats: total_bytes: {}, alloc_actual: {}, alloc_user: {}",
                    heap.stats_total_bytes(),
                    heap.stats_alloc_actual(),
                    heap.stats_alloc_user(),
                );
            }

            let start_addr = page_range.start.start_address().as_u64() as usize;
            let end_addr = page_range.end.start_address().as_u64() as usize + 4096;
            unsafe {heap.add_to_heap(start_addr, end_addr);}
        })
    };
}

pub fn create_user_heap() -> UserAllocatorWrapper {
    log::debug!("Initializing user heap");
    let inner = LockedHeapWithRescue::<38>::new(|heap, layout| {
        let old_size = heap.stats_total_bytes() as u64;
        let min_size_to_add = (align_up(old_size, layout.align() as u64) - old_size
            + layout.size() as u64)
            .next_power_of_two();

        let new_total_size = (min_size_to_add + old_size)
            .next_power_of_two()
            .max(min_size_to_add * 2);
        let new_added_size = new_total_size - old_size;

        let start = v::USER_HEAP_START + heap.stats_total_bytes() as u64;

        let page_range = {
            let mapping_start = VirtAddr::new(start);
            let mapping_end = mapping_start + new_added_size - 1u64;
            let mapping_start_page = Page::containing_address(mapping_start);
            let mapping_end_page = Page::containing_address(mapping_end);
            Page::range_inclusive(mapping_start_page, mapping_end_page)
        };

        let allocation_page_count = page_range.count();
        log::trace!(
            "User heap grows by {} pages ({}MB)",
            allocation_page_count,
            allocation_page_count * 4096 / 1024 / 1024
        );

        {
            let mut memory = crate::memory::MEMORY.lock();
            for page in page_range {
                let flags =
                    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
                memory.map_ram_user(page, flags);
            }
            memory.log_memory_utilization(log::Level::Trace);
        }

        let start_addr = page_range.start.start_address().as_u64() as usize;
        let end_addr = page_range.end.start_address().as_u64() as usize + 4096;
        unsafe {
            heap.add_to_heap(start_addr, end_addr);
        }
    });
    UserAllocatorWrapper { inner }
}

pub struct KernelAllocatorWrapper {}

pub struct UserAllocatorWrapper {
    pub inner: LockedHeapWithRescue<38>,
}

impl KernelAllocatorWrapper {
    pub fn log_heap_stats(&self, level: log::Level) {
        let _ = self;
        let inner = INNER_KERNEL_ALLOC.lock();
        log::log!(
            level,
            "Kernel heap stats: total_bytes: {}, alloc_actual: {}, alloc_user: {}",
            inner.stats_total_bytes(),
            inner.stats_alloc_actual(),
            inner.stats_alloc_user(),
        );
    }
}

unsafe impl GlobalAlloc for KernelAllocatorWrapper {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        log::trace!("Kernel allocating {:?}", layout);
        INNER_KERNEL_ALLOC.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        log::trace!("Kernel deallocating {:?}", layout);
        INNER_KERNEL_ALLOC.dealloc(ptr, layout);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        log::trace!("Kernel allocating zeroed {:?}", layout);
        INNER_KERNEL_ALLOC.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        log::trace!("Kernel reallocating {:?} to {}", layout, new_size);
        INNER_KERNEL_ALLOC.realloc(ptr, layout, new_size)
    }
}
