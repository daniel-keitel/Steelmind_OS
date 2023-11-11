// everything is UNSAFE! unsafe functions are only extra unsafe

use core::ptr::addr_of;

use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::{
    align_down, align_up,
    registers::control::Cr3Flags,
    structures::paging::{
        page::PageRangeInclusive, FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page,
        PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

use bootloader_api::info::MemoryRegionKind;

use crate::{ass, constants::v, println};

#[inline]
pub fn active_level_4_table() -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;
    let (level_4_table_frame, _) = Cr3::read();
    page_table_from_frame(level_4_table_frame)
}

#[inline]
pub fn active_level_4_table_phys_addr() -> u64 {
    use x86_64::registers::control::Cr3;
    let (level_4_table_frame, _) = Cr3::read();
    level_4_table_frame.start_address().as_u64()
}

#[inline]
pub fn page_table_from_frame(frame: PhysFrame<Size4KiB>) -> &'static mut PageTable {
    let phys = frame.start_address();
    let virt = phys_to_virt(phys);
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

#[inline]
pub fn frame_from_page_table(page_table: &mut PageTable) -> PhysFrame<Size4KiB> {
    let virt = page_table as *mut PageTable as u64;
    let phys = PhysAddr::new(virt - physical_memory_offset().as_u64());
    PhysFrame::from_start_address(phys).unwrap()
}

#[inline]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    physical_memory_offset() + phys.as_u64()
}

#[inline]
pub fn physical_memory_offset() -> VirtAddr {
    unsafe {
        VirtAddr::new(
            *crate::get_boot_info()
                .physical_memory_offset
                .as_ref()
                .unwrap_unchecked(),
        )
    }
}

pub fn get_active_l4_page_table() -> OffsetPageTable<'static> {
    let l4_table = active_level_4_table();
    unsafe { OffsetPageTable::new(l4_table, physical_memory_offset()) }
}

pub fn create_new_l4_page_table(
    frame_alloc: &mut impl FrameAllocator<Size4KiB>,
) -> OffsetPageTable<'static> {
    log::trace!("Creating new L4 page table");
    let mut active_l4_table = get_active_l4_page_table();

    let new_l4_table = page_table_from_frame(frame_alloc.allocate_frame().expect("Out of memory"));
    new_l4_table.zero();

    for i in crate::constants::KERNEL_L4_PAGE_TABLE_RANGE {
        new_l4_table[i as usize] = active_l4_table.level_4_table()[i as usize].clone();
    }

    unsafe { OffsetPageTable::new(new_l4_table, physical_memory_offset()) }
}

pub fn switch_l4_page_table(l4_table: &mut OffsetPageTable) {
    use x86_64::registers::control::Cr3;
    let frame = frame_from_page_table(l4_table.level_4_table());
    unsafe {
        Cr3::write(frame, Cr3Flags::empty());
    }
}

pub struct BootInfoFrameAllocator {
    free_memory_head: Option<*mut FreeNode>,
    total_frames: u64,
    free_frames: u64,
}

struct FreeNode {
    next: Option<*mut FreeNode>,
    // Physical address of this node
    frame: PhysFrame,
}

fn phys_addr_to_node_ref(addr: &u64) -> *mut FreeNode {
    let phys_addr = PhysAddr::new(*addr);
    let virt_addr = phys_to_virt(phys_addr);
    virt_addr.as_mut_ptr::<FreeNode>()
}

pub fn change_pat_so_write_through_plus_cache_disabled_is_write_combining() {
    unsafe { x86_64::registers::model_specific::Msr::new(0x277).write(0x0007_0406_0107_0406) };
    log::info!("Changed PAT: write_through + cache_disabled -> Write combining");
    // 0x0007040600070406  default
    // 0x0007040601070406  modified
}

pub fn set_frame_buffer_cache_to_write_combining() {
    let frame_buffer = crate::get_boot_info()
        .framebuffer
        .as_ref()
        .unwrap()
        .buffer();

    let page_range: PageRangeInclusive = {
        let region_start = VirtAddr::new(addr_of!(frame_buffer[0]) as u64);
        let region_end = region_start + frame_buffer.len() as u64 - 1u64;
        let region_start_page = Page::containing_address(region_start);
        let region_end_page = Page::containing_address(region_end);
        Page::range_inclusive(region_start_page, region_end_page)
    };

    for page in page_range {
        unsafe {
            get_active_l4_page_table()
                .update_flags(
                    page,
                    PageTableFlags::PRESENT
                        | PageTableFlags::WRITABLE
                        | PageTableFlags::NO_EXECUTE
                        | PageTableFlags::WRITE_THROUGH
                        | PageTableFlags::NO_CACHE,
                )
                .unwrap()
                .flush();
        }
    }
}

