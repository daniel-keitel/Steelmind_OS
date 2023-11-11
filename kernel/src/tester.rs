#[cfg(feature = "testing")]
use crate::{ass, different, same};

#[cfg(feature = "testing")]
#[linkme::distributed_slice]
pub static TESTS: [fn(&mut Tester)];

#[cfg(feature = "testing")]
pub struct Tester {
    number_of_tests: u32,
    counter: u32,
}

#[cfg(feature = "testing")]
impl Tester {
    pub fn start_test(&mut self, args: core::fmt::Arguments) {
        self.counter += 1;

        log::info!(
            "\nStarting test ({}/{}): {args}",
            self.counter,
            self.number_of_tests
        );
    }
}

#[cfg(feature = "testing")]
#[macro_export]
macro_rules! test {
    ($name:ident, $block:block) => {
        #[linkme::distributed_slice($crate::tester::TESTS)]
        fn $name(__tester: &mut $crate::tester::Tester) {
            __tester.start_test(format_args!(
                "{} \t({}:{})",
                stringify!($name),
                file!(),
                line!()
            ));
            $block
        }
    };
}

#[cfg(not(feature = "testing"))]
#[macro_export]
macro_rules! test {
    ($name:ident, $block:block) => {};
}

#[cfg(feature = "testing")]
pub fn ap_test_main() {}

#[cfg(feature = "testing")]
pub fn bsp_test_main() {
    run_tests();
}

#[cfg(feature = "testing")]
pub fn run_tests() {
    let number_of_tests = TESTS.len() as u32;

    if number_of_tests == 0 {
        log::error!("No tests found!");
        return;
    }

    let mut tester = Tester {
        number_of_tests,
        counter: 0,
    };
    for test in TESTS.iter().rev() {
        test(&mut tester);
    }

    crate::println!();
    for i in (0..500).rev() {
        crate::print!("\rshutdown countdown: {i}");
    }

    log::info!("\nAll({number_of_tests}) tests passed!");

    exit_qemu(QemuExitCode::Success);
}

#[cfg(feature = "testing")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;

    crate::terminal_out::panic_print(|term| {
        term.foreground = crate::terminal_out::Color::new(0xFF, 0x00, 0x00);
        term.background = crate::terminal_out::Color::new(0x70, 0x70, 0x00);
        term.write_fmt(format_args!("\n{info}\n")).unwrap();
    });
    log::error!("\n\t{info}");

    exit_qemu(QemuExitCode::Failed);

    #[allow(clippy::empty_loop)]
    loop {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
#[allow(dead_code)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

#[allow(dead_code)]
pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

test!(assertion_test, {
    ass!(5, ==, 5);
    ass!(5, ==, 3+2);
    same!(5, 3 + 2);
    ass!(5, >, 3+1);
    ass!({5 - 4}, <, 3+1, "Fail {}", 42);
    ass!([0, 1, 2, 3].iter().any(|e| *e > 2));
    different!("a", "b");
});
