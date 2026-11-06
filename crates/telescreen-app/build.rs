//! The binary links the Swift static library of telescreen-ui-macos. A link
//! argument of a dependency does not reach this binary, so the Swift runtime
//! search path is set here (spike 7).

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo::rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
