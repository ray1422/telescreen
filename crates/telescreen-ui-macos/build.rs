//! Compiles the SwiftUI views into a static library (spike 7). macOS only.
// A build script reports a failure with a panic.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("run a build tool");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("UTF-8 output")
        .trim()
        .to_owned()
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let mut sources: Vec<PathBuf> = std::fs::read_dir("swift")
        .expect("swift/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "swift"))
        .collect();
    sources.sort();
    println!("cargo::rerun-if-changed=swift");
    let optimize = if std::env::var("PROFILE").as_deref() == Ok("release") {
        "-O"
    } else {
        "-Onone"
    };
    run(Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            optimize,
        ])
        .args([
            "-module-name",
            "TelescreenUI",
            "-target",
            "arm64-apple-macosx27.0",
        ])
        .args(&sources)
        .arg("-o")
        .arg(out.join("libTelescreenUI.a")));
    let swiftc = PathBuf::from(run(Command::new("xcrun").args(["-f", "swiftc"])));
    let toolchain = swiftc
        .parent()
        .and_then(|p| p.parent())
        .expect("toolchain directory")
        .join("lib/swift/macosx");
    let sdk = run(Command::new("xcrun").args(["--show-sdk-path"]));
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=TelescreenUI");
    println!("cargo::rustc-link-search=native={}", toolchain.display());
    println!("cargo::rustc-link-search=native={sdk}/usr/lib/swift");
    // The rpath is a link argument of the final binary: telescreen-app/build.rs.
}
