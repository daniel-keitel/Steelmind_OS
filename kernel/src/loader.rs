use core::arch::asm;
use core::fmt::Debug;
use core::ptr;

use elf::endian::LittleEndian;

use elf::ElfBytes;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::structures::paging::Page;
use x86_64::structures::paging::PageTableFlags;
use x86_64::structures::paging::Size4KiB;
use x86_64::VirtAddr;

use crate::allocator::UserAllocatorWrapper;
use crate::constants::v;
use crate::constants::USER_STACK_SIZE;
use crate::memory::MEMORY;
use crate::println;
use crate::smp::get_cld;

fn map_segment(virt_addr: u64, size: u64, flags: PageTableFlags, data: &[u8]) {
    let page_range = {
        let region_start = VirtAddr::new(virt_addr);
        let region_end = region_start + size - 1u64;
        let region_start_page = Page::containing_address(region_start);
        let region_end_page = Page::containing_address(region_end);
        Page::range_inclusive(region_start_page, region_end_page)
    };

    {
        let mut mem = MEMORY.lock();
        for page in page_range {
            // println!("mapping page: {:x}", page.start_address().as_u64());
            mem.map_ram_user(page, flags | PageTableFlags::WRITABLE);
        }
    }

    let mapped_segment =
        unsafe { &mut *ptr::slice_from_raw_parts_mut(virt_addr as *mut u8, size as usize) };

    mapped_segment[..].fill(0);
    mapped_segment[..data.len()].copy_from_slice(data);

    {
        let mut mem = MEMORY.lock();
        for page in page_range {
            unsafe {
                mem.change_flags(
                    page,
                    flags | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::PRESENT,
                );
            };
        }
    }
}

fn allocate_stack() {
    log::trace!("allocate application stack");
    let mut mem = MEMORY.lock();
    let base_virt_addr = v::USER_STACK_START + 4096; //add guard page
    let page_count = USER_STACK_SIZE / 4096 - 1;
    for i in 0..page_count {
        let page =
            Page::<Size4KiB>::from_start_address(VirtAddr::new(base_virt_addr + i * 4096)).unwrap();
        mem.map_ram_user(page, PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE);
    }
}

#[repr(align(64))]
pub struct ApplicationResources {
    l4_page_table: OffsetPageTable<'static>,
    entry_point_virt_addr: u64,
    heap: UserAllocatorWrapper,
}

impl Drop for ApplicationResources {
    fn drop(&mut self) {
        MEMORY
            .lock()
            .switch_to_user_page_table(&mut self.l4_page_table);

        log::warn!("TODO free application resources");

        MEMORY.lock().switch_to_kernel_page_table();
    }
}

pub fn prepare_application(file: &[u8]) -> ApplicationResources {
    log::debug!("Preparing application");
    let l4_page_table = {
        let mut mem = MEMORY.lock();
        let mut user_page_table = mem.create_user_page_table();
        mem.switch_to_user_page_table(&mut user_page_table);
        user_page_table
    };

    let entry_point = load(file);
    let heap = crate::allocator::create_user_heap();
    allocate_stack();

    MEMORY.lock().switch_to_kernel_page_table();

    ApplicationResources {
        l4_page_table,
        entry_point_virt_addr: entry_point,
        heap,
    }
}

fn free_application(resources: ApplicationResources) {
    drop(resources);
}

