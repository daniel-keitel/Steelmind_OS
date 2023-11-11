#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use core::{
    fmt::{self, Write},
    hint,
};

use alloc::string::String;
use spin::Mutex;
use x86_64::instructions::port::Port;

#[repr(u16)]
#[derive(Clone, Copy)]
pub enum ComPort {
    COM1 = 0x3f8,
    COM2 = 0x2f8,
    COM3 = 0x3e8,
    COM4 = 0x2e8,
}

#[repr(u16)]
#[derive(Clone, Copy)]
pub enum BaudRate {
    BAUD_300 = 384,
    BAUD_600 = 192,
    BAUD_1200 = 96,
    BAUD_2400 = 48,
    BAUD_4800 = 24,
    BAUD_9600 = 12,
    BAUD_19200 = 6,
    BAUD_38400 = 3,
    BAUD_57600 = 2,
    BAUD_115200 = 1,
}

#[repr(u8)]
#[derive(Clone, Copy)]
enum DataBits {
    DATA_5BIT = 0,
    DATA_6BIT = 1,
    DATA_7BIT = 2,
    DATA_8BIT = 3,
}

mod StopBits {
    pub(super) const STOP_1BIT: u8 = 0;
    pub(super) const STOP_1_5BIT: u8 = 4;
    pub(super) const STOP_2BIT: u8 = 4;
}

#[repr(u8)]
#[derive(Clone, Copy)]
enum Parity {
    PARITY_NONE = 0,
    PARITY_ODD = 8,
    PARITY_EVEN = 24,
    PARITY_MARK = 40,
    PARITY_SPACE = 56,
}

mod ri {
    // if Divisor Latch Access Bit [DLAB] = 0
    pub(super) const RECEIVE_BUFFER_REGISTER: u8 = 0;
    ///< read only
    pub(super) const TRANSMIT_BUFFER_REGISTER: u8 = 0;
    ///< write only
    pub(super) const INTERRUPT_ENABLE_REGISTER: u8 = 1;

    // if Divisor Latch Access Bit [DLAB] = 1
    pub(super) const DIVISOR_LOW_REGISTER: u8 = 0;
    pub(super) const DIVISOR_HIGH_REGISTER: u8 = 1;

    // (irrespective from DLAB)
    pub(super) const INTERRUPT_IDENT_REGISTER: u8 = 2;
    ///< read only
    pub(super) const FIFO_CONTROL_REGISTER: u8 = 2;
    ///< write only -- 16550 and newer (esp. not 8250a)
    pub(super) const LINE_CONTROL_REGISTER: u8 = 3;
    ///< highest-order bit is DLAB (see above)
    pub(super) const MODEM_CONTROL_REGISTER: u8 = 4;
    pub(super) const LINE_STATUS_REGISTER: u8 = 5;
    pub(super) const MODEM_STATUS_REGISTER: u8 = 6;
}

mod RegisterMask {
    // Interrupt Enable Register
    pub(super) const RECEIVED_DATA_AVAILABLE: u8 = 1 << 0;
    pub(super) const TRANSMITTER_HOLDING_REGISTER_EMPTY: u8 = 1 << 1;
    pub(super) const RECEIVER_LINE_STATUS: u8 = 1 << 2;
    pub(super) const MODEM_STATUS: u8 = 1 << 3;

    // Interrupt Ident Register
    pub(super) const INTERRUPT_PENDING: u8 = 1 << 0;
    ///< 0 means interrupt pending
    pub(super) const INTERRUPT_ID_0: u8 = 1 << 1;
    pub(super) const INTERRUPT_ID_1: u8 = 1 << 2;

    // FIFO Control Register
    pub(super) const ENABLE_FIFO: u8 = 1 << 0;
    ///< 0 means disabled ^= conforming to 8250a
    pub(super) const CLEAR_RECEIVE_FIFO: u8 = 1 << 1;
    pub(super) const CLEAR_TRANSMIT_FIFO: u8 = 1 << 2;
    pub(super) const DMA_MODE_SELECT: u8 = 1 << 3;
    pub(super) const TRIGGER_RECEIVE: u8 = 1 << 6;

    // Line Control Register
    //  bits per character:  5   6   7   8
    pub(super) const WORD_LENGTH_SELECT_0: u8 = 1 << 0; //  Setting Select0:     0   1   0   1
    pub(super) const WORD_LENGTH_SELECT_1: u8 = 1 << 1; //  Setting Select1:     0   0   1   1
    pub(super) const NUMBER_OF_STOP_BITS: u8 = 1 << 2; //  0 ≙ one stop bit, 1 ≙ 1.5/2 stop bits
    pub(super) const PARITY_ENABLE: u8 = 1 << 3;
    pub(super) const EVEN_PARITY_SELECT: u8 = 1 << 4;
    pub(super) const STICK_PARITY: u8 = 1 << 5;
    pub(super) const SET_BREAK: u8 = 1 << 6;
    pub(super) const DIVISOR_LATCH_ACCESS_BIT: u8 = 1 << 7; // DLAB

    // Modem Control Register
    pub(super) const DATA_TERMINAL_READY: u8 = 1 << 0;
    pub(super) const REQUEST_TO_SEND: u8 = 1 << 1;
    pub(super) const OUT_1: u8 = 1 << 2;
    pub(super) const OUT_2: u8 = 1 << 3; // must be set for interrupts!
    pub(super) const LOOP: u8 = 1 << 4;

