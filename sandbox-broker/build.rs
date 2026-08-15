use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/macos-conceal-launcher.c");
    println!("cargo:rerun-if-changed=native/macos-conceal-shim.c");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=PI_CONCEAL_LAUNCHER_PATH");
    println!("cargo:rerun-if-env-changed=PI_CONCEAL_SHIM_PATH");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let launcher = output.join("pi-sandbox-conceal-launcher");
    let shim = output.join("libpi-sandbox-conceal.dylib");
    compile(
        &["-std=c11", "-Os", "-Wall", "-Wextra", "-Werror", "-o"],
        &launcher,
        Path::new("native/macos-conceal-launcher.c"),
    );
    compile(
        &[
            "-std=c11",
            "-Os",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-dynamiclib",
            "-o",
        ],
        &shim,
        Path::new("native/macos-conceal-shim.c"),
    );

    println!(
        "cargo:rustc-env=PI_CONCEAL_LAUNCHER_BUILD_PATH={}",
        launcher.display()
    );
    println!(
        "cargo:rustc-env=PI_CONCEAL_SHIM_BUILD_PATH={}",
        shim.display()
    );
}

fn compile(flags: &[&str], output: &Path, source: &Path) {
    let compiler = env::var_os("CC").unwrap_or_else(|| "cc".into());
    let status = Command::new(compiler)
        .args(flags)
        .arg(output)
        .arg(source)
        .status()
        .unwrap_or_else(|error| {
            panic!("cannot start C compiler for {}: {error}", source.display())
        });
    assert!(
        status.success(),
        "C compiler failed for {}",
        source.display()
    );
}
