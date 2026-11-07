use std::path::PathBuf;
use std::process::Command;

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo::rerun-if-changed=swift/View.swift");
    run(Command::new("xcrun").args(["swiftc", "-parse-as-library", "-emit-library", "-static", "-O"])
        .args(["-module-name", "TSView", "-target", "arm64-apple-macosx26.0", "swift/View.swift", "-o"])
        .arg(out.join("libTSView.a")));
    let swiftc = PathBuf::from(run(Command::new("xcrun").args(["-f", "swiftc"])));
    let toolchain_lib = swiftc.parent().unwrap().parent().unwrap().join("lib/swift/macosx");
    let sdk = run(Command::new("xcrun").args(["--show-sdk-path"]));
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=TSView");
    println!("cargo::rustc-link-search=native={}", toolchain_lib.display());
    println!("cargo::rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo::rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
