use crate::test;

#[cfg(feature = "testing")]
use super::*;
#[cfg(feature = "testing")]
use crate::ass;
#[cfg(feature = "testing")]
use constants::v;

#[cfg(feature = "testing")]
use x86_64::{
    structures::paging::{Page, PageTableFlags},
    VirtAddr,
};

test!(simple_page_table_swap, {
    let mut mem = memory::MEMORY.lock();

    mem.log_page_table_info(log::Level::Info);

    let mut user_pt_a = mem.create_user_page_table();
    log::info!("Switch to user page table A");
    mem.switch_to_user_page_table(&mut user_pt_a);
    {
        let page = Page::containing_address(VirtAddr::new(v::USER_STACK_START));
        mem.map_ram_user(page, PageTableFlags::WRITABLE);

        let user_ref: &mut u64 = unsafe { &mut *page.start_address().as_mut_ptr() };

        log::info!("Garbage: {user_ref}");
        *user_ref = 42;
    }
    mem.log_page_table_info(log::Level::Info);

    let mut user_pt_b = mem.create_user_page_table();
    log::info!("Switch to user page table B");
    mem.switch_to_user_page_table(&mut user_pt_b);
    {
        let page = Page::containing_address(VirtAddr::new(v::USER_STACK_START));
        mem.map_ram_user(page, PageTableFlags::WRITABLE);

        let user_ref: &mut u64 = unsafe { &mut *page.start_address().as_mut_ptr() };

        log::info!("Garbage: {user_ref}");
        *user_ref = 666;
    }
    mem.log_page_table_info(log::Level::Info);

    log::info!("Switch to kernel page table");
    mem.switch_to_kernel_page_table();
    mem.log_page_table_info(log::Level::Info);

    log::info!("Switch to user page table A");
    mem.switch_to_user_page_table(&mut user_pt_a);
    {
        let user_ref: &mut u64 = unsafe { &mut *(v::USER_STACK_START as *mut u64) };

        ass!(*user_ref, ==, 42);
    }

    log::info!("Switch to user page table B");
    mem.switch_to_user_page_table(&mut user_pt_b);
    {
        let user_ref: &mut u64 = unsafe { &mut *(v::USER_STACK_START as *mut u64) };

        ass!(*user_ref, ==, 666);
    }
});

test!(simple_user_application, {
    let user_app = crate::ram_disk::get_file_slice(1);

    let mut resources_a = crate::loader::prepare_application(user_app);
    let mut resources_b = crate::loader::prepare_application(user_app);

    ass!(crate::loader::run(&mut resources_a), ==, 0);
    ass!(crate::loader::run(&mut resources_b), ==, 0);
    ass!(crate::loader::run(&mut resources_a), ==, 42);
});
