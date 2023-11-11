use x86_64::instructions::port::Port;

bitfield::bitfield! {
    #[derive(Clone, Copy)]
    struct Mode(u8);
    impl Debug;
    _, set_bcd_format          : 0;
    _, set_operating_mode   : 3, 1;
    _, set_access_mode      : 5, 4;
    _, set_channel          : 7, 6;
}

#[repr(u8)]
enum AccessMode {
    LatchCountValue = 0,
    LowByteOnly = 1,
    HighByteOnly = 2,
    LowAndHighByte = 3,
}

#[repr(u8)]
enum OperatingMode {
    InterruptOnTerminalCount = 0,
    ProgrammableOneShot = 1,
    RateGenerator = 2,
    SquareWaveGenerator = 3,
    SoftwareTriggeredStrobe = 4,
    HardwareTriggeredStrobe = 5,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TimerResult {
    OutOfRange,
    NotActive,
}

impl Mode {
    fn new(access_mode: AccessMode, operating_mode: OperatingMode, bcd_format: bool) -> Self {
        let mut m = Self(0);
        m.set_bcd_format(bcd_format);
        m.set_access_mode(access_mode as u8);
        m.set_operating_mode(operating_mode as u8);
        m
    }
    fn write(self) {
        unsafe { Port::new(0x43).write(self.0) }
    }
}

bitfield::bitfield! {
    #[derive(Clone, Copy)]
    struct Control(u8);
    impl Debug;
    enable_timer_counter2, set_enable_timer_counter2 : 0;
    enable_speaker_data  , set_enable_speaker_data   : 1;
    enable_pci_serr      , set_enable_pci_serr       : 2;
    enable_nmi_iochk     , set_enable_nmi_iochk      : 3;
    refresh_cycle_toggle   , _ : 4;
    status_timer_counter2  , _ : 5;
    status_iochk_nmi_source, _ : 6;
    status_serr_nmi_source , _ : 7;
}

impl Control {
    fn read() -> Self {
        Self(unsafe { Port::new(0x61).read() })
    }

    fn write(self) {
        unsafe { Port::new(0x61).write(self.0) }
    }
}

fn set_data(channel: u8, value: u8) {
    unsafe { Port::new(0x40 + channel as u16).write(value) }
}

fn get_data(channel: u8) -> u8 {
    unsafe { Port::new(0x40 + channel as u16).read() }
}

const BASE_FREQUENCY: u64 = 1_193_182;

pub fn set(us: u16) -> Result<(), TimerResult> {
    let counter = BASE_FREQUENCY * us as u64 / 1_000_000;
    if counter > 0xffff {
        return Err(TimerResult::OutOfRange);
    }
    let mut c = Control::read();
    c.set_enable_speaker_data(false);
    c.set_enable_timer_counter2(true);
    c.write();

    let mut m = Mode::new(
        AccessMode::LowAndHighByte,
        OperatingMode::InterruptOnTerminalCount,
        false,
    );
    m.set_channel(2);
    m.write();

    set_data(2, (counter & 0xff) as u8);
    set_data(2, ((counter >> 8) & 0xff) as u8);

    Ok(())
}

pub fn get() -> u16 {
    let mut m = Mode(0);
    m.set_channel(2);
    m.write();

    get_data(2) as u16 | ((get_data(2) as u16) << 8)
}

pub fn is_active() -> bool {
    let c = Control::read();
    c.enable_timer_counter2() && !c.status_timer_counter2()
}

pub fn wait_for_timeout() -> Result<(), TimerResult> {
    loop {
        let c = Control::read();
        if !c.enable_timer_counter2() {
            return Err(TimerResult::NotActive);
        } else if c.status_timer_counter2() {
            return Ok(());
        }
        core::hint::spin_loop();
    }
}

pub fn delay(us: u16) -> Result<(), TimerResult> {
    set(us)?;
    wait_for_timeout()
}

pub fn disable() {
    let mut c = Control(0);
    c.set_enable_speaker_data(false);
    c.set_enable_timer_counter2(false);
    c.write();
}
