//! Dev tool for the clipboard hardware test (`scripts/hw-clipboard-test.sh`).
//! It simulates a password manager on Linux (CLIP-7, AC-34).
//!
//! Usage: `clip_copy [--concealed] <text>`
//!
//! The tool puts the text on the Wayland clipboard through data-control.
//! With `--concealed`, it also offers `x-kde-passwordManagerHint` with the value `secret`.
//! It prints `ready`, then keeps the selection until it is killed.
//! When the process stops, Wayland removes the selection. This simulates the clear
//! of a password manager.

#[cfg(target_os = "linux")]
fn main() {
    use std::io::Write;
    use telescreen_core::platform::Clipboard;
    use telescreen_platform_linux::clipboard::LinuxClipboard;

    let mut concealed = false;
    let mut text = None;
    for arg in std::env::args().skip(1) {
        if arg == "--concealed" {
            concealed = true;
        } else {
            text = Some(arg);
        }
    }
    let Some(text) = text else {
        eprintln!("usage: clip_copy [--concealed] <text>");
        std::process::exit(2);
    };
    let mut clipboard = match LinuxClipboard::start() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            eprintln!("clip_copy: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = clipboard.open() {
        eprintln!("clip_copy: {error:?}");
        std::process::exit(1);
    }
    if let Err(error) = clipboard.set_text(&text, concealed) {
        eprintln!("clip_copy: {error:?}");
        std::process::exit(1);
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    println!("ready");
    let _ = std::io::stdout().flush();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("clip_copy: only for Linux");
}
