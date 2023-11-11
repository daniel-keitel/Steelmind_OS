# Steelmind OS
A x86_64 multicore toy OS written in Rust with a terrible name (Referencing Awakened steelminds from Brandon Sanderson's Cosmere)

## Features
- Boots from Bios and Uefi using: https://github.com/rust-osdev/bootloader
- Multicore support
- Serial IO
- Simple buffered text output (with support for embedded images)
- Loading and running of static elf programs in separate address spaces with own heap and stack

## Install
Cross-platform installation: (should work on Linux Mac and Windows)
- install rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`) (any rustup install is ok)
- run `sh setup.sh` or manually install missing components
- install qemu and add it to PATH (not required to build bootable images)


## Build and run
run in qemu:
```cargo run``` 

help: 
```cargo run -- -h``` 

tests:
```cargo test``` 

build only (doesn't require qemu): 
```cargo run -- -b```

Bootable images can be found in `bootimage/out`

## Structure
- ./
    - workspace root for kernel and ./bootimage

- ./kernel
    - main kernel crate
    - ./kernel/smp_trampoline:
        - contains application processor initialization code 
        - is not assembled by default instead a binary is included in the repository
        - to assemble the file run with `-a` (```cargo run -- -a```)

- ./bootimage:
    - utility program used to initiate builds
    - creates disk images with bootloader and ram disk content
    - can start qemu
    - default-run for workspace
    - ./bootimage/out contains bootable images after builds

- ./user_app:
    - contains all programs which may be loaded by the kernel

## Contributing
This is just my toy OS, so it will probably will never be useful.
If you have questions about this OS (or found a major bug and want to let me know) feel free to open an Issue.
