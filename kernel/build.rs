fn main() {
    // required for linkme
    println!("cargo:rustc-link-arg-bins=-z");
    println!("cargo:rustc-link-arg-bins=nostart-stop-gc");
}
