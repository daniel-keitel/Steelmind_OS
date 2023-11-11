#![no_std]
#![no_main]

mod os_functions;

extern crate alloc;

static HELLO: &str = "Hello from Steelmind OS Program!\n";

entry_point!(main);

fn main() -> u64 {
    println!("Fmt {}", HELLO);
    666
}
