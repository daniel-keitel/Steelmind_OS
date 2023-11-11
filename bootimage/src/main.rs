use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use clap::{Parser, ValueEnum};
use pruefung::Hasher;
use tempfile::NamedTempFile;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Bootloader {
    Uefi,
    Bios,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum RedirectSerial {
    None,
    File,
    Stdout,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    Dev,
    OptDev,
    Release,
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long, default_value_t = 4)]
    smp: u8,

    #[arg(short, long, default_value_t = false)]
    build_only: bool,

    #[arg(short = 'l', long, default_value_t = Bootloader::Bios)]
    #[clap(value_enum)]
    bootloader: Bootloader,

    #[arg(short, long, default_value_t = false)]
    test: bool,

    #[arg(short, long, default_value_t = RedirectSerial::Stdout)]
    #[clap(value_enum)]
    redirect_serial: RedirectSerial,

    #[arg(short, long, default_value_t = Profile::OptDev)]
    #[clap(value_enum)]
    kernel_profile: Profile,

    #[arg(short, long, default_value_t = Profile::OptDev)]
    #[clap(value_enum)]
    user_app_profile: Profile,

    #[arg(short, long, default_value_t = false)]
    assemble_smp_trampoline: bool,
}

fn main() {
    let args = Args::parse();

    #[cfg(not(test))]
    let test_mode = args.test;

    #[cfg(test)]
    let test_mode = true;

    if args.assemble_smp_trampoline {
        let mut cmd = std::process::Command::new("nasm");
        cmd.current_dir("kernel/smp_trampoline");
        cmd.arg("-fbin");
        cmd.arg("ap.asm");
        cmd.arg("-o");
        cmd.arg("ap.bin");
        cmd.spawn().unwrap().wait().unwrap();
    }

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build");
    cmd.arg("--bin");
    cmd.arg("kernel");
    cmd.arg("--target");
    cmd.arg("x86_64-unknown-none");
    let kernel_profile = add_profile_args(&mut cmd, args.kernel_profile);
    if test_mode {
        cmd.args(["--features", "testing"]);
    }
    if !cmd.spawn().unwrap().wait().unwrap().success() {
        panic!("Failed to build kernel");
    }

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir("user_app");
    cmd.arg("build");
    let user_profile = add_profile_args(&mut cmd, args.user_app_profile);
    if !cmd.spawn().unwrap().wait().unwrap().success() {
        panic!("Failed to build kernel");
    }

    let kernel = std::path::PathBuf::from(format!(
        "{}/x86_64-unknown-none/{}/kernel",
        std::env::var("CARGO_TARGET_DIR").unwrap_or("target".into()),
        kernel_profile
    ));

    let _ = std::fs::create_dir("bootimage/out");

    let ram_disk_path: tempfile::TempPath = create_ram_disk(user_profile).into_temp_path();

    let uefi_path = "bootimage/out/uefi.img";
    bootloader::UefiBoot::new(&kernel)
        .set_ramdisk(Path::new(ram_disk_path.to_str().unwrap()))
        .create_disk_image(&std::path::PathBuf::from(&uefi_path))
        .unwrap();

    let bios_path = "bootimage/out/bios.img";
    bootloader::BiosBoot::new(&kernel)
        .set_ramdisk(Path::new(ram_disk_path.to_str().unwrap()))
        .create_disk_image(&std::path::PathBuf::from(&bios_path))
        .unwrap();

    ram_disk_path.close().unwrap();

    println!("Uefi: {uefi_path} Bios: {bios_path}");

    if args.build_only {
        return;
    }

    let log = PathBuf::from("kernel.log");
    let last_log = PathBuf::from("kernel_last.log");

    if log.exists() {
        if last_log.exists() {
            fs::remove_file(&last_log).unwrap();
        }
        fs::rename(&log, &last_log).unwrap();
    }

    let mut cmd = std::process::Command::new("qemu-system-x86_64");
    if args.bootloader == Bootloader::Bios {
        cmd.arg("-drive")
            .arg(format!("format=raw,file={bios_path}"));
    } else {
        cmd.arg("-bios")
            .arg(ovmf_prebuilt::ovmf_pure_efi())
            .arg("-drive")
            .arg(format!("format=raw,file={}", uefi_path));
    }
    cmd.args([
        "-device",
        "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-m",
        "8G",
    ]);
    cmd.arg("-smp");
    cmd.arg(format!("{}", args.smp));
    match args.redirect_serial {
        RedirectSerial::None => {}
        RedirectSerial::File => {
            cmd.arg("-serial");
            cmd.arg(format!("file:{}", log.to_str().unwrap()));
        }
        RedirectSerial::Stdout => {
            cmd.arg("-serial");
            cmd.arg("stdio");
        }
    }

    let mut child = cmd.spawn().unwrap();
    let exit_code = child.wait().unwrap();

    if test_mode {
        assert!(
            exit_code.code().unwrap() == 33,
            "Wrong qemu exit code {exit_code}"
        );
    }
    let _ = exit_code;
}

