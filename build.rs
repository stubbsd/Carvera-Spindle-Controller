use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();

    // Only apply embedded-specific settings for ARM targets
    if target.starts_with("thumbv") {
        // Copy memory.x to output directory
        let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
        File::create(out.join("memory.x"))
            .unwrap()
            .write_all(include_bytes!("memory.x"))
            .unwrap();

        // Add output directory to linker search path
        println!("cargo:rustc-link-search={}", out.display());

        // Only rebuild if memory.x changes
        println!("cargo:rerun-if-changed=memory.x");

        // Linker arguments for embedded targets only
        println!("cargo:rustc-link-arg-bins=--nmagic");
        println!("cargo:rustc-link-arg-bins=-Tlink.x");
        println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
    }
}
