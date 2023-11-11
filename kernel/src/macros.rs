#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::serial::_print_serial(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial_mark {
    () => ($crate::serial_print!("# {}:{}\n", file!(), line!()));
    ($($arg:tt)*) => ($crate::serial_print!("# {}:{}: {}\n", file!(), line!(), format_args!($($arg)*)));
}

#[macro_export]
macro_rules! print {
    ($stdout:expr; $($arg:tt)*) => ($stdout.print(format_args!($($arg)*)));
    ($($arg:tt)*) => ($crate::terminal_out::Stdout::acquire().print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    ($stdout:expr; $($arg:tt)*) => ($crate::print!($stdout; "{}\n", format_args!($($arg)*)));
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! barrier {
    ($count:expr) => {{
        static BARRIER: spin::once::Once<spin::Barrier> = spin::once::Once::new();
        BARRIER
            .call_once(|| spin::Barrier::new($count as usize))
            .wait();
    }};
}

#[macro_export]
macro_rules! ass {
    ($a:expr, $op:tt, $b:expr) => {
        let (a, b) = ($a, $b);
        if !(a $op b) {
            panic!("Assertion failed: {:?}({}) {} {:?}({})", a, stringify!($a), stringify!($op), b, stringify!($b));
        }
    };
    ($a:expr, $op:tt, $b:expr, $($arg:tt)*) => {
        let (a, b) = ($a, $b);
        if !(a $op b) {
            panic!("Assertion failed: {:?}({}) {} {:?}({})\n\t{}",a, stringify!($a), stringify!($op), b, stringify!($b), format_args!($($arg)*));
        }
    };
    ($a:expr) => {
        if !($a) {
            panic!("Assertion failed: {}", stringify!($a));
        }
    };
    ($a:expr, $($arg:tt)*) => {
        if !($a) {
            panic!("Assertion failed: {} \n\t{}", stringify!($a), format_args!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! same {
    ($a:expr, $b:expr) => {
        $crate::ass!($a, ==, $b);
    };
    ($a:expr, $b:expr, $($arg:tt)*) => {
        $crate::ass!($a, ==, $b, $($arg)*);
    };
}

#[macro_export]
macro_rules! different {
    ($a:expr, $b:expr) => {
        $crate::ass!($a, !=, $b);
    };
    ($a:expr, $b:expr, $($arg:tt)*) => {
        $crate::ass!($a, !=, $b, $($arg)*);
    };
}