fn add_profile_args(cmd: &mut std::process::Command, profile: Profile) -> &'static str {
    match profile {
        Profile::Dev => "debug",
        Profile::OptDev => {
            cmd.args(["--profile", "opt-dev"]);
            "opt-dev"
        }
        Profile::Release => {
            cmd.args(["--profile", "release-lto"]);
            "release-lto"
        }
    }
}

fn create_ram_disk(profile_name: &str) -> NamedTempFile {
    let mut img = Image::new();
    img.add_user_app("main", profile_name)
        .add_user_app("test", profile_name)
        .add_file(&"bootimage/test.jpg".into());

    img.build()
}

struct Image {
    region_ends: Vec<u64>,
    buffer: Vec<u8>,
}

#[allow(dead_code)]
impl Image {
    pub fn new() -> Self {
        Self {
            region_ends: Vec::new(),
            buffer: Vec::new(),
        }
    }

    pub fn add_file(&mut self, file: &std::path::PathBuf) -> &mut Self {
        fs::File::open(file)
            .unwrap()
            .read_to_end(&mut self.buffer)
            .unwrap();
        self.region_ends.push(self.buffer.len() as u64);
        self
    }

    pub fn add_user_app(&mut self, name: &str, profile_name: &str) -> &mut Self {
        let user_app = std::path::PathBuf::from(format!(
            "{}/x86_64-unknown-steelmind_os/{}/{}",
            std::env::var("CARGO_TARGET_DIR").unwrap_or("user_app/target".into()),
            profile_name,
            name
        ));
        self.add_file(&user_app);
        self
    }

    pub fn add_string(&mut self, string: &str) -> &mut Self {
        self.buffer.extend_from_slice(string.as_bytes());
        self.region_ends.push(self.buffer.len() as u64);
        self
    }

    pub fn build(mut self) -> NamedTempFile {
        self.region_ends.insert(0, self.region_ends.len() as u64);
        let region_lengths_buf: Vec<u8> = self
            .region_ends
            .iter()
            .flat_map(|e| e.to_le_bytes())
            .collect();

        let mut checksum = pruefung::crc::crc32::Crc32::default();
        checksum.write(&region_lengths_buf);
        checksum.write(&self.buffer);
        let checksum = checksum.finish();

        let mut img_file = NamedTempFile::new().unwrap();
        img_file
            .write_all(
                &((region_lengths_buf.len() + self.buffer.len() + 8 + 8) as u64).to_le_bytes(),
            )
            .unwrap();
        img_file.write_all(&checksum.to_le_bytes()).unwrap();
        img_file.write_all(&region_lengths_buf).unwrap();
        img_file.write_all(&self.buffer).unwrap();
        img_file
    }
}

#[test]
fn test() {
    std::env::set_current_dir("..").unwrap();
    main();
}
