use core::ops::Range;

pub const KERNEL_STACK_SIZE: u64 = 4096 * 1024;
pub const MAX_CORES: u64 = 256;
pub const USER_STACK_SIZE: u64 = 4096 * 4096; // includes guard page

pub const KERNEL_L4_PAGE_TABLE_RANGE: Range<u32> = 100..116;
#[rustfmt::skip]
pub mod v {
    use super::build_addr;
    // l4 page table start:0 end:16
    pub const KERNEL_START: u64 =          build_addr(100 ,0  ,0  ,0  ,0);


    pub const KERNEL_DYNAMIC_START: u64 =  build_addr(100 ,0  ,0  ,1  ,0);
    pub const KERNEL_DYNAMIC_END: u64 =    build_addr(108 ,0  ,0  ,0  ,0);
    pub const KERNEL_HEAP_START: u64 =     build_addr(108 ,0  ,0  ,0  ,0);

    pub const KERNEL_AP_STACKS: u64 =      build_addr(109 ,0  ,0  ,0  ,0);

    pub const KERNEL_END: u64 =            build_addr(116 ,0  ,0  ,0  ,0); // exclusive

    // keep space for bootloader 
    pub const USER_START: u64 =            build_addr(0   ,0  ,0  ,0  ,0);
    pub const USER_STACK_START: u64 =      build_addr(0   ,4  ,0  ,0  ,0); //(lowest address of guard page)
    pub const USER_HEAP_START: u64 =       build_addr(0   ,8  ,0  ,0  ,0);
    pub const USER_END: u64 =              build_addr(1   ,0  ,0  ,0  ,0); // exclusive
}

pub fn print_addr(label: &str, addr: u64) {
    let l1 = addr >> 12;
    let l2 = l1 >> 9;
    let l3 = l2 >> 9;
    let l4 = l3 >> 9;
    crate::println!(
        "{: <24}: \t {:x}  {: <3} {: <3} {: <3} {: <3} {: <4}",
        label,
        addr,
        l4 & (512 - 1),
        l3 & (512 - 1),
        l2 & (512 - 1),
        l1 & (512 - 1),
        addr & (4096 - 1)
    );
}

#[must_use]
pub const fn build_addr(l4: u32, l3: u32, l2: u32, l1: u32, offset: u32) -> u64 {
    assert!(l4 < 512);
    assert!(l3 < 512);
    assert!(l2 < 512);
    assert!(l1 < 512);
    assert!(offset < 4096);
    (l4 as u64) << 39 | (l3 as u64) << 30 | (l2 as u64) << 21 | (l1 as u64) << 12 | (offset as u64)
}