fn load(file: &[u8]) -> u64 {
    log::trace!("Loading application");
    let file = ElfBytes::<LittleEndian>::minimal_parse(file).unwrap();

    let entry_point = file.ehdr.e_entry;
    // println!("Entry point: {entry_point:X}");

    for segment in file.segments().unwrap() {
        if segment.p_type != elf::abi::PT_LOAD {
            continue;
        }
        let virt_addr = segment.p_vaddr;
        // if virt_addr < crate::constants::v::USER_START {
        //     continue;
        // }
        let size = segment.p_memsz;
        let raw_flags = segment.p_flags;
        let raw_flag_exec = raw_flags & elf::abi::PF_X != 0;
        let raw_flag_write = raw_flags & elf::abi::PF_W != 0;
        let raw_flag_read = raw_flags & elf::abi::PF_R != 0;
        crate::ass!(raw_flag_read || raw_flag_exec);

        let mut flags = PageTableFlags::empty();

        if !raw_flag_exec || raw_flag_write {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        if raw_flag_write {
            flags |= PageTableFlags::WRITABLE;
        }

        let data = file.segment_data(&segment).unwrap();

        log::trace!(
            "Loading application segment [{virt_addr:X}] size:{size:X} flags:{flags:?} data_len:{}",
            data.len()
        );

        map_segment(virt_addr, size, flags, data);
    }

    // Relics of a terrible idea with to much UB and double faults
    // let common = file.find_common_data().expect("shdrs should parse");
    // let symtab = common.symtab.unwrap();
    // let strtab = common.symtab_strs.unwrap();

    // for sym in symtab {
    //     let name = strtab.get(sym.st_name as usize).unwrap_or("no name");
    //     let addr = sym.st_value;
    //     let fp = match name {
    //         "__print" => user_functions::print as *const (),
    //         "__abort" => user_functions::abort as *const (),
    //         "__alloc" => user_functions::alloc as *const (),
    //         "__dealloc" => user_functions::dealloc as *const (),
    //         _ => ptr::null(),
    //     };
    //     if !fp.is_null() {
    //         unsafe { *(addr as *mut u64) = fp as u64 };
    //     }
    //     // println!("name:{name} addr:{addr:x} fp:{fp:p}");
    // }
    entry_point
}

mod user_functions {
    use core::{alloc::GlobalAlloc, slice};

    use crate::smp::get_cld;

    pub static FUNCTION_POINTERS: FunctionPointers = FunctionPointers {
        print,
        abort,
        alloc,
        dealloc,
    };

    #[repr(C)]
    pub struct FunctionPointers {
        print: extern "C" fn(*const u8, u64),
        abort: extern "C" fn(u64) -> !,
        alloc: extern "C" fn(u64, u64) -> *mut u8,
        dealloc: extern "C" fn(*mut u8, u64, u64),
    }

    pub extern "C" fn print(string: *const u8, len: u64) {
        let slice = unsafe { slice::from_raw_parts(string, len as usize) };
        crate::print!("{}", core::str::from_utf8(slice).unwrap());
    }
    pub extern "C" fn abort(exit_code: u64) -> ! {
        unsafe {
            super::abort(exit_code);
        }
    }
    pub extern "C" fn alloc(size: u64, alignment: u64) -> *mut u8 {
        unsafe {
            let layout =
                core::alloc::Layout::from_size_align_unchecked(size as usize, alignment as usize);
            let app_res = &mut *get_cld()
                .running_application_data
                .as_mut()
                .unwrap()
                .application_resources;
            app_res.heap.inner.alloc(layout)
        }
    }
    pub extern "C" fn dealloc(ptr: *mut u8, size: u64, alignment: u64) {
        unsafe {
            let layout =
                core::alloc::Layout::from_size_align_unchecked(size as usize, alignment as usize);
            let app_res = &mut *get_cld()
                .running_application_data
                .as_mut()
                .unwrap()
                .application_resources;
            app_res.heap.inner.dealloc(ptr, layout);
        }
    }
}

pub fn run(resources: &mut ApplicationResources) -> u64 {
    log::debug!("Running application");
    MEMORY
        .lock()
        .switch_to_user_page_table(&mut resources.l4_page_table);

    let ret = switch_stack_and_execute(resources);
    get_cld().running_application_data = None;

    MEMORY.lock().switch_to_kernel_page_table();
    ret
}

#[naked]
unsafe extern "C" fn save_instruction_pointer() {
    asm!(
        // In: rdi (abort_pointer_pointer), rsi (entry_point_pointer), rdx(saved_stack_pointer_pointer), rcx(function_pointers_pointer)
        // Out: rax (exit_code), rdx (saved_stack_pointer_pointer)
        "mov rax, [rsp]", // store return address in rax (used for abort)
        "mov [rdi], rax", // store return address in abort_pointer_pointer
        "mov r15, rdx",   // store saved_rsp_ptr in a callee save register (r15)
        "mov rdi, rcx",   // set first argument for call to entry point to function_pointers_pointer
        "call rsi",       // call entry point (sets rax to the exit code)
        "mov rdx, r15",   // set rdx to saved_rsp_ptr
        "ret",            // return
        options(noreturn),
    )
}

#[inline(never)]
extern "C" fn switch_stack_and_execute(resources: &mut ApplicationResources) -> u64 {
    unsafe {
        let new_rsp: u64 = v::USER_STACK_START + USER_STACK_SIZE - 1024;

        println!("new rsp: {:x}", new_rsp);

        let entry_point_pointer = resources.entry_point_virt_addr;

        let resources_ptr = core::ptr::addr_of_mut!(*resources);
        let cld = get_cld();
        cld.running_application_data = Some(RunningApplicationCLD {
            application_resources: resources_ptr,
            abort_addr: 0,
            saved_stack_pointer: 0,
        });
        let app_cld = cld.running_application_data.as_mut().unwrap();

        let saved_stack_pointer_pointer = core::ptr::addr_of_mut!(app_cld.saved_stack_pointer);
        let abort_pointer_pointer = core::ptr::addr_of_mut!(app_cld.abort_addr);

        let function_pointers_pointer: *const user_functions::FunctionPointers =
            &user_functions::FUNCTION_POINTERS;
        let mut ret: u64;

        // type EntryPoint = unsafe extern "C" fn(*const user_functions::FunctionPointers) -> u64;
        // let entry_point = core::mem::transmute::<_, EntryPoint>(entry_point_pointer as *const ());
        // println!("entry point: {:x}", entry_point_pointer);
        // ret = entry_point(function_pointers_pointer);
        // println!("done {ret}");
        // drop(function_pointers_storage);
        // return ret;

        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

        asm!(
            "push rbx",         // store everything on the stack which needs to be restored for after this asm block
            "push rbp",
            "push r14",
            "push r15",

            "mov [r8], rsp",    // save stack pointer
            "mov rsp, r9",      // set new stack pointer
            "mov rbp, rsp",
            "mov rdi, r10",     // first  arg for save_instruction_pointer
            "mov rsi, r11",     // second arg for save_instruction_pointer
            "mov rdx, r8",      // third  arg for save_instruction_pointer
            "mov rcx, r12",     // fourth arg for save_instruction_pointer

            "call {sip}",       // pushes return address to stack
                                // since we jump to this point directly from abort all registers (including stack are unusable) (except the return registers)

            "mov r13, rax",     // rax contains the exit code of the application (from return value or abort)
            "mov rsp, [rdx]",   // rdx contains the saved stack pointer (how it is retried divers between return and abort)


            "pop r15",          // restore everything we saved at the beginning
            "pop r14",
            "pop rbp",
            "pop rbx",

            // "2: jmp 2b",

            in("r8") saved_stack_pointer_pointer,
            in("r9") new_rsp,
            in("r10") abort_pointer_pointer,
            in("r11") entry_point_pointer,
            in("r12") function_pointers_pointer,
            out("r13") ret,
            sip = sym save_instruction_pointer
        );
        ret
    }
}

// only to be called if running_application_abort_instruction_pointer is set in the core local data
// to be only be called from within the call chain of switch_stack_and_execute
// this function will jump directly into the switch_stack_and_execute (currently running) and return from there with the return value of exit_code
#[inline(never)]
unsafe extern "C" fn abort(exit_code: u64) -> ! {
    // panic!("skip");
    let app_cld = get_cld()
    .running_application_data.as_mut()
    .expect("The abort instruction pointer is not set on the core on which abort was called, \n
             this should not happen unless abort was not called from within the call chain of switch_stack_and_execute");

    let saved_stack_pointer_pointer = core::ptr::addr_of_mut!(app_cld.saved_stack_pointer); // extra pointer level unnecessary, but it makes the assembly more clear
    let abort_pointer_pointer = core::ptr::addr_of_mut!(app_cld.abort_addr);

    asm!(
        // Out: rax (exit_code), rdx (saved_stack_pointer_pointer)
        "jmp [r8]",

        in("rax") exit_code,
        in("rdx") saved_stack_pointer_pointer,
        in("r8") abort_pointer_pointer,
    );
    panic!("should not be reached");
}

#[derive(Debug, Clone)]
pub struct RunningApplicationCLD {
    application_resources: *mut ApplicationResources,
    abort_addr: u64,
    saved_stack_pointer: u64,
}