impl BootInfoFrameAllocator {
    fn initialize_free_memory() -> (Option<*mut FreeNode>, u64) {
        log::info!("Initializing physical memory allocator");
        // bootloader bug mitigation
        let l4 = get_active_l4_page_table();
        let boot_info = crate::get_boot_info();
        let base_addr = x86_64::align_down(*boot_info.ramdisk_addr.as_ref().unwrap(), 4096);
        let mut first_phys_addr = None;
        let mut last_phys_addr = None;
        for i in 0..align_up(boot_info.ramdisk_len, 4096) / 4096 {
            let page = x86_64::structures::paging::Page::<x86_64::structures::paging::Size4KiB>::from_start_address(x86_64::VirtAddr::new(base_addr + i * 4096)).unwrap();
            let phys_addr = l4.translate_page(page).unwrap().start_address().as_u64();
            if let Some(last_phys_addr) = last_phys_addr {
                ass!(phys_addr, ==, last_phys_addr + 4096); // Mitigation assumes the physical memory region to be contiguous
            } else {
                first_phys_addr = Some(phys_addr);
            }
            last_phys_addr = Some(phys_addr);
        }

        let already_mapped_range = first_phys_addr.unwrap()..=last_phys_addr.unwrap();

        log::debug!(
            "Bootloader ram disk corruption mitigation: physical range of ramdisk:{:x} - {:x}",
            first_phys_addr.unwrap(),
            last_phys_addr.unwrap()
        );

        let mut removed_page_count = 0;

        let mut raw_usable_frame_addresses = crate::get_boot_info()
            .memory_regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable && r.start > 0)
            .map(|r| {
                log::trace!("Memory region: {r:?}");
                align_up(r.start, 4096)..align_down(r.end, 4096)
            })
            .flat_map(|r| r.step_by(4096))
            .filter(|r| {
                if already_mapped_range.contains(r) {
                    removed_page_count += 1;
                    false
                } else {
                    true
                }
            })
            .peekable();

        let free_memory_head = raw_usable_frame_addresses.peek().map(phys_addr_to_node_ref);

        let mut frame_count = 0;

        while let Some(phys_addr) = raw_usable_frame_addresses.next() {
            let node_ref = phys_addr_to_node_ref(&phys_addr);
            let node_next_ref = raw_usable_frame_addresses.peek().map(phys_addr_to_node_ref);

            unsafe {
                *node_ref = FreeNode {
                    next: node_next_ref,
                    frame: PhysFrame::from_start_address(PhysAddr::new(phys_addr)).unwrap(),
                };
            }
            frame_count += 1;
        }

        log::debug!("Bootloader ram disk corruption mitigation: removed {removed_page_count} pages of ramdisk from free list");

        (free_memory_head, frame_count)
    }

    pub fn new() -> Self {
        println!("Initializing memory");

        let (free_memory_head, frame_count) = Self::initialize_free_memory();

        println!("{}MB available", frame_count * 4096 / 1024 / 1024);

        Self {
            free_memory_head,
            total_frames: frame_count,
            free_frames: frame_count,
        }
    }
}

unsafe impl Send for BootInfoFrameAllocator {}
unsafe impl Sync for BootInfoFrameAllocator {}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.free_memory_head.map(|node| {
            self.free_frames -= 1;
            self.free_memory_head = unsafe { (*node).next };
            unsafe { (*node).frame }
        })
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        let node = phys_addr_to_node_ref(&frame.start_address().as_u64());
        unsafe {
            *node = FreeNode {
                next: self.free_memory_head,
                frame,
            };
        }
        self.free_memory_head = Some(node);
        self.free_frames += 1;
    }
}

pub struct Memory {
    kernel_page_table: OffsetPageTable<'static>,
    frame_allocator: BootInfoFrameAllocator,
}

impl Memory {
    pub fn new() -> Self {
        log::info!("Initializing main memory management structure");
        let kernel_page_table = get_active_l4_page_table();
        let frame_allocator = BootInfoFrameAllocator::new();
        Self {
            kernel_page_table,
            frame_allocator,
        }
    }

