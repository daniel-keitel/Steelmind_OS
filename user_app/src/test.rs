#![no_std]
#![no_main]

mod os_functions;

use core::{panic, sync::atomic::AtomicU64};

extern crate alloc;

static HELLO: &str = "Hello from Steelmind OS Program!\n";

static mut COUNTER: AtomicU64 = AtomicU64::new(0);

entry_point!(main);

fn main() -> u64 {
    println!("Fmt {}", HELLO);

    unsafe {
        let ret = COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if ret == 0 {
            rec(10);
        } else {
            panic!("PANIC");
        }
        ret
    }
}

#[inline(never)]
fn rec(count: u64) {
    println!("Count: {}", count);
    if count == 0 {
        let mut prime_iter = (2..u64::MAX)
            .filter(|n| !(2..*n).any(|i| n % i == 0))
            .enumerate();
        let v: alloc::vec::Vec<_> = prime_iter.by_ref().take(10).collect();
        print_vec(v);
        return;
    }
    rec(count - 1);
}

#[inline(never)]
fn print_vec<T: core::fmt::Debug>(v: alloc::vec::Vec<T>) {
    for e in v {
        println!("{e:?}");
    }
}
