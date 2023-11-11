use pruefung::Hasher;

use crate::{ass, get_boot_info};

pub fn get_file_slice(index: usize) -> &'static [u8] {
    let bootinfo = get_boot_info();

    let ramdisk_ptr = (*bootinfo.ramdisk_addr.as_ref().unwrap()) as *mut u8;

    #[allow(clippy::cast_ptr_alignment)]
    let ramdisk_u64_slice = unsafe {
        core::slice::from_raw_parts(ramdisk_ptr as *const u64, bootinfo.ramdisk_len as usize / 8)
    };

    let ramdisk_u8_slice = unsafe {
        core::slice::from_raw_parts(ramdisk_ptr.cast_const(), bootinfo.ramdisk_len as usize)
    };

    let file_count = ramdisk_u64_slice[2];
    let file_end_slice = &ramdisk_u64_slice[3..];
    ass!(index, <, file_count as usize);

    let (raw_file_start, file_size) = if index == 0 {
        (0, file_end_slice[0])
    } else {
        (
            file_end_slice[index - 1],
            file_end_slice[index] - file_end_slice[index - 1],
        )
    };

    let file_start = raw_file_start + (file_count + 3) * 8;

    &ramdisk_u8_slice[file_start as usize..(file_start + file_size) as usize]
}

pub fn get_file_count() -> usize {
    let bootinfo = get_boot_info();

    let ramdisk_ptr = (*bootinfo.ramdisk_addr.as_ref().unwrap()) as *mut u8;

    #[allow(clippy::cast_ptr_alignment)]
    let ramdisk_u64_slice = unsafe {
        core::slice::from_raw_parts(ramdisk_ptr as *const u64, bootinfo.ramdisk_len as usize / 8)
    };

    let file_count = ramdisk_u64_slice[2];
    file_count as usize
}

pub fn assert_soundness() {
    log::debug!("Checking ram disk soundness");
    let bootinfo = get_boot_info();
    ass!(bootinfo.ramdisk_addr.as_ref().is_some());
    ass!(bootinfo.ramdisk_len, >=, 8+8+8+8+1);

    let ramdisk_ptr = (*bootinfo.ramdisk_addr.as_ref().unwrap()) as *mut u8;

    #[allow(clippy::cast_ptr_alignment)]
    let ramdisk_u64_slice = unsafe {
        core::slice::from_raw_parts(ramdisk_ptr as *const u64, bootinfo.ramdisk_len as usize / 8)
    };

    let ramdisk_u8_slice = unsafe {
        core::slice::from_raw_parts(ramdisk_ptr.cast_const(), bootinfo.ramdisk_len as usize)
    };

    let ram_disk_read_length = ramdisk_u64_slice[0];
    ass!(ram_disk_read_length, ==, bootinfo.ramdisk_len);

    let ram_disk_read_checksum = ramdisk_u64_slice[1];
    let mut checksum = pruefung::crc::crc32::Crc32::default();
    checksum.write(&ramdisk_u8_slice[8 + 8..]);
    let checksum = checksum.finish();
    ass!(ram_disk_read_checksum, ==, checksum);

    let file_count = ramdisk_u64_slice[2];

    ass!(file_count, <=, (ram_disk_read_length - 8 - 8 - 8) / 9);

    log::debug!("Ramdisk ok");
}