    // Line Status Register
    pub(super) const DATA_READY: u8 = 1 << 0; // Set when there is a value in the receive buffer
    pub(super) const OVERRUN_ERROR: u8 = 1 << 1;
    pub(super) const PARITY_ERROR: u8 = 1 << 2;
    pub(super) const FRAMING_ERROR: u8 = 1 << 3;
    pub(super) const BREAK_INTERRUPT: u8 = 1 << 4;
    pub(super) const TRANSMITTER_HOLDING_REGISTER: u8 = 1 << 5;
    pub(super) const TRANSMITTER_EMPTY: u8 = 1 << 6; // Send buffer empty (ready to send)

    // Modem Status Register
    pub(super) const DELTA_CLEAR_TO_SEND: u8 = 1 << 0;
    pub(super) const DELTA_DATA_SET_READY: u8 = 1 << 1;
    pub(super) const TRAILING_EDGE_RING_INDICATOR: u8 = 1 << 2;
    pub(super) const DELTA_DATA_CARRIER_DETECT: u8 = 1 << 3;
    pub(super) const CLEAR_TO_SEND: u8 = 1 << 4;
    pub(super) const DATA_SET_READY: u8 = 1 << 5;
    pub(super) const RING_INDICATOR: u8 = 1 << 6;
    pub(super) const DATA_CARRIER_DETECT: u8 = 1 << 7;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerialError {
    Busy,
    OverrunError,
    ParityError,
    FramingError,
}

fn rr(com: ComPort, index: u8) -> u8 {
    unsafe { Port::new(com as u16 + index as u16).read() }
}

fn wr(com: ComPort, index: u8, value: u8) {
    unsafe { Port::new(com as u16 + index as u16).write(value) }
}

pub fn init(com: ComPort, baud: BaudRate) {
    wr(com, ri::LINE_CONTROL_REGISTER, 0b1000_0000); //enable DLAB
    wr(com, ri::DIVISOR_LOW_REGISTER, (baud as u16 & 0xff) as u8); //set baud rate
    wr(com, ri::DIVISOR_HIGH_REGISTER, (baud as u16 >> 8) as u8);
    wr(com, ri::LINE_CONTROL_REGISTER, 0b0000_0011); //8N1 + disable DLAB
    wr(com, ri::INTERRUPT_ENABLE_REGISTER, 0); //disable interrupts
    wr(com, ri::MODEM_CONTROL_REGISTER, 0); //disable loopback
}

pub struct ReadPort {
    com: ComPort,
}

pub struct WritePort {
    com: ComPort,
}

impl ReadPort {
    pub const fn new(com: ComPort) -> Self {
        Self { com }
    }

    pub fn try_read(&self) -> Result<u8, SerialError> {
        let status = rr(self.com, ri::LINE_STATUS_REGISTER);
        if status & RegisterMask::OVERRUN_ERROR != 0 {
            return Err(SerialError::OverrunError);
        }
        if status & RegisterMask::PARITY_ERROR != 0 {
            return Err(SerialError::ParityError);
        }
        if status & RegisterMask::FRAMING_ERROR != 0 {
            return Err(SerialError::FramingError);
        }
        if status & RegisterMask::DATA_READY != 0 {
            Ok(rr(self.com, ri::RECEIVE_BUFFER_REGISTER))
        } else {
            Err(SerialError::Busy)
        }
    }

    pub fn read(&self) -> Result<u8, SerialError> {
        loop {
            match self.try_read() {
                Ok(v) => return Ok(v),
                Err(SerialError::Busy) => {}
                Err(e) => return Err(e),
            }
            hint::spin_loop();
        }
    }

    pub fn read_line(&self) -> Result<String, SerialError> {
        let mut s = String::new();
        loop {
            let c = self.read()?;
            if c == b'\r' {
                return Ok(s);
            }
            if let Ok(c) = core::str::from_utf8(&[c]) {
                s += c;
            }
        }
    }
}

impl WritePort {
    pub const fn new(com: ComPort) -> Self {
        Self { com }
    }

    pub fn try_write(&self, value: u8) -> Result<(), SerialError> {
        let status = rr(self.com, ri::LINE_STATUS_REGISTER);
        if status & RegisterMask::OVERRUN_ERROR != 0 {
            return Err(SerialError::OverrunError);
        }
        if status & RegisterMask::PARITY_ERROR != 0 {
            return Err(SerialError::ParityError);
        }
        if status & RegisterMask::FRAMING_ERROR != 0 {
            return Err(SerialError::FramingError);
        }
        if status & RegisterMask::TRANSMITTER_HOLDING_REGISTER != 0 {
            wr(self.com, ri::TRANSMIT_BUFFER_REGISTER, value);
            Ok(())
        } else {
            Err(SerialError::Busy)
        }
    }

    pub fn write(&self, value: u8) {
        while self.try_write(value).is_err() {
            hint::spin_loop();
        }
    }

    pub fn flush(&self) {
        while rr(self.com, ri::LINE_STATUS_REGISTER) & RegisterMask::TRANSMITTER_EMPTY == 0 {
            hint::spin_loop();
        }
    }
}

impl fmt::Write for WritePort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if c.is_ascii() {
                self.write(c as u8);
            }
        }
        Ok(())
    }
}

lazy_static::lazy_static! {
    pub static ref SERIAL: (Mutex<ReadPort>, Mutex<WritePort>) = {
        init(ComPort::COM1, BaudRate::BAUD_115200);
        (Mutex::new(ReadPort::new(ComPort::COM1)), Mutex::new(WritePort::new(ComPort::COM1)))
    };
}

#[doc(hidden)]
pub fn _print_serial(args: fmt::Arguments) {
    let _ = SERIAL.1.lock().write_fmt(args);
}
