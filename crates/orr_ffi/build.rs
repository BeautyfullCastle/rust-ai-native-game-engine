// Hands the target triple to the tests (they drive the `cc` crate to compile the C client).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=ORR_FFI_TARGET={}", std::env::var("TARGET").unwrap_or_default());
}