    pub const fn get_memory_utilization(&self) -> (u64, u64) {
        (
            (self.frame_allocator.total_frames - self.frame_allocator.free_frames),
            self.frame_allocator.total_frames,
        )
    }

    pub fn log_memory_utilization(&self, level: log::Level) {
        let util = self.get_memory_utilization();
        log::log!(
            level,
            "Memory utilization {}/{} pages; {}MB free of {}MB",
            util.0,
            util.1,
            (util.1 - util.0) * 4096 / 1024 / 1024,
            util.1 * 4096 / 1024 / 1024,
        );
    }

    pub fn create_user_page_table(&mut self) -> OffsetPageTable<'static> {
        create_new_l4_page_table(&mut self.frame_allocator)
    }

    pub fn switch_to_user_page_table(&mut self, user_page_table: &mut OffsetPageTable<'static>) {
        let _ = self;
        switch_l4_page_table(user_page_table);
    }

    pub fn switch_to_kernel_page_table(&mut self) {
        switch_l4_page_table(&mut self.kernel_page_table);
    }

    pub fn map_ram_kernel(&mut self, page: Page, flags: PageTableFlags) -> PhysFrame {
        ass!((v::KERNEL_START..v::KERNEL_END).contains(&page.start_address().as_u64()));
        let frame = self
            .frame_allocator
            .allocate_frame()
            .or_else(|| {
                log::error!(
                    "Out of memory: while allocating {page:?} with flags({flags:?}) for kernel"
                );
                self.log_memory_utilization(log::Level::Error);
                panic!("Out of memory");
            })
            .unwrap();
        unsafe { self.map_frame(page, flags | PageTableFlags::PRESENT, frame) };
        frame
    }

    pub fn map_ram_user(&mut self, page: Page, flags: PageTableFlags) -> PhysFrame {
        ass!((v::USER_START..v::USER_END).contains(&page.start_address().as_u64()));
        let frame = self
            .frame_allocator
            .allocate_frame()
            .or_else(|| {
                log::error!(
                    "Out of memory: while allocating {page:?} with flags({flags:?}) for user"
                );
                self.log_memory_utilization(log::Level::Error);
                panic!("Out of memory");
            })
            .unwrap();
        unsafe {
            self.map_frame(
                page,
                flags | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::PRESENT,
                frame,
            );
        };
        frame
    }

    pub unsafe fn map_frame(&mut self, page: Page, flags: PageTableFlags, frame: PhysFrame) {
        unsafe {
            get_active_l4_page_table()
                .map_to(page, frame, flags, &mut self.frame_allocator)
                .unwrap_or_else(|e| {
                    panic!(
                        "Unable to map page:{:?} to frame:{:?}:\n{:?}",
                        page, frame, e
                    )
                })
                .flush();
        };
    }

    pub unsafe fn change_flags(&mut self, page: Page, flags: PageTableFlags) {
        let _ = self;
        get_active_l4_page_table()
            .update_flags(page, flags)
            .unwrap_or_else(|e| panic!("Unable to change flags of page:{:?}:\n{:?}", page, e))
            .flush();
    }

    pub unsafe fn unmap_ram(&mut self, page: Page) {
        let frame = self.unmap(page);
        self.frame_allocator.deallocate_frame(frame);
    }

    pub unsafe fn unmap(&mut self, page: Page) -> PhysFrame {
        let _ = self;
        let (frame, flusher) = get_active_l4_page_table()
            .unmap(page)
            .unwrap_or_else(|e| panic!("Unable to unmap page:{:?}:\n{:?}", page, e));
        flusher.flush();
        frame
    }

    pub fn log_page_table_info(&mut self, level: log::Level) {
        let _ = self;
        for (i, l3) in get_active_l4_page_table()
            .level_4_table()
            .iter_mut()
            .enumerate()
        {
            if !l3.is_unused() {
                let flags = l3.flags();
                let phys = l3.frame().unwrap().start_address();
                let ptr = phys_to_virt(phys).as_mut_ptr();
                let l3_table: &PageTable = unsafe { &*ptr };

                let l3_entry_count = l3_table.iter().filter(|e| !e.is_unused()).count();

                log::log!(level, "L3[{i: <3}]: virt:{ptr:p} phys:{phys:p} entry_count:{l3_entry_count} flags:{flags:?}");
            }
        }
        log::log!(level, "");
    }
}

unsafe impl Send for Memory {}
unsafe impl Sync for Memory {}

lazy_static! {
    pub static ref MEMORY: Mutex<Memory> = Mutex::new(Memory::new());
}
